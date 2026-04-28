use chrono::{Local, NaiveDate};
use serde_json::json;
use sqlx::{Pool, Row, Sqlite, Transaction};

use crate::{
    app_error::{codes, CommandError, CommandResult},
    command_idempotency::{
        system_error, CommandLedgerResultSummary, CommandLedgerSummary, IdempotentCommandResult,
        SanitizedLedgerIntent, WriteCommandContext, WriteCommandExecutor, WriteCommandRequest,
    },
    db_error_monitoring::{classify_db_error_code, is_room_unavailable_conflict_message},
    domain::booking::{
        pricing::calculate_stay_price_tx, BookingError, BookingResult, OriginSideEffect,
    },
    models::{status, Booking, CreateReservationRequest, ModifyReservationRequest},
};

use super::{
    billing_service::{
        record_cancellation_fee_tx, record_charge_tx, record_deposit_tx,
        record_deposit_with_origin_tx,
    },
    guest_service::{create_reservation_guest_manifest, link_booking_guests},
    support::{
        begin_immediate_tx, ensure_one_row_affected, fetch_booking, insert_room_calendar_rows,
        invalid_state_transition, lookup_booking_room_id, read_f64_strict,
    },
};

fn mark_write_db_error(error: BookingError) -> BookingError {
    match error {
        BookingError::Database(message) => BookingError::database_write(message),
        other => other,
    }
}

fn map_known_reservation_command_error(message: &str) -> Option<CommandError> {
    if is_room_unavailable_conflict_message(message) {
        return Some(CommandError::user(
            codes::CONFLICT_ROOM_UNAVAILABLE,
            message.to_string(),
        ));
    }

    match classify_db_error_code(message) {
        Some(codes::DB_LOCKED_RETRYABLE) => Some(
            CommandError::system(codes::DB_LOCKED_RETRYABLE, message.to_string()).retryable(true),
        ),
        _ => None,
    }
}

fn map_create_reservation_command_error(error: BookingError) -> CommandError {
    match error {
        BookingError::Validation(message)
            if message == "Number of nights must be greater than 0" =>
        {
            CommandError::user(codes::BOOKING_INVALID_NIGHTS, "Số đêm phải lớn hơn 0")
        }
        BookingError::NotFound(message) if message.starts_with("Không tìm thấy phòng ") => {
            CommandError::user(codes::ROOM_NOT_FOUND, message)
        }
        BookingError::NotFound(message) if message.starts_with("Booking not found: ") => {
            CommandError::user(codes::BOOKING_NOT_FOUND, message)
        }
        BookingError::Validation(message) | BookingError::Conflict(message) => {
            if let Some(mapped) = map_known_reservation_command_error(&message) {
                return mapped;
            }
            CommandError::user(codes::BOOKING_INVALID_STATE, message)
        }
        BookingError::DatabaseWrite(message) | BookingError::Database(message) => {
            if let Some(mapped) = map_known_reservation_command_error(&message) {
                return mapped;
            }
            CommandError::system(codes::SYSTEM_INTERNAL_ERROR, message)
        }
        BookingError::DateTimeParse(message) | BookingError::NotFound(message) => {
            CommandError::system(codes::SYSTEM_INTERNAL_ERROR, message)
        }
    }
}

