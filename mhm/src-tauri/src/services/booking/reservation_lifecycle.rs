use chrono::{Local, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{Pool, Row, Sqlite, Transaction};

use crate::{
    aggregate_locks::{booking_key, global_manager, room_key, AggregateLockGuard},
    app_error::{codes, CommandError, CommandResult},
    command_idempotency::{
        system_error, CommandLedgerResultSummary, CommandLedgerSummary, IdempotentCommandResult,
        ResolvedWriteCommandGuard, SanitizedLedgerIntent, WriteCommandContext,
        WriteCommandExecutor, WriteCommandRequest,
    },
    db_error_monitoring::{classify_db_error_code, is_room_unavailable_conflict_message},
    domain::booking::{BookingError, BookingResult, OriginSideEffect},
    models::{status, Booking, CreateReservationRequest, ModifyReservationRequest},
    money::{validate_non_negative_money_vnd, MoneyVnd},
    outbox::{OutboxAggregateKeySource, OutboxEventSpec},
};

use super::{
    billing_service::{
        record_cancellation_fee_tx, record_cancellation_fee_with_origin_tx, record_charge_tx,
        record_charge_with_origin_tx, record_deposit_tx, record_deposit_with_origin_tx,
    },
    guest_service::{create_reservation_guest_manifest, link_booking_guests},
    pricing_service::calculate_stay_price_tx,
    stay_lifecycle::MAX_RATE_PER_NIGHT_VND,
    support::{
        ensure_one_row_affected, insert_room_calendar_rows, invalid_state_transition,
        lock_booking_and_read_room, merge_pricing_snapshot, read_money_vnd_or_zero,
        read_money_vnd_strict, validate_non_negative_booking_money, FolioLock,
    },
};

#[cfg(test)]
use super::support::{begin_immediate_tx, fetch_booking, lookup_booking_room_id};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReservationCancelResponse {
    pub ok: bool,
    pub booking_id: String,
}

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

fn map_reservation_command_error(error: BookingError) -> CommandError {
    match error {
        BookingError::Validation(message)
            if message == "Number of nights must be greater than 0" =>
        {
            CommandError::user(codes::BOOKING_INVALID_NIGHTS, "Số đêm phải lớn hơn 0")
        }
        BookingError::NotFound(message)
            if message.starts_with("Không tìm thấy booking ")
                || message.starts_with("Booking not found:") =>
        {
            CommandError::user(codes::BOOKING_NOT_FOUND, message)
        }
        BookingError::Validation(message) | BookingError::Conflict(message) => {
            if message.contains(codes::CONFLICT_INVALID_STATE_TRANSITION) {
                return CommandError::user(codes::CONFLICT_INVALID_STATE_TRANSITION, message);
            }
            if let Some(mapped) = map_known_reservation_command_error(&message) {
                return mapped;
            }
            CommandError::user(codes::BOOKING_INVALID_STATE, message)
        }
        BookingError::DatabaseWrite(message) | BookingError::Database(message) => {
            if let Some(mapped) = map_known_reservation_command_error(&message) {
                return mapped;
            }
            if message.contains("FOREIGN KEY constraint failed") {
                return CommandError::user(codes::BOOKING_NOT_FOUND, "Booking not found");
            }
            CommandError::system(codes::SYSTEM_INTERNAL_ERROR, message)
        }
        BookingError::DateTimeParse(message) | BookingError::NotFound(message) => {
            CommandError::system(codes::SYSTEM_INTERNAL_ERROR, message)
        }
    }
}

pub(crate) fn canonical_vnd_units(amount: Option<MoneyVnd>) -> CommandResult<i64> {
    let Some(amount) = amount else {
        return Ok(0);
    };

    validate_non_negative_money_vnd(amount, "deposit_amount")
}

fn create_room_lock_keys(intent: &serde_json::Value) -> CommandResult<Vec<String>> {
    let room_id = intent
        .get("room_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| system_error("reservation create hash payload missing room_id"))?;
    Ok(vec![room_key(room_id)?])
}

fn reservation_booking_lock_keys(intent: &serde_json::Value) -> CommandResult<Vec<String>> {
    let booking_id = intent
        .get("booking_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| system_error("reservation hash payload missing booking_id"))?;
    Ok(vec![booking_key(booking_id)?])
}

struct ReservationResolvedGuard {
    room_id: String,
    _guard: AggregateLockGuard,
}

async fn resolve_room_lock(
    room_id: String,
) -> CommandResult<ResolvedWriteCommandGuard<ReservationResolvedGuard>> {
    let lock_key = room_key(&room_id)?;
    let guard = global_manager().acquire([lock_key.clone()]).await?;
    Ok(ResolvedWriteCommandGuard::new(
        ReservationResolvedGuard {
            room_id,
            _guard: guard,
        },
        [lock_key],
    ))
}

async fn resolve_reservation_lock(
    pool: Pool<Sqlite>,
    booking_id: String,
) -> CommandResult<ResolvedWriteCommandGuard<ReservationResolvedGuard>> {
    let (guard, room_id) = lock_booking_and_read_room(
        &pool,
        &booking_id,
        FolioLock::Skip,
        map_reservation_command_error,
    )
    .await?;
    let guard = guard
        .acquire_next(global_manager(), [room_key(&room_id)?])
        .await?;
    let lock_keys = guard.keys().to_vec();

    Ok(ResolvedWriteCommandGuard::new(
        ReservationResolvedGuard {
            room_id,
            _guard: guard,
        },
        lock_keys,
    ))
}

fn command_origin_key(ctx: &WriteCommandContext) -> String {
    format!("{}:{}", ctx.command_name, ctx.idempotency_key)
}

pub async fn create_reservation_tx(
    tx: &mut Transaction<'_, Sqlite>,
    req: CreateReservationRequest,
    origin: Option<&OriginSideEffect>,
) -> BookingResult<String> {
    let derived_nights =
        validate_requested_nights(&req.check_in_date, &req.check_out_date, req.nights)?;
    validate_reservation_rate_override(req.rate_override_per_night)?;

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
    let deposit_amount = req
        .deposit_amount
        .map(|amount| validate_non_negative_booking_money(amount, "deposit_amount"))
        .transpose()?
        .unwrap_or(0);

    // Giá tay đè giá engine, và đè PHẲNG: tổng tiền là `rate × nights`, không
    // cộng thêm dòng nào engine tính — cùng luật `check_in_tx` (Task 13).
    let (total_price, rate_overridden_at, pricing_snapshot) = match req.rate_override_per_night {
        Some(rate) => {
            // `validate_reservation_rate_override` đã chặn giá ngoài biên
            // trước khi tới được đây; giữ nguyên bản sao này làm lưới chặn ở
            // tầng transaction, cùng hình "belt-and-braces" với
            // `set_booking_rate_tx`/`check_in_tx`.
            if rate <= 0 || rate > MAX_RATE_PER_NIGHT_VND {
                return Err(BookingError::validation(
                    "Giá mỗi đêm không hợp lệ".to_string(),
                ));
            }
            let total =
                crate::pricing::checked_mul_money(rate, i64::from(derived_nights), "total_price")
                    .map_err(BookingError::validation)?;

            // Giá engine chỉ tính để LƯU LẠI trong pricing_snapshot cho chủ
            // khách sạn tra cứu sau này (đã giảm giá cho ai bao nhiêu), không
            // dùng làm tiền thật — nên lỗi ở bước này không được làm hỏng cả
            // lượt tạo đặt trước. Lỗi thì lưu `null` (không rõ), không lưu 0.
            //
            // Dùng `req.guests` THẬT (không viết cứng `None` như `check_in_tx`):
            // đường không-override của reservation (nhánh `None` ngay dưới)
            // cũng gọi engine với `req.guests`, nên đây mới là tham số khớp
            // với "giá engine sẽ tính cho đúng lượt đặt này" — viết cứng
            // `None` sẽ ghi một `engine_total` không thật, nói dối chủ khách
            // sạn về việc đã giảm bao nhiêu.
            let engine_total = calculate_stay_price_tx(
                tx,
                &req.room_id,
                &req.check_in_date,
                &req.check_out_date,
                "nightly",
                req.guests,
            )
            .await
            .map(|pricing| pricing.total)
            .ok();

            let snapshot = merge_pricing_snapshot(
                None,
                "manual_rate",
                json!({
                    "rate_per_night": rate,
                    "engine_total": engine_total,
                    "set_at": now.clone(),
                }),
            );

            (total, Some(now.clone()), Some(snapshot))
        }
        None => {
            let pricing = calculate_stay_price_tx(
                tx,
                &req.room_id,
                &req.check_in_date,
                &req.check_out_date,
                "nightly",
                req.guests,
            )
            .await?;
            (pricing.total, None, None)
        }
    };

    // Cọc vượt tổng tiền đẩy booking vào ngõ cụt y hệt `check_in_tx`:
    // `record_deposit_tx` dưới đây cộng thẳng `deposit_amount` vào
    // `paid_amount`, `confirm_reservation_tx` mang `paid_amount` đó đi
    // nguyên vẹn khi kích hoạt booking, và `check_out_tx` cuối cùng từ chối
    // thẳng khi `already_paid > final_total`. Đặt SAU match ở trên vì đây là
    // chỗ ĐẦU TIÊN cả hai nhánh (giá tay lẫn giá engine) đều đã có
    // `total_price` thật trong tay — `validate_reservation_rate_override` chỉ
    // chặn được nhánh giá tay, bỏ lọt đường không-override.
    if deposit_amount > total_price {
        return Err(BookingError::validation(format!(
            "Tiền cọc {deposit_amount}đ, cao hơn tổng tiền {total_price}đ — sửa lại giá hoặc số tiền cọc"
        )));
    }

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
            scheduled_checkout, pricing_snapshot, guests, rate_overridden_at, created_at
         ) VALUES (?, ?, ?, ?, ?, NULL, ?, ?, 0, ?, ?, ?, NULL, 'reservation', 'nightly', ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&booking_id)
    .bind(&req.room_id)
    .bind(&guest_manifest.primary_guest_id)
    .bind(&req.check_in_date)
    .bind(&req.check_out_date)
    .bind(derived_nights)
    .bind(total_price)
    .bind(status::booking::BOOKED)
    .bind(req.source.as_deref().unwrap_or("phone"))
    .bind(req.notes.as_deref())
    .bind(deposit_amount)
    .bind(req.guest_phone.as_deref())
    .bind(&req.check_in_date)
    .bind(&req.check_out_date)
    .bind(&pricing_snapshot)
    .bind(req.guests)
    .bind(&rate_overridden_at)
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

    if deposit_amount > 0 {
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

#[cfg(test)]
pub async fn fetch_booking_by_id(pool: &Pool<Sqlite>, booking_id: &str) -> BookingResult<Booking> {
    fetch_booking(
        pool,
        booking_id,
        format!("Booking not found: {}", booking_id),
        read_money_vnd_strict,
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
                source, notes, created_at, rate_overridden_at
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
        total_price: read_money_vnd_strict(&row, "total_price"),
        paid_amount: read_money_vnd_strict(&row, "paid_amount"),
        status: row.get("status"),
        source: row.get("source"),
        notes: row.get("notes"),
        created_at: row.get("created_at"),
        rate_overridden_at: row.get("rate_overridden_at"),
    })
}

#[cfg(test)]
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
    let deposit_vnd_units = canonical_vnd_units(req.deposit_amount)?;

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
        "deposit_vnd_units": deposit_vnd_units,
        // Ảnh hưởng trực tiếp tới `total_price` như mọi trường khác ở trên —
        // thiếu nó thì đổi giá tay dưới cùng một idempotency key sẽ bị lặp
        // lại kết quả CŨ trong im lặng thay vì bị báo
        // `CONFLICT_IDEMPOTENCY_HASH_MISMATCH`. Cùng lý do
        // `build_check_in_hash_payload` ở `stay_lifecycle.rs`.
        "rate_override_per_night": req.rate_override_per_night,
    });

    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("reservation.create.v1")),
        ("room_present", json!(true)),
        ("check_in_date", json!(check_in_date.clone())),
        ("check_out_date", json!(check_out_date.clone())),
        ("nights", json!(nights)),
        ("deposit_present", json!(deposit_vnd_units > 0)),
        ("deposit_vnd_units", json!(deposit_vnd_units)),
    ])?;
    let summary = CommandLedgerSummary::new("Create reservation")?
        .with_aggregate_ref("room", "room", None::<String>)?
        .with_business_date(check_in_date.clone())?;
    let request = WriteCommandRequest::new_sanitized(hash_payload, ledger_intent, summary)?
        .with_primary_aggregate_key(format!("room:{room_id}"))
        .with_lock_key_deriver(create_room_lock_keys)
        .with_success_summary(CommandLedgerResultSummary::success("Reservation created")?)
        .with_outbox_event(OutboxEventSpec::new(
            "booking.reservation_created",
            OutboxAggregateKeySource::response_field("booking", "id"),
            &["bookings", "rooms"],
        )?);

    let request_for_service = req;
    let origin_idempotency_key = command_origin_key(ctx);
    let room_id_for_guard = room_id.clone();

    WriteCommandExecutor::new(pool.clone())
        .execute_with_resolved_guard(
            ctx,
            request,
            move || resolve_room_lock(room_id_for_guard),
            move |tx, _guard| {
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
            },
        )
        .await
}

