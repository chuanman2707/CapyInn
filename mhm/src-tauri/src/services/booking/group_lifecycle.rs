use chrono::{Duration, Local, NaiveDate};
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
    models::{status, BookingGroup, GroupCheckinRequest, GroupCheckoutRequest},
};

use super::{
    billing_service::{record_charge_tx, record_payment_tx, record_payment_with_origin_tx},
    guest_service::{create_group_guest_manifest, link_booking_guests},
    support::{
        begin_immediate_tx, ensure_one_row_affected, ensure_rows_affected,
        insert_room_calendar_rows, invalid_state_transition,
    },
};

const GROUP_ACTIVE: &str = "active";
const GROUP_BOOKED: &str = "booked";
const GROUP_COMPLETED: &str = "completed";
const GROUP_PARTIAL_CHECKOUT: &str = "partial_checkout";

fn map_group_checkin_command_error(error: BookingError) -> CommandError {
    match error {
        BookingError::Validation(message) if message == "Phải chọn ít nhất 1 phòng" => {
            CommandError::user(codes::GROUP_INVALID_ROOM_COUNT, message)
        }
        BookingError::Validation(message) if message == "Số phòng phải > 0" => {
            CommandError::user(codes::GROUP_INVALID_ROOM_COUNT, message)
        }
        BookingError::Validation(message) if message == "Số đêm phải > 0" => {
            CommandError::user(codes::BOOKING_INVALID_NIGHTS, message)
        }
        BookingError::NotFound(message) if message.starts_with("Phòng ") => {
            CommandError::user(codes::ROOM_NOT_FOUND, message)
        }
        BookingError::Validation(message) | BookingError::Conflict(message) => {
            if message.contains(codes::CONFLICT_INVALID_STATE_TRANSITION) {
                return CommandError::user(codes::CONFLICT_INVALID_STATE_TRANSITION, message);
            }
            if is_room_unavailable_conflict_message(&message) {
                return CommandError::user(codes::CONFLICT_ROOM_UNAVAILABLE, message);
            }
            CommandError::user(codes::BOOKING_INVALID_STATE, message)
        }
        BookingError::DatabaseWrite(message) | BookingError::Database(message) => {
            if classify_db_error_code(&message) == Some(codes::DB_LOCKED_RETRYABLE) {
                return CommandError::system(codes::DB_LOCKED_RETRYABLE, message).retryable(true);
            }
            CommandError::system(codes::SYSTEM_INTERNAL_ERROR, message)
        }
        BookingError::DateTimeParse(message) | BookingError::NotFound(message) => {
            CommandError::system(codes::SYSTEM_INTERNAL_ERROR, message)
        }
    }
}

fn normalized_room_ids(room_ids: &[String]) -> Vec<String> {
    let mut normalized = room_ids.to_vec();
    normalized.sort();
    normalized
}

fn build_group_checkin_hash_payload(req: &GroupCheckinRequest) -> serde_json::Value {
    let paid_minor_units = req
        .paid_amount
        .map(|amount| json!(format!("{:.0}", amount * 100.0)))
        .unwrap_or(serde_json::Value::Null);
    let guests_per_room = req
        .guests_per_room
        .iter()
        .map(|(room_id, guests)| {
            let guests = guests
                .iter()
                .map(|guest| {
                    json!({
                        "guest_type": guest.guest_type.clone(),
                        "full_name": guest.full_name.clone(),
                        "doc_number": guest.doc_number.clone(),
                        "dob": guest.dob.clone(),
                        "gender": guest.gender.clone(),
                        "nationality": guest.nationality.clone(),
                        "address": guest.address.clone(),
                        "visa_expiry": guest.visa_expiry.clone(),
                        "scan_path": guest.scan_path.clone(),
                        "phone": guest.phone.clone(),
                    })
                })
                .collect::<Vec<_>>();
            (room_id.clone(), json!(guests))
        })
        .collect::<serde_json::Map<_, _>>();

    json!({
        "schema": "group.checkin.v1",
        "group_name": req.group_name.clone(),
        "organizer_name": req.organizer_name.clone(),
        "organizer_phone": req.organizer_phone.clone(),
        "check_in_date": req.check_in_date.clone(),
        "room_ids": normalized_room_ids(&req.room_ids),
        "master_room_id": req.master_room_id.clone(),
        "guests_per_room": guests_per_room,
        "nights": req.nights,
        "source": req.source.clone(),
        "notes": req.notes.clone(),
        "paid_minor_units": paid_minor_units,
    })
}