pub async fn create_reservation_tx(
    tx: &mut Transaction<'_, Sqlite>,
    req: CreateReservationRequest,
    origin: Option<&OriginSideEffect>,
) -> BookingResult<String> {
    let derived_nights =
        validate_requested_nights(&req.check_in_date, &req.check_out_date, req.nights)?;

    let conflicts = sqlx::query(
        "SELECT date FROM room_calendar WHERE room_id = ? AND date >= ? AND date < ? ORDER BY date ASC",
    )
    .bind(&req.room_id)
    .bind(&req.check_in_date)
    .bind(&req.check_out_date)
    .fetch_all(&mut **tx)
    .await?;

    if let Some(first_conflict) = conflicts.first() {
        let first_date: String = first_conflict.get("date");
        return Err(BookingError::conflict(format!(
            "Room {} is booked on {}. Cannot create reservation.",
            req.room_id, first_date
        )));
    }

    let now = Local::now().to_rfc3339();
    let deposit_amount = req.deposit_amount.unwrap_or(0.0);
    let pricing = calculate_stay_price_tx(
        tx,
        &req.room_id,
        &req.check_in_date,
        &req.check_out_date,
        "nightly",
    )
    .await?;

    let guest_manifest = create_reservation_guest_manifest(
        tx,
        &req.guest_name,
        req.guest_doc_number.as_deref(),
        req.guest_phone.as_deref(),
        &now,
    )
    .await
    .map_err(mark_write_db_error)?;

    let booking_id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO bookings (
            id, room_id, primary_guest_id, check_in_at, expected_checkout, actual_checkout,
            nights, total_price, paid_amount, status, source, notes, created_by,
            booking_type, pricing_type, deposit_amount, guest_phone, scheduled_checkin,
            scheduled_checkout, pricing_snapshot, created_at
         ) VALUES (?, ?, ?, ?, ?, NULL, ?, ?, 0, ?, ?, ?, NULL, 'reservation', 'nightly', ?, ?, ?, ?, NULL, ?)",
    )
    .bind(&booking_id)
    .bind(&req.room_id)
    .bind(&guest_manifest.primary_guest_id)
    .bind(&req.check_in_date)
    .bind(&req.check_out_date)
    .bind(derived_nights)
    .bind(pricing.total)
    .bind(status::booking::BOOKED)
    .bind(req.source.as_deref().unwrap_or("phone"))
    .bind(req.notes.as_deref())
    .bind(deposit_amount)
    .bind(req.guest_phone.as_deref())
    .bind(&req.check_in_date)
    .bind(&req.check_out_date)
    .bind(&now)
    .execute(&mut **tx)
    .await
    .map_err(BookingError::from)
    .map_err(mark_write_db_error)?;

    link_booking_guests(tx, &booking_id, &guest_manifest.guest_ids)
        .await
        .map_err(mark_write_db_error)?;

    insert_booked_calendar_rows(
        tx,
        &req.room_id,
        &booking_id,
        &req.check_in_date,
        &req.check_out_date,
    )
    .await
    .map_err(mark_write_db_error)?;

    if deposit_amount > 0.0 {
        match origin {
            Some(origin) => {
                record_deposit_with_origin_tx(
                    tx,
                    &booking_id,
                    deposit_amount,
                    "Reservation deposit",
                    origin,
                )
                .await
                .map_err(mark_write_db_error)?;
            }
            None => {
                record_deposit_tx(tx, &booking_id, deposit_amount, "Reservation deposit")
                    .await
                    .map_err(mark_write_db_error)?;
            }
        }
    }

    Ok(booking_id)
}

pub async fn fetch_booking_by_id(pool: &Pool<Sqlite>, booking_id: &str) -> BookingResult<Booking> {
    fetch_booking(
        pool,
        booking_id,
        format!("Booking not found: {}", booking_id),
        read_f64_strict,
    )
    .await
}

pub async fn fetch_booking_by_id_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
) -> BookingResult<Booking> {
    let row = sqlx::query(
        "SELECT id, room_id, primary_guest_id, check_in_at, expected_checkout,
                actual_checkout, nights, total_price, paid_amount, status,
                source, notes, created_at
         FROM bookings WHERE id = ?",
    )
    .bind(booking_id)
    .fetch_optional(&mut **tx)
    .await?;

    let row =
        row.ok_or_else(|| BookingError::not_found(format!("Booking not found: {}", booking_id)))?;

    Ok(Booking {
        id: row.get("id"),
        room_id: row.get("room_id"),
        primary_guest_id: row.get("primary_guest_id"),
        check_in_at: row.get("check_in_at"),
        expected_checkout: row.get("expected_checkout"),
        actual_checkout: row.get("actual_checkout"),
        nights: row.get("nights"),
        total_price: read_f64_strict(&row, "total_price"),
        paid_amount: read_f64_strict(&row, "paid_amount"),
        status: row.get("status"),
        source: row.get("source"),
        notes: row.get("notes"),
        created_at: row.get("created_at"),
    })
}