pub async fn cancel_reservation_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    locked_room_id: &str,
    origin: Option<&OriginSideEffect>,
) -> BookingResult<ReservationCancelResponse> {
    let booking = sqlx::query(
        "SELECT room_id, status, COALESCE(deposit_amount, 0) AS deposit_amount
         FROM bookings
         WHERE id = ?",
    )
    .bind(booking_id)
    .fetch_optional(&mut **tx)
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

    let deposit_amount = read_money_vnd_or_zero(&booking, "deposit_amount");

    let result = sqlx::query("UPDATE bookings SET status = ? WHERE id = ? AND status = ?")
        .bind(status::booking::CANCELLED)
        .bind(booking_id)
        .bind(status::booking::BOOKED)
        .execute(&mut **tx)
        .await?;
    ensure_one_row_affected(
        result,
        format!("reservation {booking_id} is no longer booked"),
    )?;

    sqlx::query("DELETE FROM room_calendar WHERE booking_id = ? AND status = ?")
        .bind(booking_id)
        .bind(status::calendar::BOOKED)
        .execute(&mut **tx)
        .await?;

    if deposit_amount > 0 {
        match origin {
            Some(origin) => {
                record_cancellation_fee_with_origin_tx(
                    tx,
                    booking_id,
                    deposit_amount,
                    "Deposit retained (cancellation)",
                    origin,
                )
                .await?;
            }
            None => {
                record_cancellation_fee_tx(
                    tx,
                    booking_id,
                    deposit_amount,
                    "Deposit retained (cancellation)",
                )
                .await?;
            }
        }
    }

    let remaining_booked: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM room_calendar WHERE room_id = ? AND status = ?")
            .bind(&room_id)
            .bind(status::calendar::BOOKED)
            .fetch_one(&mut **tx)
            .await?;

    let room_status = sqlx::query_scalar::<_, String>("SELECT status FROM rooms WHERE id = ?")
        .bind(&room_id)
        .fetch_one(&mut **tx)
        .await?;

    if room_status == status::room::BOOKED && remaining_booked.0 == 0 {
        sqlx::query("UPDATE rooms SET status = ? WHERE id = ?")
            .bind(status::room::VACANT)
            .bind(&room_id)
            .execute(&mut **tx)
            .await?;
    }

    Ok(ReservationCancelResponse {
        ok: true,
        booking_id: booking_id.to_string(),
    })
}