fn group_checkin_lock_keys_from_payload(
    hash_payload: &serde_json::Value,
) -> CommandResult<Vec<String>> {
    let room_ids = hash_payload
        .get("room_ids")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| system_error("group check-in lock payload missing room_ids"))?;

    room_ids
        .iter()
        .map(|value| {
            let room_id = value
                .as_str()
                .ok_or_else(|| system_error("group check-in lock room id must be a string"))?;
            crate::aggregate_locks::room_key(room_id)
        })
        .collect()
}

fn build_group_checkin_payment_origins(
    idempotency_key: &str,
    room_ids: &[String],
) -> CommandResult<std::collections::HashMap<String, OriginSideEffect>> {
    let mut origins = std::collections::HashMap::new();
    for (ordinal, room_id) in normalized_room_ids(room_ids).into_iter().enumerate() {
        origins.insert(
            room_id,
            OriginSideEffect::new(idempotency_key, ordinal as i64).map_err(system_error)?,
        );
    }
    Ok(origins)
}

#[allow(dead_code)]
pub async fn group_checkin(
    pool: &Pool<Sqlite>,
    user_id: Option<String>,
    req: GroupCheckinRequest,
) -> BookingResult<BookingGroup> {
    validate_group_checkin_request(&req)?;

    let mut tx = begin_immediate_tx(pool).await?;
    let group_id = group_checkin_tx(&mut tx, user_id.as_deref(), &req, None).await?;
    tx.commit().await.map_err(BookingError::from)?;
    fetch_group(pool, &group_id).await
}

pub async fn group_checkin_idempotent(
    pool: &Pool<Sqlite>,
    user_id: Option<String>,
    ctx: &WriteCommandContext,
    req: GroupCheckinRequest,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    validate_group_checkin_request(&req).map_err(map_group_checkin_command_error)?;

    let effective_checkin_date = req
        .check_in_date
        .clone()
        .unwrap_or_else(|| Local::now().format("%Y-%m-%d").to_string());
    let paid_amount = req.paid_amount.unwrap_or(0.0);

    let hash_payload = build_group_checkin_hash_payload(&req);
    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("group.checkin.v1")),
        ("room_count", json!(req.room_ids.len())),
        ("guest_room_count", json!(req.guests_per_room.len())),
        (
            "guest_form_count",
            json!(req
                .guests_per_room
                .values()
                .map(|guests| guests.len())
                .sum::<usize>()),
        ),
        ("nights", json!(req.nights)),
        ("check_in_date", json!(effective_checkin_date.clone())),
        (
            "has_organizer_contact",
            json!(req.organizer_phone.is_some()),
        ),
        ("has_source", json!(req.source.is_some())),
        ("has_notes", json!(req.notes.is_some())),
        ("has_paid_amount", json!(req.paid_amount.is_some())),
        ("paid_amount_positive", json!(paid_amount > 0.0)),
    ])?;
    let summary =
        CommandLedgerSummary::new("Group check-in")?.with_business_date(effective_checkin_date)?;
    let runtime_lock_keys = group_checkin_lock_keys_from_payload(&hash_payload)?;
    let request = WriteCommandRequest::new_sanitized(hash_payload, ledger_intent, summary)?
        .with_lock_key_deriver(group_checkin_lock_keys_from_payload)
        .with_success_summary(CommandLedgerResultSummary::success("Group checked in")?);

    let req_for_service = req;
    let user_id_for_service = user_id;
    let origin_idempotency_key = ctx.idempotency_key.clone();

    WriteCommandExecutor::new(pool.clone())
        .execute_with_pre_transaction_guard(
            ctx,
            request,
            move || async move {
                crate::aggregate_locks::global_manager()
                    .acquire(runtime_lock_keys)
                    .await
            },
            move |tx| {
                Box::pin(async move {
                    let payment_origins = if req_for_service.paid_amount.unwrap_or(0.0) > 0.0 {
                        Some(build_group_checkin_payment_origins(
                            &origin_idempotency_key,
                            &req_for_service.room_ids,
                        )?)
                    } else {
                        None
                    };
                    let group_id = group_checkin_tx(
                        tx,
                        user_id_for_service.as_deref(),
                        &req_for_service,
                        payment_origins.as_ref(),
                    )
                    .await
                    .map_err(map_group_checkin_command_error)?;
                    let group = fetch_group_tx(tx, &group_id)
                        .await
                        .map_err(map_group_checkin_command_error)?;
                    serde_json::to_value(&group).map_err(system_error)
                })
            },
        )
        .await
}