pub async fn create_reservation(
    pool: &Pool<Sqlite>,
    req: CreateReservationRequest,
) -> BookingResult<Booking> {
    let mut tx = begin_immediate_tx(pool)
        .await
        .map_err(mark_write_db_error)?;

    let booking_id = create_reservation_tx(&mut tx, req, None)
        .await
        .map_err(mark_write_db_error)?;

    tx.commit()
        .await
        .map_err(BookingError::from)
        .map_err(mark_write_db_error)?;

    fetch_booking_by_id(pool, &booking_id).await
}

pub async fn create_reservation_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    req: CreateReservationRequest,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let room_id = req.room_id.clone();
    let check_in_date = req.check_in_date.clone();
    let check_out_date = req.check_out_date.clone();
    let nights = req.nights;
    let source = req.source.clone();
    let deposit_minor_units = req
        .deposit_amount
        .map(|amount| format!("{:.0}", amount * 100.0))
        .unwrap_or_else(|| "0".to_string());

    let hash_payload = json!({
        "schema": "reservation.create.v1",
        "room_id": room_id.clone(),
        "guest_name": req.guest_name.clone(),
        "guest_doc_number": req.guest_doc_number.clone(),
        "guest_phone": req.guest_phone.clone(),
        "check_in_date": check_in_date.clone(),
        "check_out_date": check_out_date.clone(),
        "nights": nights,
        "source": source.clone(),
        "notes": req.notes.clone(),
        "deposit_minor_units": deposit_minor_units,
    });

    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("reservation.create.v1")),
        ("room_id", json!(room_id.clone())),
        ("check_in_date", json!(check_in_date.clone())),
        ("check_out_date", json!(check_out_date.clone())),
        ("nights", json!(nights)),
        (
            "deposit_present",
            json!(req.deposit_amount.unwrap_or(0.0) > 0.0),
        ),
    ])?;
    let summary = CommandLedgerSummary::new("Create reservation")?
        .with_aggregate_ref("room", room_id, None::<String>)?
        .with_business_date(check_in_date)?;
    let request = WriteCommandRequest::new_sanitized(hash_payload, ledger_intent, summary)?
        .with_success_summary(CommandLedgerResultSummary::success("Reservation created")?);

    let request_for_service = req;
    let origin_idempotency_key = ctx.idempotency_key.clone();

    WriteCommandExecutor::new(pool.clone())
        .execute_atomic(ctx, request, move |tx| {
            Box::pin(async move {
                let origin =
                    OriginSideEffect::new(origin_idempotency_key, 0).map_err(system_error)?;
                let booking_id = create_reservation_tx(tx, request_for_service, Some(&origin))
                    .await
                    .map_err(map_create_reservation_command_error)?;
                let booking = fetch_booking_by_id_tx(tx, &booking_id)
                    .await
                    .map_err(map_create_reservation_command_error)?;
                serde_json::to_value(&booking).map_err(system_error)
            })
        })
        .await
}