#[cfg(test)]
pub async fn cancel_reservation(pool: &Pool<Sqlite>, booking_id: &str) -> BookingResult<()> {
    let locked_room_id = lookup_booking_room_id(pool, booking_id).await?;
    let _lock_guard = global_manager()
        .acquire([
            booking_key(booking_id).map_err(|error| BookingError::validation(error.message))?,
            room_key(&locked_room_id).map_err(|error| BookingError::validation(error.message))?,
        ])
        .await
        .map_err(|error| BookingError::validation(error.message))?;

    let mut tx = begin_immediate_tx(pool).await?;
    cancel_reservation_tx(&mut tx, booking_id, &locked_room_id, None).await?;
    tx.commit().await.map_err(BookingError::from)?;

    Ok(())
}

pub async fn confirm_reservation_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    locked_room_id: &str,
    origin: Option<&OriginSideEffect>,
) -> BookingResult<Booking> {
    let reservation = load_booked_reservation(tx, booking_id).await?;
    if reservation.room_id != locked_room_id {
        return Err(invalid_state_transition(format!(
            "reservation {booking_id} changed rooms before confirmation"
        )));
    }
    reject_no_show_confirmation(tx, booking_id).await?;

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
        tx,
        &reservation.room_id,
        &today.format("%Y-%m-%d").to_string(),
        &effective_checkout,
        &reservation.pricing_type,
        reservation.guests,
    )
    .await?;
    let total_price = pricing.total;
    let actual_nights = (effective_checkout_date - today).num_days() as i32;
    let check_in_at = now.to_rfc3339();
    // Cột `expected_checkout` của booking đang ở phải là RFC3339, giống hệt
    // đường walk-in trong `stay_lifecycle::check_in_tx`: `extend_stay` đọc cột
    // này bằng `DateTime::parse_from_rfc3339`, gặp chuỗi chỉ-có-ngày là chrono
    // báo "premature end of input". `effective_checkout` dạng ngày vẫn giữ
    // nguyên cho phần tính giá và ghi lịch phòng bên dưới.
    let expected_checkout = (now + chrono::Duration::days(actual_nights as i64)).to_rfc3339();

    sqlx::query("DELETE FROM room_calendar WHERE booking_id = ?")
        .bind(booking_id)
        .execute(&mut **tx)
        .await?;

    insert_calendar_rows(
        tx,
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
    .bind(&expected_checkout)
    .bind(actual_nights)
    .bind(total_price)
    .bind(reservation.paid_amount)
    .bind(booking_id)
    .bind(status::booking::BOOKED)
    .execute(&mut **tx)
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
        .execute(&mut **tx)
        .await?;
    ensure_one_row_affected(
        result,
        format!(
            "room {} is no longer available for confirmation",
            reservation.room_id
        ),
    )?;

    match origin {
        Some(origin) => {
            record_charge_with_origin_tx(
                tx,
                booking_id,
                total_price,
                "Room charge (reservation)",
                check_in_at,
                origin,
            )
            .await?;
        }
        None => {
            record_charge_tx(
                tx,
                booking_id,
                total_price,
                "Room charge (reservation)",
                check_in_at,
            )
            .await?;
        }
    }

    fetch_booking_by_id_tx(tx, booking_id).await
}