async fn group_checkin_tx(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: Option<&str>,
    req: &GroupCheckinRequest,
    payment_origins_by_room: Option<&std::collections::HashMap<String, OriginSideEffect>>,
) -> BookingResult<String> {
    let now = Local::now();
    let now_rfc3339 = now.to_rfc3339();
    let today_str = now.format("%Y-%m-%d").to_string();
    let is_reservation = req
        .check_in_date
        .as_ref()
        .map(|date| date != &today_str)
        .unwrap_or(false);
    let checkin_date = req.check_in_date.clone().unwrap_or(today_str);
    let checkin_naive = parse_date(&checkin_date)?;
    let checkout_naive = checkin_naive + Duration::days(req.nights as i64);
    let checkout_date = checkout_naive.format("%Y-%m-%d").to_string();

    validate_rooms_for_group(
        tx,
        &req.room_ids,
        is_reservation,
        &checkin_date,
        &checkout_date,
    )
    .await?;

    let group_id = uuid::Uuid::new_v4().to_string();
    let group_status = if is_reservation {
        GROUP_BOOKED
    } else {
        GROUP_ACTIVE
    };
    sqlx::query(
        "INSERT INTO booking_groups (
            id, group_name, organizer_name, organizer_phone, total_rooms, status, notes, created_by, created_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&group_id)
    .bind(&req.group_name)
    .bind(&req.organizer_name)
    .bind(req.organizer_phone.as_deref())
    .bind(req.room_ids.len() as i32)
    .bind(group_status)
    .bind(req.notes.as_deref())
    .bind(user_id)
    .bind(&now_rfc3339)
    .execute(&mut **tx)
    .await?;

    let paid_total = req.paid_amount.unwrap_or(0.0);
    let paid_per_room = if paid_total <= 0.0 || req.room_ids.is_empty() {
        0.0
    } else {
        paid_total / req.room_ids.len() as f64
    };
    let mut master_booking_id: Option<String> = None;

    for room_id in &req.room_ids {
        let is_master = room_id == &req.master_room_id;
        let room_guests = req
            .guests_per_room
            .get(room_id.as_str())
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let guest_manifest = create_group_guest_manifest(
            tx,
            room_guests,
            &format!("Khách đoàn {} - {}", req.group_name, room_id),
            &now_rfc3339,
        )
        .await?;

        let booking_id = uuid::Uuid::new_v4().to_string();
        let booking_status = if is_reservation {
            status::booking::BOOKED
        } else {
            status::booking::ACTIVE
        };
        let booking_type = if is_reservation {
            "reservation"
        } else {
            "walk-in"
        };
        let booking_checkin_at = if is_reservation {
            format!("{}T14:00:00+07:00", &checkin_date)
        } else {
            now_rfc3339.clone()
        };
        let booking_checkout_at = if is_reservation {
            format!("{}T12:00:00+07:00", &checkout_date)
        } else {
            (now + Duration::days(req.nights as i64)).to_rfc3339()
        };
        let pricing = calculate_stay_price_tx(
            tx,
            room_id,
            if is_reservation {
                &checkin_date
            } else {
                &booking_checkin_at
            },
            if is_reservation {
                &checkout_date
            } else {
                &booking_checkout_at
            },
            "nightly",
        )
        .await?;
        let deposit_amount = if is_reservation { paid_per_room } else { 0.0 };
        let guest_phone = room_guests.first().and_then(|guest| guest.phone.as_deref());

        sqlx::query(
            "INSERT INTO bookings (
                id, room_id, primary_guest_id, check_in_at, expected_checkout, actual_checkout,
                nights, total_price, paid_amount, status, source, notes, created_by,
                booking_type, pricing_type, deposit_amount, guest_phone, scheduled_checkin,
                scheduled_checkout, group_id, is_master_room, pricing_snapshot, created_at
             ) VALUES (?, ?, ?, ?, ?, NULL, ?, ?, 0, ?, ?, ?, ?, ?, 'nightly', ?, ?, ?, ?, ?, ?, NULL, ?)",
        )
        .bind(&booking_id)
        .bind(room_id)
        .bind(&guest_manifest.primary_guest_id)
        .bind(&booking_checkin_at)
        .bind(&booking_checkout_at)
        .bind(req.nights)
        .bind(pricing.total)
        .bind(booking_status)
        .bind(req.source.as_deref().unwrap_or("walk-in"))
        .bind(req.notes.as_deref())
        .bind(user_id)
        .bind(booking_type)
        .bind(deposit_amount)
        .bind(guest_phone)
        .bind(if is_reservation {
            Some(checkin_date.as_str())
        } else {
            None
        })
        .bind(if is_reservation {
            Some(checkout_date.as_str())
        } else {
            None
        })
        .bind(&group_id)
        .bind(if is_master { 1 } else { 0 })
        .bind(&now_rfc3339)
        .execute(&mut **tx)
        .await?;

        if is_master {
            master_booking_id = Some(booking_id.clone());
        }

        link_booking_guests(tx, &booking_id, &guest_manifest.guest_ids).await?;

        if !is_reservation {
            record_charge_tx(
                tx,
                &booking_id,
                pricing.total,
                "Tiền phòng (đoàn)",
                booking_checkin_at.clone(),
            )
            .await?;

            if paid_per_room > 0.0 {
                if let Some(origins) = payment_origins_by_room {
                    if let Some(origin) = origins.get(room_id) {
                        record_payment_with_origin_tx(
                            tx,
                            &booking_id,
                            paid_per_room,
                            "Thanh toán group check-in",
                            origin,
                        )
                        .await?;
                    } else {
                        record_payment_tx(
                            tx,
                            &booking_id,
                            paid_per_room,
                            "Thanh toán group check-in",
                        )
                        .await?;
                    }
                } else {
                    record_payment_tx(tx, &booking_id, paid_per_room, "Thanh toán group check-in")
                        .await?;
                }
            }
        } else if paid_per_room > 0.0 {
            if let Some(origins) = payment_origins_by_room {
                if let Some(origin) = origins.get(room_id) {
                    record_payment_with_origin_tx(
                        tx,
                        &booking_id,
                        paid_per_room,
                        "Đặt cọc đoàn",
                        origin,
                    )
                    .await?;
                } else {
                    record_payment_tx(tx, &booking_id, paid_per_room, "Đặt cọc đoàn").await?;
                }
            } else {
                record_payment_tx(tx, &booking_id, paid_per_room, "Đặt cọc đoàn").await?;
            }
        }

        insert_group_calendar_rows(
            tx,
            room_id,
            &booking_id,
            checkin_naive,
            checkout_naive,
            if is_reservation {
                status::calendar::BOOKED
            } else {
                status::calendar::OCCUPIED
            },
        )
        .await?;

        if !is_reservation {
            let result = sqlx::query("UPDATE rooms SET status = ? WHERE id = ? AND status = ?")
                .bind(status::room::OCCUPIED)
                .bind(room_id)
                .bind(status::room::VACANT)
                .execute(&mut **tx)
                .await?;
            ensure_one_row_affected(result, format!("room {room_id} is no longer vacant"))?;
        }
    }

    if let Some(ref booking_id) = master_booking_id {
        sqlx::query("UPDATE booking_groups SET master_booking_id = ? WHERE id = ?")
            .bind(booking_id)
            .bind(&group_id)
            .execute(&mut **tx)
            .await?;
    }

    Ok(group_id)
}