pub async fn cancel_reservation(pool: &Pool<Sqlite>, booking_id: &str) -> BookingResult<()> {
    let locked_room_id = lookup_booking_room_id(pool, booking_id).await?;
    let _lock_guard = crate::aggregate_locks::global_manager()
        .acquire([
            crate::aggregate_locks::booking_key(booking_id)
                .map_err(|error| BookingError::validation(error.message))?,
            crate::aggregate_locks::room_key(&locked_room_id)
                .map_err(|error| BookingError::validation(error.message))?,
        ])
        .await
        .map_err(|error| BookingError::validation(error.message))?;

    let mut tx = begin_immediate_tx(pool).await?;

    let booking = sqlx::query(
        "SELECT room_id, status, COALESCE(deposit_amount, 0) AS deposit_amount
         FROM bookings
         WHERE id = ?",
    )
    .bind(booking_id)
    .fetch_optional(&mut *tx)
    .await?;

    let booking = booking
        .ok_or_else(|| BookingError::not_found(format!("Booking not found: {}", booking_id)))?;

    let status: String = booking.get("status");
    let room_id: String = booking.get("room_id");
    if room_id != locked_room_id {
        return Err(invalid_state_transition(format!(
            "reservation {booking_id} changed rooms before cancellation"
        )));
    }

    if status != status::booking::BOOKED {
        return Err(invalid_state_transition(format!(
            "reservation {booking_id} is no longer booked"
        )));
    }

    let deposit_amount: f64 = booking.get("deposit_amount");

    let result = sqlx::query("UPDATE bookings SET status = ? WHERE id = ? AND status = ?")
        .bind(status::booking::CANCELLED)
        .bind(booking_id)
        .bind(status::booking::BOOKED)
        .execute(&mut *tx)
        .await?;
    ensure_one_row_affected(
        result,
        format!("reservation {booking_id} is no longer booked"),
    )?;

    sqlx::query("DELETE FROM room_calendar WHERE booking_id = ? AND status = ?")
        .bind(booking_id)
        .bind(status::calendar::BOOKED)
        .execute(&mut *tx)
        .await?;

    if deposit_amount > 0.0 {
        record_cancellation_fee_tx(
            &mut tx,
            booking_id,
            deposit_amount,
            "Deposit retained (cancellation)",
        )
        .await?;
    }

    let remaining_booked: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM room_calendar WHERE room_id = ? AND status = ?")
            .bind(&room_id)
            .bind(status::calendar::BOOKED)
            .fetch_one(&mut *tx)
            .await?;

    let room_status = sqlx::query_scalar::<_, String>("SELECT status FROM rooms WHERE id = ?")
        .bind(&room_id)
        .fetch_one(&mut *tx)
        .await?;

    if room_status == status::room::BOOKED && remaining_booked.0 == 0 {
        sqlx::query("UPDATE rooms SET status = ? WHERE id = ?")
            .bind(status::room::VACANT)
            .bind(&room_id)
            .execute(&mut *tx)
            .await?;
    }

    tx.commit().await.map_err(BookingError::from)?;

    Ok(())
}

pub async fn confirm_reservation(pool: &Pool<Sqlite>, booking_id: &str) -> BookingResult<Booking> {
    let locked_room_id = lookup_booking_room_id(pool, booking_id).await?;
    let _lock_guard = crate::aggregate_locks::global_manager()
        .acquire([
            crate::aggregate_locks::booking_key(booking_id)
                .map_err(|error| BookingError::validation(error.message))?,
            crate::aggregate_locks::room_key(&locked_room_id)
                .map_err(|error| BookingError::validation(error.message))?,
        ])
        .await
        .map_err(|error| BookingError::validation(error.message))?;

    let mut tx = begin_immediate_tx(pool).await?;
    let reservation = load_booked_reservation(&mut tx, booking_id).await?;
    if reservation.room_id != locked_room_id {
        return Err(invalid_state_transition(format!(
            "reservation {booking_id} changed rooms before confirmation"
        )));
    }
    reject_no_show_confirmation(&mut tx, booking_id).await?;

    let now = Local::now();
    let today = now.date_naive();
    let scheduled_checkout = parse_date(&reservation.scheduled_checkout)?;
    let effective_checkout_date = if scheduled_checkout <= today {
        today + chrono::Duration::days(1)
    } else {
        scheduled_checkout
    };
    let effective_checkout = effective_checkout_date.format("%Y-%m-%d").to_string();
    let pricing = calculate_stay_price_tx(
        &mut tx,
        &reservation.room_id,
        &today.format("%Y-%m-%d").to_string(),
        &effective_checkout,
        &reservation.pricing_type,
    )
    .await?;
    let actual_nights = (effective_checkout_date - today).num_days() as i32;
    let check_in_at = now.to_rfc3339();

    sqlx::query("DELETE FROM room_calendar WHERE booking_id = ?")
        .bind(booking_id)
        .execute(&mut *tx)
        .await?;

    insert_calendar_rows(
        &mut tx,
        &reservation.room_id,
        booking_id,
        today,
        effective_checkout_date,
        status::calendar::OCCUPIED,
    )
    .await?;

    let result = sqlx::query(
        "UPDATE bookings
         SET status = ?, check_in_at = ?, expected_checkout = ?, nights = ?, total_price = ?, paid_amount = ?
         WHERE id = ? AND status = ?",
    )
    .bind(status::booking::ACTIVE)
    .bind(&check_in_at)
    .bind(&effective_checkout)
    .bind(actual_nights)
    .bind(pricing.total)
    .bind(reservation.paid_amount)
    .bind(booking_id)
    .bind(status::booking::BOOKED)
    .execute(&mut *tx)
    .await?;
    ensure_one_row_affected(
        result,
        format!("reservation {booking_id} is no longer booked"),
    )?;

    let result = sqlx::query("UPDATE rooms SET status = ? WHERE id = ? AND status IN (?, ?)")
        .bind(status::room::OCCUPIED)
        .bind(&reservation.room_id)
        .bind(status::room::VACANT)
        .bind(status::room::BOOKED)
        .execute(&mut *tx)
        .await?;
    ensure_one_row_affected(
        result,
        format!(
            "room {} is no longer available for confirmation",
            reservation.room_id
        ),
    )?;

    record_charge_tx(
        &mut tx,
        booking_id,
        pricing.total,
        "Room charge (reservation)",
        check_in_at,
    )
    .await?;

    tx.commit().await.map_err(BookingError::from)?;

    fetch_booking(
        pool,
        booking_id,
        format!("Booking not found: {}", booking_id),
        read_f64_strict,
    )
    .await
}