#[cfg(test)]
pub async fn confirm_reservation(pool: &Pool<Sqlite>, booking_id: &str) -> BookingResult<Booking> {
    let locked_room_id = lookup_booking_room_id(pool, booking_id).await?;
    let _lock_guard = global_manager()
        .acquire([
            booking_key(booking_id).map_err(|error| BookingError::validation(error.message))?,
            room_key(&locked_room_id).map_err(|error| BookingError::validation(error.message))?,
        ])
        .await
        .map_err(|error| BookingError::validation(error.message))?;

    let mut tx = begin_immediate_tx(pool).await?;
    let booking = confirm_reservation_tx(&mut tx, booking_id, &locked_room_id, None).await?;
    tx.commit().await.map_err(BookingError::from)?;

    Ok(booking)
}

pub async fn modify_reservation_tx(
    tx: &mut Transaction<'_, Sqlite>,
    req: ModifyReservationRequest,
    locked_room_id: &str,
) -> BookingResult<Booking> {
    let derived_nights = validate_requested_nights(
        &req.new_check_in_date,
        &req.new_check_out_date,
        req.new_nights,
    )? as i64;

    let reservation = load_booked_reservation(tx, &req.booking_id).await?;
    if reservation.room_id != locked_room_id {
        return Err(invalid_state_transition(format!(
            "reservation {} changed rooms before modification",
            req.booking_id
        )));
    }

    sqlx::query("DELETE FROM room_calendar WHERE booking_id = ? AND status = ?")
        .bind(&req.booking_id)
        .bind(status::calendar::BOOKED)
        .execute(&mut **tx)
        .await?;

    let conflicts = sqlx::query(
        "SELECT date FROM room_calendar WHERE room_id = ? AND date >= ? AND date < ? ORDER BY date ASC",
    )
    .bind(&reservation.room_id)
    .bind(&req.new_check_in_date)
    .bind(&req.new_check_out_date)
    .fetch_all(&mut **tx)
    .await?;

    if let Some(first_conflict) = conflicts.first() {
        let first_date: String = first_conflict.get("date");
        return Err(BookingError::conflict(format!(
            "Room {} is booked on {}. Cannot modify.",
            reservation.room_id, first_date
        )));
    }

    let effective_guests = req.new_guests.or(reservation.guests);
    let pricing = calculate_stay_price_tx(
        tx,
        &reservation.room_id,
        &req.new_check_in_date,
        &req.new_check_out_date,
        &reservation.pricing_type,
        effective_guests,
    )
    .await?;
    let total_price = pricing.total;

    let result = sqlx::query(
        "UPDATE bookings
         SET check_in_at = ?, expected_checkout = ?, scheduled_checkin = ?, scheduled_checkout = ?, nights = ?, total_price = ?, guests = ?
         WHERE id = ? AND status = ? AND room_id = ?",
    )
    .bind(&req.new_check_in_date)
    .bind(&req.new_check_out_date)
    .bind(&req.new_check_in_date)
    .bind(&req.new_check_out_date)
    .bind(derived_nights)
    .bind(total_price)
    .bind(effective_guests)
    .bind(&req.booking_id)
    .bind(status::booking::BOOKED)
    .bind(locked_room_id)
    .execute(&mut **tx)
    .await?;
    ensure_one_row_affected(
        result,
        format!("reservation {} is no longer booked", req.booking_id),
    )?;

    insert_booked_calendar_rows(
        tx,
        &reservation.room_id,
        &req.booking_id,
        &req.new_check_in_date,
        &req.new_check_out_date,
    )
    .await?;

    fetch_booking_by_id_tx(tx, &req.booking_id).await
}