pub async fn group_checkout(pool: &Pool<Sqlite>, req: GroupCheckoutRequest) -> BookingResult<()> {
    if req.booking_ids.is_empty() {
        return Err(BookingError::validation(
            "Phải chọn ít nhất 1 phòng để checkout".to_string(),
        ));
    }

    let now = Local::now().to_rfc3339();
    let mut unique_booking_ids = Vec::new();
    let mut seen_booking_ids = std::collections::HashSet::new();
    for id in &req.booking_ids {
        if seen_booking_ids.insert(id.clone()) {
            unique_booking_ids.push(id.clone());
        }
    }

    let mut query_builder: sqlx::QueryBuilder<Sqlite> =
        sqlx::QueryBuilder::new("SELECT id, room_id FROM bookings WHERE group_id = ");
    query_builder.push_bind(&req.group_id);
    query_builder.push(" AND id IN (");
    let mut separated = query_builder.separated(", ");
    for id in &unique_booking_ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");

    let rows = query_builder.build().fetch_all(pool).await?;
    let mut booking_room_map = std::collections::HashMap::new();
    for row in rows {
        let id: String = row.get("id");
        let room_id: String = row.get("room_id");
        booking_room_map.insert(id, room_id);
    }

    for id in &req.booking_ids {
        if !booking_room_map.contains_key(id) {
            return Err(BookingError::not_found(format!(
                "Booking {} không tìm thấy hoặc đã checkout",
                id
            )));
        }
    }

    let mut room_ids = Vec::new();
    let mut seen_room_ids = std::collections::HashSet::new();
    for booking_id in &unique_booking_ids {
        if let Some(room_id) = booking_room_map.get(booking_id) {
            if seen_room_ids.insert(room_id.clone()) {
                room_ids.push(room_id.clone());
            }
        }
    }

    let mut lock_keys = vec![crate::aggregate_locks::group_key(&req.group_id)
        .map_err(|error| BookingError::validation(error.message))?];
    for booking_id in &unique_booking_ids {
        lock_keys.push(
            crate::aggregate_locks::booking_key(booking_id)
                .map_err(|error| BookingError::validation(error.message))?,
        );
    }
    for room_id in &room_ids {
        lock_keys.push(
            crate::aggregate_locks::room_key(room_id)
                .map_err(|error| BookingError::validation(error.message))?,
        );
    }
    let _lock_guard = crate::aggregate_locks::global_manager()
        .acquire(lock_keys)
        .await
        .map_err(|error| BookingError::validation(error.message))?;

    let mut tx = begin_immediate_tx(pool).await?;

    let mut query_builder: sqlx::QueryBuilder<Sqlite> =
        sqlx::QueryBuilder::new("SELECT id, room_id FROM bookings WHERE group_id = ");
    query_builder.push_bind(&req.group_id);
    query_builder.push(" AND id IN (");
    let mut separated = query_builder.separated(", ");
    for id in &unique_booking_ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");

    let rows = query_builder.build().fetch_all(&mut *tx).await?;
    let mut current_booking_room_map = std::collections::HashMap::new();
    for row in rows {
        let id: String = row.get("id");
        let room_id: String = row.get("room_id");
        current_booking_room_map.insert(id, room_id);
    }
    ensure_group_checkout_room_map_still_locked(
        &req.group_id,
        &unique_booking_ids,
        &booking_room_map,
        &current_booking_room_map,
    )?;

    let mut qb = sqlx::QueryBuilder::new("UPDATE bookings SET status = ");
    qb.push_bind(status::booking::CHECKED_OUT);
    qb.push(", actual_checkout = ");
    qb.push_bind(&now);
    qb.push(" WHERE group_id = ");
    qb.push_bind(&req.group_id);
    qb.push(" AND status = ");
    qb.push_bind(status::booking::ACTIVE);
    qb.push(" AND id IN (");
    let mut sep = qb.separated(", ");
    for id in &unique_booking_ids {
        sep.push_bind(id);
    }
    sep.push_unseparated(")");
    let result = qb.build().execute(&mut *tx).await?;
    ensure_rows_affected(
        result,
        unique_booking_ids.len() as u64,
        format!(
            "one or more bookings in group {} are no longer active",
            req.group_id
        ),
    )?;

    let mut qb = sqlx::QueryBuilder::new("UPDATE rooms SET status = ");
    qb.push_bind(status::room::CLEANING);
    qb.push(" WHERE status = ");
    qb.push_bind(status::room::OCCUPIED);
    qb.push(" AND id IN (");
    let mut sep = qb.separated(", ");
    for rid in &room_ids {
        sep.push_bind(rid);
    }
    sep.push_unseparated(")");
    let result = qb.build().execute(&mut *tx).await?;
    ensure_rows_affected(
        result,
        room_ids.len() as u64,
        format!(
            "one or more rooms in group {} are no longer occupied",
            req.group_id
        ),
    )?;

    let mut qb = sqlx::QueryBuilder::new(
        "INSERT INTO housekeeping (id, room_id, status, triggered_at, created_at) ",
    );
    qb.push_values(&room_ids, |mut b, rid| {
        b.push_bind(uuid::Uuid::new_v4().to_string())
            .push_bind(rid)
            .push_bind("needs_cleaning")
            .push_bind(&now)
            .push_bind(&now);
    });
    qb.build().execute(&mut *tx).await?;

    let mut qb = sqlx::QueryBuilder::new("DELETE FROM room_calendar WHERE booking_id IN (");
    let mut sep = qb.separated(", ");
    for id in &unique_booking_ids {
        sep.push_bind(id);
    }
    sep.push_unseparated(")");
    qb.build().execute(&mut *tx).await?;

    maybe_reassign_master_booking(&mut tx, &req.group_id, &unique_booking_ids).await?;

    let remaining_active: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM bookings WHERE group_id = ? AND status = ?")
            .bind(&req.group_id)
            .bind(status::booking::ACTIVE)
            .fetch_one(&mut *tx)
            .await?;

    sqlx::query("UPDATE booking_groups SET status = ? WHERE id = ?")
        .bind(if remaining_active.0 == 0 {
            GROUP_COMPLETED
        } else {
            GROUP_PARTIAL_CHECKOUT
        })
        .bind(&req.group_id)
        .execute(&mut *tx)
        .await?;

    if let Some(final_paid) = req.final_paid.filter(|amount| *amount > 0.0) {
        let target_booking: (String,) = sqlx::query_as(
            "SELECT id
             FROM bookings
             WHERE group_id = ?
             ORDER BY CASE WHEN status = ? THEN 0 ELSE 1 END, created_at ASC
             LIMIT 1",
        )
        .bind(&req.group_id)
        .bind(status::booking::ACTIVE)
        .fetch_one(&mut *tx)
        .await?;

        record_payment_tx(
            &mut tx,
            &target_booking.0,
            final_paid,
            "Thanh toán group checkout",
        )
        .await?;
    }

    tx.commit().await.map_err(BookingError::from)?;
    Ok(())
}

