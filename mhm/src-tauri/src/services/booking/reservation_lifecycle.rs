use chrono::{Local, NaiveDate};
use sqlx::{Pool, Row, Sqlite};

use crate::{
    domain::booking::{pricing::calculate_stay_price_tx, BookingError, BookingResult},
    models::{status, Booking, CreateReservationRequest, ModifyReservationRequest},
};

use super::{
    billing_service::{record_cancellation_fee_tx, record_charge_tx, record_deposit_tx},
    guest_service::{create_reservation_guest_manifest, link_booking_guests},
    support::{
        begin_tx, fetch_booking, insert_room_calendar_rows, read_f64_strict, CalendarInsertMode,
    },
};

fn mark_write_db_error(error: BookingError) -> BookingError {
    match error {
        BookingError::Database(message) => BookingError::database_write(message),
        other => other,
    }
}

pub async fn create_reservation(
    pool: &Pool<Sqlite>,
    req: CreateReservationRequest,
) -> BookingResult<Booking> {
    let derived_nights =
        validate_requested_nights(&req.check_in_date, &req.check_out_date, req.nights)?;

    let mut tx = begin_tx(pool).await.map_err(mark_write_db_error)?;

    let conflicts = sqlx::query(
        "SELECT date FROM room_calendar WHERE room_id = ? AND date >= ? AND date < ? ORDER BY date ASC",
    )
    .bind(&req.room_id)
    .bind(&req.check_in_date)
    .bind(&req.check_out_date)
    .fetch_all(&mut *tx)
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
        &mut tx,
        &req.room_id,
        &req.check_in_date,
        &req.check_out_date,
        "nightly",
    )
    .await?;

    let guest_manifest = create_reservation_guest_manifest(
        &mut tx,
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
    .execute(&mut *tx)
    .await
    .map_err(BookingError::from)
    .map_err(mark_write_db_error)?;

    link_booking_guests(&mut tx, &booking_id, &guest_manifest.guest_ids)
        .await
        .map_err(mark_write_db_error)?;

    insert_booked_calendar_rows(
        &mut tx,
        &req.room_id,
        &booking_id,
        &req.check_in_date,
        &req.check_out_date,
    )
    .await
    .map_err(mark_write_db_error)?;

    if deposit_amount > 0.0 {
        record_deposit_tx(&mut tx, &booking_id, deposit_amount, "Reservation deposit")
            .await
            .map_err(mark_write_db_error)?;
    }

    tx.commit()
        .await
        .map_err(BookingError::from)
        .map_err(mark_write_db_error)?;

    fetch_booking(
        pool,
        &booking_id,
        format!("Booking not found: {}", booking_id),
        read_f64_strict,
    )
    .await
}

pub async fn cancel_reservation(pool: &Pool<Sqlite>, booking_id: &str) -> BookingResult<()> {
    let mut tx = begin_tx(pool).await?;

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
    if status != status::booking::BOOKED {
        return Err(BookingError::conflict(format!(
            "Can only cancel reservations in 'booked' status (current: {})",
            status
        )));
    }

    let room_id: String = booking.get("room_id");
    let deposit_amount: f64 = booking.get("deposit_amount");

    sqlx::query("UPDATE bookings SET status = ? WHERE id = ?")
        .bind(status::booking::CANCELLED)
        .bind(booking_id)
        .execute(&mut *tx)
        .await?;

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
    let mut tx = begin_tx(pool).await?;
    let reservation = load_booked_reservation(&mut tx, booking_id).await?;
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

    sqlx::query(
        "UPDATE bookings
         SET status = ?, check_in_at = ?, expected_checkout = ?, nights = ?, total_price = ?, paid_amount = ?
         WHERE id = ?",
    )
    .bind(status::booking::ACTIVE)
    .bind(&check_in_at)
    .bind(&effective_checkout)
    .bind(actual_nights)
    .bind(pricing.total)
    .bind(reservation.paid_amount)
    .bind(booking_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query("UPDATE rooms SET status = ? WHERE id = ?")
        .bind(status::room::OCCUPIED)
        .bind(&reservation.room_id)
        .execute(&mut *tx)
        .await?;

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

    let mut tx = begin_tx(pool).await?;
    let reservation = load_booked_reservation(&mut tx, &req.booking_id).await?;

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

    sqlx::query(
        "UPDATE bookings
         SET check_in_at = ?, expected_checkout = ?, scheduled_checkin = ?, scheduled_checkout = ?, nights = ?, total_price = ?
         WHERE id = ?",
    )
    .bind(&req.new_check_in_date)
    .bind(&req.new_check_out_date)
    .bind(&req.new_check_in_date)
    .bind(&req.new_check_out_date)
    .bind(derived_nights)
    .bind(pricing.total)
    .bind(&req.booking_id)
    .execute(&mut *tx)
    .await?;

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
    insert_room_calendar_rows(
        tx,
        room_id,
        booking_id,
        from,
        to,
        calendar_status,
        CalendarInsertMode::Insert,
    )
    .await
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
        return Err(BookingError::conflict(format!(
            "Can only operate on reservations in 'booked' status (current: {})",
            booking_status
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