#[cfg(test)]
pub async fn modify_reservation(
    pool: &Pool<Sqlite>,
    req: ModifyReservationRequest,
) -> BookingResult<Booking> {
    let locked_room_id = lookup_booking_room_id(pool, &req.booking_id).await?;
    let _lock_guard = global_manager()
        .acquire([
            booking_key(&req.booking_id)
                .map_err(|error| BookingError::validation(error.message))?,
            room_key(&locked_room_id).map_err(|error| BookingError::validation(error.message))?,
        ])
        .await
        .map_err(|error| BookingError::validation(error.message))?;

    let mut tx = begin_immediate_tx(pool).await?;
    let booking = modify_reservation_tx(&mut tx, req, &locked_room_id).await?;
    tx.commit().await.map_err(BookingError::from)?;

    Ok(booking)
}

pub async fn cancel_reservation_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    booking_id: &str,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let hash_payload = json!({
        "schema": "reservation.cancel.v1",
        "booking_id": booking_id,
    });
    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("reservation.cancel.v1")),
        ("booking_present", json!(true)),
    ])?;
    let summary = CommandLedgerSummary::new("Cancel reservation")?.with_aggregate_ref(
        "booking",
        "booking",
        None::<String>,
    )?;
    let request = WriteCommandRequest::new_sanitized(hash_payload, ledger_intent, summary)?
        .with_primary_aggregate_key(format!("booking:{booking_id}"))
        .with_lock_key_deriver(reservation_booking_lock_keys)
        .with_success_summary(CommandLedgerResultSummary::success(
            "Reservation cancelled",
        )?)
        .with_outbox_event(OutboxEventSpec::new(
            "booking.reservation_cancelled",
            OutboxAggregateKeySource::response_field("booking", "booking_id"),
            &["bookings", "rooms", "folio"],
        )?);

    let pool_for_guard = pool.clone();
    let booking_id_for_guard = booking_id.to_string();
    let booking_id_for_service = booking_id.to_string();
    let origin_idempotency_key = command_origin_key(ctx);

    WriteCommandExecutor::new(pool.clone())
        .execute_with_resolved_guard(
            ctx,
            request,
            move || resolve_reservation_lock(pool_for_guard, booking_id_for_guard),
            move |tx, guard| {
                Box::pin(async move {
                    let origin =
                        OriginSideEffect::new(origin_idempotency_key, 0).map_err(system_error)?;
                    let response = cancel_reservation_tx(
                        tx,
                        &booking_id_for_service,
                        &guard.room_id,
                        Some(&origin),
                    )
                    .await
                    .map_err(map_reservation_command_error)?;
                    serde_json::to_value(response).map_err(system_error)
                })
            },
        )
        .await
}