pub(crate) fn ensure_group_checkout_room_map_still_locked(
    group_id: &str,
    booking_ids: &[String],
    locked_booking_room_map: &std::collections::HashMap<String, String>,
    current_booking_room_map: &std::collections::HashMap<String, String>,
) -> BookingResult<()> {
    for booking_id in booking_ids {
        if current_booking_room_map.get(booking_id) != locked_booking_room_map.get(booking_id) {
            return Err(invalid_state_transition(format!(
                "one or more bookings in group {group_id} changed rooms before checkout"
            )));
        }
    }

    Ok(())
}

fn validate_group_checkin_request(req: &GroupCheckinRequest) -> BookingResult<()> {
    if req.room_ids.is_empty() {
        return Err(BookingError::validation(
            "Phải chọn ít nhất 1 phòng".to_string(),
        ));
    }
    if req.nights <= 0 {
        return Err(BookingError::validation("Số đêm phải > 0".to_string()));
    }
    if !req.room_ids.contains(&req.master_room_id) {
        return Err(BookingError::validation(
            "Phòng đại diện phải nằm trong danh sách phòng".to_string(),
        ));
    }
    let unique_room_count = req
        .room_ids
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len();
    if unique_room_count != req.room_ids.len() {
        return Err(BookingError::validation(
            "Phòng không được lặp trong cùng một group".to_string(),
        ));
    }

    Ok(())
}