pub async fn modify_reservation(
    pool: &Pool<Sqlite>,
    req: ModifyReservationRequest,
) -> BookingResult<Booking> {
    let derived_nights = validate_requested_nights(
        &req.new_check_in_date,
        &req.new_check_out_date,
        req.new_nights,
    )? as i64;

    let locked_room_id = lookup_booking_room_id(pool, &req.booking_id).await?;
    let _lock_guard = crate::aggregate_locks::global_manager()
        .acquire([
            crate::aggregate_locks::booking_key(&req.booking_id)
                .map_err(|error| BookingError::validation(error.message))?,
            crate::aggregate_locks::room_key(&locked_room_id)
                .map_err(|error| BookingError::validation(error.message))?,
        ])
        .await
        .map_err(|error| BookingError::validation(error.message))?;

    let mut tx = begin_immediate_tx(pool).await?;
    let reservation = load_booked_reservation(&mut tx, &req.booking_id).await?;
    if reservation.room_id != locked_room_id {
        return Err(invalid_state_transition(format!(
            "reservation {} changed rooms before modification",
            req.booking_id
        )));
    }

    sqlx::query("DELETE FROM room_calendar WHERE booking_id = ? AND status = ?")
        .bind(&req.booking_id)
        .bind(status::calendar::BOOKED)
        .execute(&mut *tx)
        .await?;

    let conflicts = sqlx::query(
        "SELECT date FROM room_calendar WHERE room_id = ? AND date >= ? AND date < ? ORDER BY date ASC",
    )
    .bind(&reservation.room_id)
    .bind(&req.new_check_in_date)
    .bind(&req.new_check_out_date)
    .fetch_all(&mut *tx)
    .await?;

    if let Some(first_conflict) = conflicts.first() {
        let first_date: String = first_conflict.get("date");
        return Err(BookingError::conflict(format!(
            "Room {} is booked on {}. Cannot modify.",
            reservation.room_id, first_date
        )));
    }

    let pricing = calculate_stay_price_tx(
        &mut tx,
        &reservation.room_id,
        &req.new_check_in_date,
        &req.new_check_out_date,
        &reservation.pricing_type,
    )
    .await?;

    let result = sqlx::query(
        "UPDATE bookings
         SET check_in_at = ?, expected_checkout = ?, scheduled_checkin = ?, scheduled_checkout = ?, nights = ?, total_price = ?
         WHERE id = ? AND status = ? AND room_id = ?",
    )
    .bind(&req.new_check_in_date)
    .bind(&req.new_check_out_date)
    .bind(&req.new_check_in_date)
    .bind(&req.new_check_out_date)
    .bind(derived_nights)
    .bind(pricing.total)
    .bind(&req.booking_id)
    .bind(status::booking::BOOKED)
    .bind(&locked_room_id)
    .execute(&mut *tx)
    .await?;
    ensure_one_row_affected(
        result,
        format!("reservation {} is no longer booked", req.booking_id),
    )?;

    insert_booked_calendar_rows(
        &mut tx,
        &reservation.room_id,
        &req.booking_id,
        &req.new_check_in_date,
        &req.new_check_out_date,
    )
    .await?;

    tx.commit().await.map_err(BookingError::from)?;

    fetch_booking(
        pool,
        &req.booking_id,
        format!("Booking not found: {}", req.booking_id),
        read_f64_strict,
    )
    .await
}