pub async fn confirm_reservation_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    booking_id: &str,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let hash_payload = json!({
        "schema": "reservation.confirm.v1",
        "booking_id": booking_id,
    });
    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("reservation.confirm.v1")),
        ("booking_present", json!(true)),
    ])?;
    let summary = CommandLedgerSummary::new("Confirm reservation")?.with_aggregate_ref(
        "booking",
        "booking",
        None::<String>,
    )?;
    let request = WriteCommandRequest::new_sanitized(hash_payload, ledger_intent, summary)?
        .with_primary_aggregate_key(format!("booking:{booking_id}"))
        .with_lock_key_deriver(reservation_booking_lock_keys)
        .with_success_summary(CommandLedgerResultSummary::success(
            "Reservation confirmed",
        )?)
        .with_outbox_event(OutboxEventSpec::new(
            "booking.reservation_confirmed",
            OutboxAggregateKeySource::response_field("booking", "id"),
            &["bookings", "rooms", "folio"],
        )?);

    let pool_for_guard = pool.clone();
    let booking_id_for_guard = booking_id.to_string();
    let booking_id_for_service = booking_id.to_string();
    let origin_idempotency_key = command_origin_key(ctx);

    WriteCommandExecutor::new(pool.clone())
        .execute_with_resolved_guard(
            ctx,
            request,
            move || resolve_reservation_lock(pool_for_guard, booking_id_for_guard),
            move |tx, guard| {
                Box::pin(async move {
                    let origin =
                        OriginSideEffect::new(origin_idempotency_key, 0).map_err(system_error)?;
                    let booking = confirm_reservation_tx(
                        tx,
                        &booking_id_for_service,
                        &guard.room_id,
                        Some(&origin),
                    )
                    .await
                    .map_err(map_reservation_command_error)?;
                    serde_json::to_value(booking).map_err(system_error)
                })
            },
        )
        .await
}