async fn validate_rooms_for_group(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    room_ids: &[String],
    is_reservation: bool,
    checkin_date: &str,
    checkout_date: &str,
) -> BookingResult<()> {
    for room_id in room_ids {
        let room_status = sqlx::query_scalar::<_, String>("SELECT status FROM rooms WHERE id = ?")
            .bind(room_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| BookingError::not_found(format!("Phòng {} không tồn tại", room_id)))?;

        if !is_reservation && room_status != status::room::VACANT {
            return Err(BookingError::conflict(format!(
                "Phòng {} không trống (status: {})",
                room_id, room_status
            )));
        }

        let conflicts: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM room_calendar WHERE room_id = ? AND date >= ? AND date < ?",
        )
        .bind(room_id)
        .bind(checkin_date)
        .bind(checkout_date)
        .fetch_one(&mut **tx)
        .await?;

        if conflicts.0 > 0 {
            return Err(BookingError::conflict(format!(
                "Phòng {} có lịch trùng trong khoảng ngày đã chọn",
                room_id
            )));
        }
    }

    Ok(())
}

async fn maybe_reassign_master_booking(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    group_id: &str,
    checked_out_booking_ids: &[String],
) -> BookingResult<()> {
    let current_master = sqlx::query_scalar::<_, String>(
        "SELECT master_booking_id FROM booking_groups WHERE id = ? LIMIT 1",
    )
    .bind(group_id)
    .fetch_optional(&mut **tx)
    .await?;

    let Some(current_master) = current_master else {
        return Ok(());
    };

    if !checked_out_booking_ids.contains(&current_master) {
        return Ok(());
    }

    let next_master = sqlx::query_scalar::<_, String>(
        "SELECT id FROM bookings WHERE group_id = ? AND status = ? ORDER BY created_at ASC LIMIT 1",
    )
    .bind(group_id)
    .bind(status::booking::ACTIVE)
    .fetch_optional(&mut **tx)
    .await?;

    if let Some(next_master) = next_master {
        sqlx::query("UPDATE bookings SET is_master_room = 0 WHERE group_id = ?")
            .bind(group_id)
            .execute(&mut **tx)
            .await?;
        sqlx::query("UPDATE bookings SET is_master_room = 1 WHERE id = ?")
            .bind(&next_master)
            .execute(&mut **tx)
            .await?;
        sqlx::query("UPDATE booking_groups SET master_booking_id = ? WHERE id = ?")
            .bind(&next_master)
            .bind(group_id)
            .execute(&mut **tx)
            .await?;
    } else {
        sqlx::query("UPDATE bookings SET is_master_room = 0 WHERE group_id = ?")
            .bind(group_id)
            .execute(&mut **tx)
            .await?;
        sqlx::query("UPDATE booking_groups SET master_booking_id = NULL WHERE id = ?")
            .bind(group_id)
            .execute(&mut **tx)
            .await?;
    }

    Ok(())
}