fn validate_requested_nights(
    check_in_date: &str,
    check_out_date: &str,
    requested_nights: i32,
) -> BookingResult<i32> {
    let check_in = parse_date(check_in_date)?;
    let check_out = parse_date(check_out_date)?;
    let derived_nights = (check_out - check_in).num_days();
    if derived_nights <= 0 {
        return Err(BookingError::validation(
            "Check-out date must be after check-in date".to_string(),
        ));
    }
    if requested_nights != derived_nights as i32 {
        return Err(BookingError::validation(format!(
            "Number of nights must match the date range (expected {})",
            derived_nights
        )));
    }

    Ok(derived_nights as i32)
}

async fn insert_booked_calendar_rows(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    room_id: &str,
    booking_id: &str,
    check_in_date: &str,
    check_out_date: &str,
) -> BookingResult<()> {
    insert_calendar_rows(
        tx,
        room_id,
        booking_id,
        parse_date(check_in_date)?,
        parse_date(check_out_date)?,
        status::calendar::BOOKED,
    )
    .await
}

async fn insert_calendar_rows(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    room_id: &str,
    booking_id: &str,
    from: NaiveDate,
    to: NaiveDate,
    calendar_status: &str,
) -> BookingResult<()> {
    insert_room_calendar_rows(tx, room_id, booking_id, from, to, calendar_status).await
}

async fn load_booked_reservation(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    booking_id: &str,
) -> BookingResult<BookedReservation> {
    let row = sqlx::query(
        "SELECT room_id, status, paid_amount, scheduled_checkout, pricing_type
         FROM bookings
         WHERE id = ?",
    )
    .bind(booking_id)
    .fetch_optional(&mut **tx)
    .await?;

    let row =
        row.ok_or_else(|| BookingError::not_found(format!("Booking not found: {}", booking_id)))?;
    let booking_status: String = row.get("status");
    if booking_status != status::booking::BOOKED {
        return Err(invalid_state_transition(format!(
            "reservation {booking_id} is no longer booked"
        )));
    }

    let scheduled_checkout = row
        .get::<Option<String>, _>("scheduled_checkout")
        .ok_or_else(|| {
            BookingError::not_found(format!("Missing scheduled checkout for {}", booking_id))
        })?;

    Ok(BookedReservation {
        room_id: row.get("room_id"),
        paid_amount: row.get::<Option<f64>, _>("paid_amount").unwrap_or(0.0),
        scheduled_checkout,
        pricing_type: row
            .get::<Option<String>, _>("pricing_type")
            .unwrap_or_else(|| "nightly".to_string()),
    })
}

async fn reject_no_show_confirmation(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    booking_id: &str,
) -> BookingResult<()> {
    let no_show = sqlx::query_scalar::<_, String>(
        "SELECT booking_id FROM room_calendar WHERE booking_id = ? AND status = ? LIMIT 1",
    )
    .bind(booking_id)
    .bind(status::booking::NO_SHOW)
    .fetch_optional(&mut **tx)
    .await?;

    if no_show.is_some() {
        return Err(BookingError::conflict(format!(
            "Cannot confirm no-show reservation {}",
            booking_id
        )));
    }

    Ok(())
}

struct BookedReservation {
    room_id: String,
    paid_amount: f64,
    scheduled_checkout: String,
    pricing_type: String,
}

fn parse_date(value: &str) -> BookingResult<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|error| BookingError::datetime_parse(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::mark_write_db_error;
    use crate::domain::booking::BookingError;

    #[test]
    fn mark_write_db_error_promotes_database_errors_but_preserves_missing_record() {
        assert_eq!(
            mark_write_db_error(BookingError::database("disk full")),
            BookingError::database_write("disk full")
        );
        assert_eq!(
            mark_write_db_error(BookingError::not_found("Booking not found: booking-1")),
            BookingError::not_found("Booking not found: booking-1")
        );
        assert_eq!(
            mark_write_db_error(BookingError::database_write("disk full")),
            BookingError::database_write("disk full")
        );
    }
}