pub async fn modify_reservation_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    req: ModifyReservationRequest,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let hash_payload = json!({
        "schema": "reservation.modify.v1",
        "booking_id": req.booking_id.clone(),
        "new_check_in_date": req.new_check_in_date.clone(),
        "new_check_out_date": req.new_check_out_date.clone(),
        "new_nights": req.new_nights,
    });
    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("reservation.modify.v1")),
        ("booking_present", json!(true)),
        ("nights", json!(req.new_nights)),
    ])?;
    let summary = CommandLedgerSummary::new("Modify reservation")?
        .with_aggregate_ref("booking", "booking", None::<String>)?
        .with_business_date(req.new_check_in_date.clone())?;
    let request = WriteCommandRequest::new_sanitized(hash_payload, ledger_intent, summary)?
        .with_primary_aggregate_key(format!("booking:{}", req.booking_id))
        .with_lock_key_deriver(reservation_booking_lock_keys)
        .with_success_summary(CommandLedgerResultSummary::success("Reservation modified")?)
        .with_outbox_event(OutboxEventSpec::new(
            "booking.reservation_modified",
            OutboxAggregateKeySource::response_field("booking", "id"),
            &["bookings", "rooms"],
        )?);

    let pool_for_guard = pool.clone();
    let booking_id_for_guard = req.booking_id.clone();

    WriteCommandExecutor::new(pool.clone())
        .execute_with_resolved_guard(
            ctx,
            request,
            move || resolve_reservation_lock(pool_for_guard, booking_id_for_guard),
            move |tx, guard| {
                Box::pin(async move {
                    let booking = modify_reservation_tx(tx, req, &guard.room_id)
                        .await
                        .map_err(map_reservation_command_error)?;
                    serde_json::to_value(booking).map_err(system_error)
                })
            },
        )
        .await
}