async fn insert_group_calendar_rows(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    room_id: &str,
    booking_id: &str,
    from: NaiveDate,
    to: NaiveDate,
    calendar_status: &str,
) -> BookingResult<()> {
    insert_room_calendar_rows(tx, room_id, booking_id, from, to, calendar_status).await
}

async fn fetch_group_tx(
    tx: &mut Transaction<'_, Sqlite>,
    group_id: &str,
) -> BookingResult<BookingGroup> {
    let row = sqlx::query(
        "SELECT id, group_name, master_booking_id, organizer_name, organizer_phone,
                total_rooms, status, notes, created_by, created_at
         FROM booking_groups
         WHERE id = ?",
    )
    .bind(group_id)
    .fetch_optional(&mut **tx)
    .await?;

    let row =
        row.ok_or_else(|| BookingError::not_found(format!("Không tìm thấy group {}", group_id)))?;

    Ok(BookingGroup {
        id: row.get("id"),
        group_name: row.get("group_name"),
        master_booking_id: row.get("master_booking_id"),
        organizer_name: row.get("organizer_name"),
        organizer_phone: row.get("organizer_phone"),
        total_rooms: row.get("total_rooms"),
        status: row.get("status"),
        notes: row.get("notes"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
    })
}

#[allow(dead_code)]
async fn fetch_group(pool: &Pool<Sqlite>, group_id: &str) -> BookingResult<BookingGroup> {
    let row = sqlx::query(
        "SELECT id, group_name, master_booking_id, organizer_name, organizer_phone,
                total_rooms, status, notes, created_by, created_at
         FROM booking_groups
         WHERE id = ?",
    )
    .bind(group_id)
    .fetch_optional(pool)
    .await?;

    let row =
        row.ok_or_else(|| BookingError::not_found(format!("Không tìm thấy group {}", group_id)))?;

    Ok(BookingGroup {
        id: row.get("id"),
        group_name: row.get("group_name"),
        master_booking_id: row.get("master_booking_id"),
        organizer_name: row.get("organizer_name"),
        organizer_phone: row.get("organizer_phone"),
        total_rooms: row.get("total_rooms"),
        status: row.get("status"),
        notes: row.get("notes"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
    })
}

fn parse_date(value: &str) -> BookingResult<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .map_err(|error| BookingError::datetime_parse(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_group_checkin_command_error_maps_invalid_state_transition_code() {
        let error = map_group_checkin_command_error(BookingError::conflict(format!(
            "{}: room R101 is no longer vacant",
            codes::CONFLICT_INVALID_STATE_TRANSITION
        )));

        assert_eq!(error.code, codes::CONFLICT_INVALID_STATE_TRANSITION);
    }
}