/// Trần số đêm cho một đặt phòng. Trước nhánh này, ngày đi chỉ đọc được (suy
/// ra từ một ô số đêm mang `max={90}`), nên 90 đêm là trần cứng theo cấu trúc.
/// Nhánh này cho gõ tay ngày đi trực tiếp và xoá ô số đêm đó — nhưng không có
/// gì thay thế trần cũ, nên một lỗi gõ năm (`2036` thay vì `2026`) tạo ra một
/// đặt phòng khoảng 3650 đêm, khoá phòng đó cả thập kỷ trong `room_calendar`.
/// Đây là khôi phục lại trần cũ, không phải đặt ra chính sách mới.
///
/// `pub` vì tầng dựng thẻ của trợ lý (`agent::assistant::draft`) kiểm lại đúng
/// trần này **trước** khi dựng thẻ đặt phòng, để lễ tân không bấm *Đồng ý* cho
/// một cái thẻ mà lệnh chắc chắn sẽ từ chối. Hai chỗ đọc **cùng một** hằng số:
/// chép tay con số 90 sang bên kia là dựng ra hai chính sách trôi độc lập.
pub const MAX_RESERVATION_NIGHTS: i64 = 90;

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
    if derived_nights > MAX_RESERVATION_NIGHTS {
        return Err(BookingError::validation(format!(
            "Number of nights must not exceed {}",
            MAX_RESERVATION_NIGHTS
        )));
    }
    if requested_nights != derived_nights as i32 {
        return Err(BookingError::validation(format!(
            "Number of nights must match the date range (expected {})",
            derived_nights
        )));
    }

    Ok(derived_nights as i32)
}

/// Chặn giá tay ngoài biên NGAY TẠI ĐÂY, trước khi bất cứ phép nhân nào
/// (`checked_mul_money`) có cơ hội chạy trên một `rate` chưa kiểm biên — nếu
/// không, `checked_mul_money` có thể ăn một giá trị tràn số / không an toàn
/// và trả thẳng ra người dùng một thông báo TIẾNG ANH (qua command error
/// mapper), hoặc — với giá âm — để lọt xuống guard cọc-vượt-tổng, nơi đọc
/// nhầm một tổng âm ra như một mức giá. Cùng thứ tự với `set_booking_rate_tx`
/// và `validate_check_in_request` ở `stay_lifecycle.rs`: biên trước, phép
/// nhân sau. Đây là finding Important của review Task 13, không được lặp lại.
///
/// Guard cọc-vượt-tổng KHÔNG nằm ở đây: ở đây (chưa mở transaction, chưa gọi
/// engine) chỉ biết được tổng tiền khi có giá tay, nên không thể chặn đường
/// không-override — guard đó nằm trong `create_reservation_tx`, sau khi
/// `total_price` đã biết ở CẢ hai nhánh.
fn validate_reservation_rate_override(
    rate_override_per_night: Option<MoneyVnd>,
) -> BookingResult<()> {
    if let Some(rate) = rate_override_per_night {
        if rate <= 0 || rate > MAX_RATE_PER_NIGHT_VND {
            return Err(BookingError::validation(
                "Giá mỗi đêm không hợp lệ".to_string(),
            ));
        }
    }

    Ok(())
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
        "SELECT room_id, status, paid_amount, scheduled_checkout, pricing_type, guests
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
        paid_amount: read_money_vnd_or_zero(&row, "paid_amount"),
        scheduled_checkout,
        pricing_type: row
            .get::<Option<String>, _>("pricing_type")
            .unwrap_or_else(|| "nightly".to_string()),
        guests: row.get("guests"),
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
    paid_amount: MoneyVnd,
    scheduled_checkout: String,
    pricing_type: String,
    guests: Option<i32>,
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

    #[test]
    fn map_reservation_command_error_marks_locked_lookup_retryable() {
        let error =
            super::map_reservation_command_error(BookingError::database("database is locked"));

        assert_eq!(error.code, crate::app_error::codes::DB_LOCKED_RETRYABLE);
        assert!(error.retryable);
    }
}
