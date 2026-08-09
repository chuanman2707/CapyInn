use chrono::{DateTime, Duration, Local, NaiveDate};
use serde_json::json;
use sqlx::{Pool, Row, Sqlite, Transaction};

use crate::{
    app_error::{codes, CommandError, CommandResult},
    command_idempotency::{
        system_error, CommandLedgerResultSummary, CommandLedgerSummary, IdempotentCommandResult,
        ResolvedWriteCommandGuard, SanitizedLedgerIntent, WriteCommandContext,
        WriteCommandExecutor, WriteCommandRequest,
    },
    db_error_monitoring::{classify_db_error_code, is_room_unavailable_conflict_message},
    domain::booking::{BookingError, BookingResult, OriginSideEffect},
    models::{
        status, Booking, CheckInRequest, CheckOutRequest, CheckOutResponse, CheckoutSettlementMode,
        CheckoutSettlementPreview, CheckoutSettlementPreviewRequest,
    },
    money::MoneyVnd,
    outbox::{OutboxAggregateKeySource, OutboxEventSpec},
};

use super::{
    billing_service::{
        record_charge_tx, record_charge_with_origin_tx, record_payment_tx,
        record_payment_with_origin_tx,
    },
    guest_service::{create_guest_manifest, guest_hash_payload_entries, link_booking_guests},
    pricing_service::calculate_stay_price_tx,
    support::{
        begin_tx, ensure_one_row_affected, insert_room_calendar_rows, invalid_state_transition,
        lock_booking_and_read_room, lookup_booking_expected_checkout, lookup_booking_total_price,
        map_room_calendar_insert_error, merge_pricing_snapshot, parse_booking_datetime,
        read_money_vnd_or_zero, room_calendar_stays_tx, room_stays_to_json,
        validate_non_negative_booking_money, FolioLock, RoomStayRow,
    },
};

#[cfg(test)]
use super::support::{begin_immediate_tx, fetch_booking, lookup_booking_room_id};

pub(super) fn mark_write_db_error(error: BookingError) -> BookingError {
    match error {
        BookingError::Database(message) => BookingError::database_write(message),
        other => other,
    }
}

fn ensure_locked_room_matches_booking(
    locked_room_id: &str,
    current_room_id: &str,
    message: impl AsRef<str>,
) -> BookingResult<()> {
    if locked_room_id == current_room_id {
        Ok(())
    } else {
        Err(invalid_state_transition(message))
    }
}

fn stay_command_origin_key(ctx: &WriteCommandContext) -> String {
    format!("{}:{}", ctx.command_name, ctx.idempotency_key)
}

pub(super) fn map_check_in_command_error(error: BookingError) -> CommandError {
    match error {
        BookingError::Validation(message) => {
            CommandError::user(codes::BOOKING_INVALID_STATE, message)
        }
        BookingError::Conflict(message) => {
            if is_room_unavailable_conflict_message(&message) {
                return CommandError::user(codes::CONFLICT_ROOM_UNAVAILABLE, message);
            }
            CommandError::user(codes::BOOKING_INVALID_STATE, message)
        }
        BookingError::NotFound(message) if message.starts_with("Không tìm thấy phòng ") => {
            CommandError::user(codes::ROOM_NOT_FOUND, message)
        }
        BookingError::NotFound(message) => CommandError::user(codes::BOOKING_NOT_FOUND, message),
        BookingError::DatabaseWrite(message) | BookingError::Database(message) => {
            if classify_db_error_code(&message) == Some(codes::DB_LOCKED_RETRYABLE) {
                return CommandError::system(codes::DB_LOCKED_RETRYABLE, message).retryable(true);
            }
            CommandError::system(codes::SYSTEM_INTERNAL_ERROR, message)
        }
        BookingError::DateTimeParse(message) => {
            CommandError::system(codes::SYSTEM_INTERNAL_ERROR, message)
        }
    }
}

pub(super) fn map_check_out_command_error(error: BookingError) -> CommandError {
    match error {
        BookingError::Validation(message)
            if message == "Tổng quyết toán phải lớn hơn hoặc bằng 0"
                || message == "final_total must be greater than or equal to 0" =>
        {
            CommandError::user(codes::BOOKING_INVALID_SETTLEMENT_TOTAL, message)
        }
        BookingError::Validation(message)
            if message == "Overpaid booking requires refund handling before checkout" =>
        {
            CommandError::user(codes::BOOKING_INVALID_STATE, message)
        }
        BookingError::Validation(message) | BookingError::Conflict(message) => {
            CommandError::user(codes::BOOKING_INVALID_STATE, message)
        }
        BookingError::NotFound(message) => CommandError::user(codes::BOOKING_NOT_FOUND, message),
        BookingError::DatabaseWrite(message) | BookingError::Database(message) => {
            if classify_db_error_code(&message) == Some(codes::DB_LOCKED_RETRYABLE) {
                return CommandError::system(codes::DB_LOCKED_RETRYABLE, message).retryable(true);
            }
            CommandError::system(codes::SYSTEM_INTERNAL_ERROR, message)
        }
        BookingError::DateTimeParse(message) => {
            CommandError::system(codes::SYSTEM_INTERNAL_ERROR, message)
        }
    }
}

pub fn map_extend_stay_command_error(error: BookingError) -> CommandError {
    match error {
        BookingError::Conflict(message) => {
            if is_room_unavailable_conflict_message(&message) {
                return CommandError::user(codes::CONFLICT_ROOM_UNAVAILABLE, message);
            }
            CommandError::user(codes::BOOKING_INVALID_STATE, message)
        }
        BookingError::Validation(message) => {
            CommandError::user(codes::BOOKING_INVALID_STATE, message)
        }
        BookingError::NotFound(message) if message.starts_with("Không tìm thấy phòng ") => {
            CommandError::user(codes::ROOM_NOT_FOUND, message)
        }
        BookingError::NotFound(message) => CommandError::user(codes::BOOKING_NOT_FOUND, message),
        BookingError::DatabaseWrite(message) | BookingError::Database(message) => {
            if classify_db_error_code(&message) == Some(codes::DB_LOCKED_RETRYABLE) {
                return CommandError::system(codes::DB_LOCKED_RETRYABLE, message).retryable(true);
            }
            CommandError::system(codes::SYSTEM_INTERNAL_ERROR, message)
        }
        BookingError::DateTimeParse(message) => {
            CommandError::system(codes::SYSTEM_INTERNAL_ERROR, message)
        }
    }
}

fn build_check_in_hash_payload(req: &CheckInRequest) -> serde_json::Value {
    json!({
        "schema": "stay.check_in.v1",
        "room_id": req.room_id.clone(),
        "guests": guest_hash_payload_entries(&req.guests),
        "nights": req.nights,
        "source": req.source.clone(),
        "notes": req.notes.clone(),
        "paid_amount": req.paid_amount.unwrap_or(0),
        "pricing_type": req.pricing_type.clone().unwrap_or_else(|| "nightly".to_string()),
        // Ảnh hưởng trực tiếp tới `total_price`, giống mọi trường khác ở trên —
        // thiếu nó thì đổi giá tay dưới cùng một idempotency key sẽ bị lặp lại
        // kết quả CŨ trong im lặng thay vì bị báo `CONFLICT_IDEMPOTENCY_HASH_MISMATCH`.
        "rate_override_per_night": req.rate_override_per_night,
    })
}

fn build_check_out_hash_payload(req: &CheckOutRequest) -> serde_json::Value {
    json!({
        "schema": "stay.check_out.v1",
        "booking_id": req.booking_id.clone(),
        "settlement_mode": req.settlement_mode,
        "final_total": req.final_total,
    })
}

fn build_extend_stay_hash_payload(booking_id: &str) -> serde_json::Value {
    json!({
        "schema": "stay.extend.v1",
        "booking_id": booking_id,
        "operation": "add_one_night",
    })
}

fn build_shorten_stay_hash_payload(booking_id: &str) -> serde_json::Value {
    json!({
        "schema": "stay.shorten.v1",
        "booking_id": booking_id,
        "operation": "remove_one_night",
    })
}

fn build_set_booking_rate_hash_payload(
    booking_id: &str,
    rate_per_night: MoneyVnd,
) -> serde_json::Value {
    json!({
        "schema": "stay.set_rate.v1",
        "booking_id": booking_id,
        "rate_per_night": rate_per_night,
    })
}

fn check_in_lock_keys_from_payload(hash_payload: &serde_json::Value) -> CommandResult<Vec<String>> {
    let room_id = hash_payload
        .get("room_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| system_error("check-in lock payload missing room_id"))?;
    Ok(vec![crate::aggregate_locks::room_key(room_id)?])
}

fn check_out_initial_lock_keys_from_payload(
    hash_payload: &serde_json::Value,
) -> CommandResult<Vec<String>> {
    let booking_id = hash_payload
        .get("booking_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| system_error("checkout lock payload missing booking_id"))?;
    Ok(vec![
        crate::aggregate_locks::booking_key(booking_id)?,
        crate::aggregate_locks::folio_key(booking_id)?,
    ])
}

fn extend_stay_initial_lock_keys_from_payload(
    hash_payload: &serde_json::Value,
) -> CommandResult<Vec<String>> {
    let booking_id = hash_payload
        .get("booking_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| system_error("extend-stay lock payload missing booking_id"))?;
    Ok(vec![
        crate::aggregate_locks::booking_key(booking_id)?,
        crate::aggregate_locks::folio_key(booking_id)?,
    ])
}

fn shorten_stay_initial_lock_keys_from_payload(
    hash_payload: &serde_json::Value,
) -> CommandResult<Vec<String>> {
    let booking_id = hash_payload
        .get("booking_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| system_error("shorten-stay lock payload missing booking_id"))?;
    Ok(vec![
        crate::aggregate_locks::booking_key(booking_id)?,
        crate::aggregate_locks::folio_key(booking_id)?,
    ])
}

fn set_booking_rate_initial_lock_keys_from_payload(
    hash_payload: &serde_json::Value,
) -> CommandResult<Vec<String>> {
    let booking_id = hash_payload
        .get("booking_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| system_error("set-booking-rate lock payload missing booking_id"))?;
    Ok(vec![
        crate::aggregate_locks::booking_key(booking_id)?,
        crate::aggregate_locks::folio_key(booking_id)?,
    ])
}

pub(super) async fn fetch_booking_tx(
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

    let row = row
        .ok_or_else(|| BookingError::not_found(format!("Không tìm thấy booking {}", booking_id)))?;

    Ok(Booking {
        id: row.get("id"),
        room_id: row.get("room_id"),
        primary_guest_id: row.get("primary_guest_id"),
        check_in_at: row.get("check_in_at"),
        expected_checkout: row.get("expected_checkout"),
        actual_checkout: row.get("actual_checkout"),
        nights: row.get("nights"),
        total_price: read_money_vnd_or_zero(&row, "total_price"),
        paid_amount: read_money_vnd_or_zero(&row, "paid_amount"),
        status: row.get("status"),
        source: row.get("source"),
        notes: row.get("notes"),
        created_at: row.get("created_at"),
        rate_overridden_at: row.get("rate_overridden_at"),
    })
}

async fn check_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    req: &CheckInRequest,
    user_id: Option<&str>,
    origin_key: Option<&str>,
) -> BookingResult<Booking> {
    validate_check_in_request(req)?;

    let now = Local::now();
    let check_in_at = now.to_rfc3339();
    let expected_checkout = (now + Duration::days(req.nights as i64)).to_rfc3339();
    let checkin_date = now.date_naive();
    let checkout_date = (now + Duration::days(req.nights as i64)).date_naive();
    let pricing_type = req
        .pricing_type
        .clone()
        .unwrap_or_else(|| "nightly".to_string());

    let room = sqlx::query("SELECT id, status FROM rooms WHERE id = ?")
        .bind(&req.room_id)
        .fetch_optional(&mut **tx)
        .await?;

    let room = room
        .ok_or_else(|| BookingError::not_found(format!("Không tìm thấy phòng {}", req.room_id)))?;
    let room_status: String = room.get("status");
    if room_status != status::room::VACANT {
        return Err(BookingError::conflict(format!(
            "Phòng {} không trống (status: {})",
            req.room_id, room_status
        )));
    }

    let conflicts = sqlx::query(
        "SELECT rc.date, COALESCE(g.full_name, '') AS guest_name
         FROM room_calendar rc
         LEFT JOIN bookings b ON b.id = rc.booking_id
         LEFT JOIN guests g ON g.id = b.primary_guest_id
         WHERE rc.room_id = ? AND rc.date >= ? AND rc.date < ?
         ORDER BY rc.date ASC",
    )
    .bind(&req.room_id)
    .bind(checkin_date.format("%Y-%m-%d").to_string())
    .bind(checkout_date.format("%Y-%m-%d").to_string())
    .fetch_all(&mut **tx)
    .await?;

    if let Some(first_conflict) = conflicts.first() {
        let first_date: String = first_conflict.get("date");
        let guest_name: String = first_conflict.get("guest_name");
        let first_conflict_date = NaiveDate::parse_from_str(&first_date, "%Y-%m-%d")
            .map_err(|error| BookingError::datetime_parse(error.to_string()))?;
        let max_nights = (first_conflict_date - checkin_date).num_days();

        return Err(BookingError::conflict(format!(
            "Room {} has a reservation starting {} ({}). Max {} nights.",
            req.room_id, first_date, guest_name, max_nights
        )));
    }

    // Giá tay đè giá engine, và đè PHẲNG: tổng tiền là `rate × nights`, không
    // cộng thêm phụ thu người thứ N hay bất cứ dòng nào engine tính — lễ tân
    // đã mặc cả ra một con số chốt, không phải một cấu phần để engine tính
    // tiếp. `check_in_tx` gốc cũng gọi engine với `guests: None` (không thu
    // phụ thu thêm người), nên hành vi không-override giữ nguyên y hệt cũ.
    let (total_price, rate_overridden_at, pricing_snapshot) = match req.rate_override_per_night {
        Some(rate) => {
            // `validate_check_in_request` đã chặn giá ngoài biên trước khi
            // tới được đây; giữ nguyên bản sao này làm lưới chặn ở tầng
            // transaction, cùng hình "belt-and-braces" với `set_booking_rate_tx`.
            if rate <= 0 || rate > MAX_RATE_PER_NIGHT_VND {
                return Err(BookingError::validation(
                    "Giá mỗi đêm không hợp lệ".to_string(),
                ));
            }
            let total =
                crate::pricing::checked_mul_money(rate, i64::from(req.nights), "total_price")
                    .map_err(BookingError::validation)?;

            // Giá engine chỉ tính để LƯU LẠI trong pricing_snapshot cho chủ
            // khách sạn tra cứu sau này (đã giảm giá cho ai bao nhiêu), không
            // dùng làm tiền thật — nên lỗi ở bước này không được làm hỏng cả
            // lượt nhận phòng. Lỗi thì lưu `null` (không rõ), không lưu 0 — 0
            // sẽ đọc nhầm thành "engine định giá phòng này bằng không".
            let engine_total = calculate_stay_price_tx(
                tx,
                &req.room_id,
                &check_in_at,
                &expected_checkout,
                &pricing_type,
                None,
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
                    "set_at": check_in_at.clone(),
                }),
            );

            (total, Some(check_in_at.clone()), Some(snapshot))
        }
        None => {
            let pricing = calculate_stay_price_tx(
                tx,
                &req.room_id,
                &check_in_at,
                &expected_checkout,
                &pricing_type,
                None,
            )
            .await?;
            (pricing.total, None, None)
        }
    };

    // Thu vượt tổng tiền dẫn tới một booking không có lối thoát: `check_out_tx`
    // từ chối thẳng khi `already_paid > final_total`. Đặt SAU match ở trên vì
    // đây là chỗ ĐẦU TIÊN cả hai nhánh (giá tay lẫn giá engine) đều đã có
    // `total_price` thật trong tay — `validate_check_in_request` trước đây chỉ
    // chặn được nhánh giá tay (nơi nó tự tính lại `rate × nights`), bỏ lọt
    // đường không-override, con đường mọi lượt check-in bình thường đi qua.
    // Đặt TRƯỚC mọi ghi (kể cả `create_guest_manifest` ngay dưới đây) để
    // không để lại việc dở dang khi bị từ chối.
    if let Some(paid_amount) = req.paid_amount {
        if paid_amount > total_price {
            return Err(BookingError::validation(format!(
                "Khách trả {paid_amount}đ, cao hơn tổng tiền {total_price}đ — sửa lại giá hoặc số tiền thu"
            )));
        }
    }

    let booking_id = uuid::Uuid::new_v4().to_string();
    let guest_manifest = create_guest_manifest(tx, &req.guests, &check_in_at)
        .await
        .map_err(mark_write_db_error)?;

    sqlx::query(
        "INSERT INTO bookings (
            id, room_id, primary_guest_id, check_in_at, expected_checkout,
            actual_checkout, nights, total_price, paid_amount, status, source,
            notes, created_by, booking_type, pricing_type, pricing_snapshot,
            rate_overridden_at, created_at
        ) VALUES (?, ?, ?, ?, ?, NULL, ?, ?, 0, ?, ?, ?, ?, 'walk-in', ?, ?, ?, ?)",
    )
    .bind(&booking_id)
    .bind(&req.room_id)
    .bind(&guest_manifest.primary_guest_id)
    .bind(&check_in_at)
    .bind(&expected_checkout)
    .bind(req.nights)
    .bind(total_price)
    .bind(status::booking::ACTIVE)
    .bind(req.source.as_deref().unwrap_or("walk-in"))
    .bind(&req.notes)
    .bind(user_id)
    .bind(&pricing_type)
    .bind(&pricing_snapshot)
    .bind(&rate_overridden_at)
    .bind(&check_in_at)
    .execute(&mut **tx)
    .await
    .map_err(BookingError::from)
    .map_err(mark_write_db_error)?;

    link_booking_guests(tx, &booking_id, &guest_manifest.guest_ids)
        .await
        .map_err(mark_write_db_error)?;

    if let Some(origin_key) = origin_key {
        let origin = crate::domain::booking::OriginSideEffect::new(origin_key, 0)?;
        record_charge_with_origin_tx(
            tx,
            &booking_id,
            total_price,
            "Tiền phòng",
            check_in_at.clone(),
            &origin,
        )
        .await
        .map_err(mark_write_db_error)?;
    } else {
        record_charge_tx(
            tx,
            &booking_id,
            total_price,
            "Tiền phòng",
            check_in_at.clone(),
        )
        .await
        .map_err(mark_write_db_error)?;
    }

    if let Some(paid_amount) = req.paid_amount.filter(|amount| *amount > 0) {
        if let Some(origin_key) = origin_key {
            let origin = crate::domain::booking::OriginSideEffect::new(origin_key, 1)?;
            record_payment_with_origin_tx(
                tx,
                &booking_id,
                paid_amount,
                "Thanh toán khi check-in",
                &origin,
            )
            .await
            .map_err(mark_write_db_error)?;
        } else {
            record_payment_tx(tx, &booking_id, paid_amount, "Thanh toán khi check-in")
                .await
                .map_err(mark_write_db_error)?;
        }
    }

    insert_occupied_calendar_rows(tx, &req.room_id, &booking_id, checkin_date, checkout_date)
        .await
        .map_err(mark_write_db_error)?;

    let result = sqlx::query("UPDATE rooms SET status = ? WHERE id = ? AND status = ?")
        .bind(status::room::OCCUPIED)
        .bind(&req.room_id)
        .bind(status::room::VACANT)
        .execute(&mut **tx)
        .await
        .map_err(BookingError::from)
        .map_err(mark_write_db_error)?;
    ensure_one_row_affected(result, format!("room {} is no longer vacant", req.room_id))?;

    fetch_booking_tx(tx, &booking_id).await
}

#[cfg(test)]
pub async fn check_in(
    pool: &Pool<Sqlite>,
    req: CheckInRequest,
    user_id: Option<String>,
) -> BookingResult<Booking> {
    validate_check_in_request(&req)?;

    let _lock_guard = crate::aggregate_locks::global_manager()
        .acquire([crate::aggregate_locks::room_key(&req.room_id)
            .map_err(|error| BookingError::validation(error.message))?])
        .await
        .map_err(|error| BookingError::validation(error.message))?;

    let mut tx = begin_immediate_tx(pool)
        .await
        .map_err(mark_write_db_error)?;

    let booking = check_in_tx(&mut tx, &req, user_id.as_deref(), None)
        .await
        .map_err(mark_write_db_error)?;

    tx.commit()
        .await
        .map_err(BookingError::from)
        .map_err(mark_write_db_error)?;

    Ok(booking)
}

pub async fn check_in_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    req: CheckInRequest,
    user_id: Option<String>,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    validate_check_in_request(&req).map_err(|error| {
        map_check_in_command_error(error).with_request_id(ctx.request_id.clone())
    })?;

    let hash_payload = build_check_in_hash_payload(&req);
    let notes_present = req
        .notes
        .as_ref()
        .map(|notes| !notes.trim().is_empty())
        .unwrap_or(false);
    let paid_amount = req.paid_amount.unwrap_or(0);
    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("stay.check_in.v1")),
        ("room_present", json!(true)),
        ("guest_count", json!(req.guests.len())),
        ("nights", json!(req.nights)),
        ("source_present", json!(req.source.is_some())),
        ("notes_present", json!(notes_present)),
        ("paid_amount_vnd", json!(paid_amount)),
        ("pricing_type_present", json!(req.pricing_type.is_some())),
    ])?;
    let summary = CommandLedgerSummary::new("Check in")?.with_aggregate_ref(
        "room",
        "room",
        None::<String>,
    )?;
    let request = WriteCommandRequest::new_sanitized(hash_payload.clone(), ledger_intent, summary)?
        .with_primary_aggregate_key(format!("room:{}", req.room_id))
        .with_lock_key_deriver(check_in_lock_keys_from_payload)
        .with_success_summary(CommandLedgerResultSummary::success("Checked in")?)
        .with_outbox_event(OutboxEventSpec::new(
            "booking.checked_in",
            OutboxAggregateKeySource::response_field("booking", "id"),
            &["bookings", "rooms", "folio"],
        )?);

    let runtime_lock_keys = check_in_lock_keys_from_payload(&hash_payload)?;
    let origin_key = format!("{}:{}", ctx.command_name, ctx.idempotency_key);
    let req_for_service = req;
    let user_id_for_service = user_id;

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
                    let booking = check_in_tx(
                        tx,
                        &req_for_service,
                        user_id_for_service.as_deref(),
                        Some(origin_key.as_str()),
                    )
                    .await
                    .map_err(map_check_in_command_error)?;
                    serde_json::to_value(&booking).map_err(system_error)
                })
            },
        )
        .await
}

#[allow(dead_code)]
struct CheckoutSettlementComputation {
    room_id: String,
    original_nights: i32,
    actual_nights: i32,
    settled_nights: i32,
    original_total: MoneyVnd,
    recommended_total: MoneyVnd,
    already_paid: MoneyVnd,
    explanation: String,
    reporting_checkout: String,
    /// The booking's `pricing_snapshot` column as it stood before checkout —
    /// carried through so `check_out_tx` can merge `checkout_settlement`
    /// into it instead of overwriting keys a room change already wrote
    /// (`room_stays`).
    pricing_snapshot: Option<String>,
}

fn settlement_mode_label(mode: CheckoutSettlementMode) -> &'static str {
    match mode {
        CheckoutSettlementMode::ActualNights => "Thanh toán theo số đêm thực tế",
        CheckoutSettlementMode::Hourly => "Thanh toán theo giờ",
        CheckoutSettlementMode::BookedNights => "Thanh toán theo số đêm đã đặt",
    }
}

fn actual_nights_for_checkout(
    check_in_at: &str,
    actual_checkout: DateTime<Local>,
) -> BookingResult<i32> {
    let check_in_date = parse_booking_datetime(check_in_at)?.date_naive();
    let checkout_date = actual_checkout.date_naive();

    Ok((checkout_date - check_in_date).num_days().max(1) as i32)
}

fn settlement_boundary_for(check_in_at: &str, settled_nights: i32) -> BookingResult<String> {
    let check_in_dt = parse_booking_datetime(check_in_at)?;
    Ok((check_in_dt + Duration::days(settled_nights as i64)).to_rfc3339())
}

fn reporting_checkout_for(
    check_in_at: &str,
    settled_nights: i32,
    actual_checkout: DateTime<Local>,
    settlement_mode: CheckoutSettlementMode,
) -> BookingResult<String> {
    let check_in_dt = parse_booking_datetime(check_in_at)?;
    let check_in_date = check_in_dt.date_naive();

    let report_date = match settlement_mode {
        CheckoutSettlementMode::BookedNights => {
            check_in_date + Duration::days(settled_nights as i64)
        }
        CheckoutSettlementMode::ActualNights | CheckoutSettlementMode::Hourly => {
            let checkout_date = actual_checkout.date_naive();
            if checkout_date <= check_in_date {
                check_in_date + Duration::days(1)
            } else {
                checkout_date
            }
        }
    };

    Ok(report_date.format("%Y-%m-%d").to_string())
}

fn settlement_explanation(
    settlement_mode: CheckoutSettlementMode,
    settled_nights: i32,
    recommended_total: MoneyVnd,
) -> String {
    match settlement_mode {
        CheckoutSettlementMode::Hourly => {
            "Thanh toán theo giờ: nhập tay số tiền quyết toán".to_string()
        }
        CheckoutSettlementMode::ActualNights | CheckoutSettlementMode::BookedNights => {
            let nightly_rate = if settled_nights > 0 {
                recommended_total as f64 / settled_nights as f64
            } else {
                recommended_total as f64
            };

            format!(
                "{}: {} đêm x {:.0}đ",
                settlement_mode_label(settlement_mode),
                settled_nights,
                nightly_rate
            )
        }
    }
}

/// Builds the *inner* `checkout_settlement` object — the shape
/// `revenue_queries.rs` reads via `json_extract(pricing_snapshot,
/// '$.checkout_settlement.mode')` and `'...reporting_checkout')`. Callers
/// merge this under the `checkout_settlement` key with `merge_pricing_snapshot`
/// rather than writing it as the whole `pricing_snapshot` column, so a
/// `room_stays` key written earlier by a room change survives checkout.
#[allow(clippy::too_many_arguments)]
fn checkout_settlement_value(
    settlement_mode: CheckoutSettlementMode,
    original_nights: i32,
    actual_nights: i32,
    settled_nights: i32,
    reporting_checkout: &str,
    original_total: MoneyVnd,
    settled_total: MoneyVnd,
    manual_override: bool,
) -> serde_json::Value {
    serde_json::json!({
        "mode": settlement_mode,
        "label": settlement_mode_label(settlement_mode),
        "reporting_checkout": reporting_checkout,
        "original_nights": original_nights,
        "actual_nights": actual_nights,
        "settled_nights": settled_nights,
        "original_total": original_total,
        "settled_total": settled_total,
        "manual_override": manual_override,
    })
}

/// Caps the cumulative night count in `rows` to `settled_nights`, walking the
/// segments in the date order `room_calendar_stays_tx` returns them: each
/// segment keeps up to its own count, the next one gets whatever budget is
/// left, and any segment past the point the budget hits zero is dropped
/// entirely — the guest never reached it under the nights actually settled.
///
/// This is only correct because `rows` are *occupancy segments in date order*,
/// not one row per room: a guest who moves A → B → A gets three segments, so
/// the budget is spent on the nights actually slept first. Group by room
/// instead and the earliest-dated A row swallows the whole budget while B
/// silently disappears from the invoice.
fn truncate_room_stays_to_settled_nights(
    rows: &[RoomStayRow],
    settled_nights: i32,
) -> Vec<RoomStayRow> {
    let mut remaining = i64::from(settled_nights.max(0));
    let mut truncated = Vec::new();

    for row in rows {
        if remaining <= 0 {
            break;
        }
        let nights_here = row.nights.min(remaining);
        truncated.push(RoomStayRow {
            room_id: row.room_id.clone(),
            room_name: row.room_name.clone(),
            nights: nights_here,
            first_night: row.first_night.clone(),
        });
        remaining -= nights_here;
    }

    truncated
}

async fn preview_checkout_settlement_tx(
    tx: &mut Transaction<'_, Sqlite>,
    req: &CheckoutSettlementPreviewRequest,
    now: DateTime<Local>,
) -> BookingResult<CheckoutSettlementComputation> {
    let booking = sqlx::query(
        "SELECT room_id, check_in_at, nights, total_price, paid_amount,
                COALESCE(pricing_type, 'nightly') AS pricing_type, guests, status,
                pricing_snapshot
         FROM bookings WHERE id = ?",
    )
    .bind(&req.booking_id)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or_else(|| {
        BookingError::not_found(format!(
            "Không tìm thấy booking đang active {}",
            req.booking_id
        ))
    })?;

    let booking_status: String = booking.get("status");
    if booking_status != status::booking::ACTIVE {
        return Err(invalid_state_transition(format!(
            "booking {} is no longer active",
            req.booking_id
        )));
    }

    let room_id: String = booking.get("room_id");
    let check_in_at: String = booking.get("check_in_at");
    let pricing_type: String = booking.get("pricing_type");
    let original_nights: i32 = booking.get("nights");
    let original_total = read_money_vnd_or_zero(&booking, "total_price");
    let guests: Option<i32> = booking.get("guests");
    let already_paid = read_money_vnd_or_zero(&booking, "paid_amount");
    let pricing_snapshot: Option<String> = booking.get("pricing_snapshot");
    let actual_nights = actual_nights_for_checkout(&check_in_at, now)?;

    let (settled_nights, recommended_total) = match req.settlement_mode {
        CheckoutSettlementMode::ActualNights => {
            let settled_nights = actual_nights.max(1);
            let settlement_boundary = settlement_boundary_for(&check_in_at, settled_nights)?;
            let pricing = calculate_stay_price_tx(
                tx,
                &room_id,
                &check_in_at,
                &settlement_boundary,
                &pricing_type,
                guests,
            )
            .await?;
            (settled_nights, pricing.total)
        }
        CheckoutSettlementMode::BookedNights => (original_nights.max(1), original_total),
        CheckoutSettlementMode::Hourly => (1, original_total),
    };

    let explanation =
        settlement_explanation(req.settlement_mode, settled_nights, recommended_total);
    let reporting_checkout =
        reporting_checkout_for(&check_in_at, settled_nights, now, req.settlement_mode)?;

    Ok(CheckoutSettlementComputation {
        room_id,
        original_nights,
        actual_nights,
        settled_nights,
        original_total,
        recommended_total,
        already_paid,
        explanation,
        reporting_checkout,
        pricing_snapshot,
    })
}

#[allow(dead_code)]
pub async fn preview_checkout_settlement(
    pool: &Pool<Sqlite>,
    req: CheckoutSettlementPreviewRequest,
) -> BookingResult<CheckoutSettlementPreview> {
    preview_checkout_settlement_at(pool, req, Local::now()).await
}

#[allow(dead_code)]
pub async fn preview_checkout_settlement_at(
    pool: &Pool<Sqlite>,
    req: CheckoutSettlementPreviewRequest,
    now: DateTime<Local>,
) -> BookingResult<CheckoutSettlementPreview> {
    let mut tx = begin_tx(pool).await?;
    let settlement = preview_checkout_settlement_tx(&mut tx, &req, now).await?;

    tx.rollback().await.map_err(BookingError::from)?;
    Ok(CheckoutSettlementPreview {
        settlement_mode: req.settlement_mode,
        settled_nights: settlement.settled_nights,
        recommended_total: settlement.recommended_total,
        explanation: settlement.explanation,
    })
}

#[cfg(test)]
pub async fn check_out(pool: &Pool<Sqlite>, req: CheckOutRequest) -> BookingResult<()> {
    check_out_at(pool, req, Local::now()).await
}

#[cfg(test)]
pub async fn check_out_at(
    pool: &Pool<Sqlite>,
    req: CheckOutRequest,
    now: DateTime<Local>,
) -> BookingResult<()> {
    validate_non_negative_booking_money(req.final_total, "final_total")?;
    let room_id = lookup_booking_room_id(pool, &req.booking_id).await?;
    let _lock_guard = crate::aggregate_locks::global_manager()
        .acquire([
            crate::aggregate_locks::booking_key(&req.booking_id)
                .map_err(|error| BookingError::validation(error.message))?,
            crate::aggregate_locks::room_key(&room_id)
                .map_err(|error| BookingError::validation(error.message))?,
            crate::aggregate_locks::folio_key(&req.booking_id)
                .map_err(|error| BookingError::validation(error.message))?,
        ])
        .await
        .map_err(|error| BookingError::validation(error.message))?;

    let mut tx = begin_immediate_tx(pool)
        .await
        .map_err(mark_write_db_error)?;

    check_out_tx(&mut tx, req, now, &room_id, None).await?;

    tx.commit()
        .await
        .map_err(BookingError::from)
        .map_err(mark_write_db_error)?;

    Ok(())
}

async fn check_out_tx(
    tx: &mut Transaction<'_, Sqlite>,
    req: CheckOutRequest,
    now: DateTime<Local>,
    locked_room_id: &str,
    origin_key: Option<String>,
) -> BookingResult<CheckOutResponse> {
    let final_total = validate_non_negative_booking_money(req.final_total, "final_total")?;
    let preview_req = CheckoutSettlementPreviewRequest {
        booking_id: req.booking_id.clone(),
        settlement_mode: req.settlement_mode,
    };
    let settlement = preview_checkout_settlement_tx(tx, &preview_req, now).await?;
    ensure_locked_room_matches_booking(
        locked_room_id,
        &settlement.room_id,
        format!("booking {} changed rooms before checkout", req.booking_id),
    )?;
    let actual_checkout = now.to_rfc3339();

    if settlement.already_paid > final_total {
        return Err(BookingError::validation(
            "Overpaid booking requires refund handling before checkout".to_string(),
        ));
    }

    let charge_delta = final_total - settlement.original_total;
    if charge_delta != 0 {
        let adjustment_note = if charge_delta < 0 {
            "Điều chỉnh giảm tiền phòng khi quyết toán check-out"
        } else {
            "Điều chỉnh tăng tiền phòng khi quyết toán check-out"
        };

        if let Some(origin_key) = origin_key.as_deref() {
            let origin = OriginSideEffect::new(origin_key, 0)?;
            record_charge_with_origin_tx(
                tx,
                &req.booking_id,
                charge_delta,
                adjustment_note,
                actual_checkout.clone(),
                &origin,
            )
            .await
            .map_err(mark_write_db_error)?;
        } else {
            record_charge_tx(
                tx,
                &req.booking_id,
                charge_delta,
                adjustment_note,
                actual_checkout.clone(),
            )
            .await
            .map_err(mark_write_db_error)?;
        }
    }

    let payment_delta = final_total - settlement.already_paid;
    if payment_delta > 0 {
        if let Some(origin_key) = origin_key.as_deref() {
            let origin = OriginSideEffect::new(origin_key, 1)?;
            record_payment_with_origin_tx(
                tx,
                &req.booking_id,
                payment_delta,
                "Thanh toán khi check-out",
                &origin,
            )
            .await
            .map_err(mark_write_db_error)?;
        } else {
            record_payment_tx(
                tx,
                &req.booking_id,
                payment_delta,
                "Thanh toán khi check-out",
            )
            .await
            .map_err(mark_write_db_error)?;
        }
    }

    let manual_override = final_total != settlement.recommended_total;
    let settlement_value = checkout_settlement_value(
        req.settlement_mode,
        settlement.original_nights,
        settlement.actual_nights,
        settlement.settled_nights,
        &settlement.reporting_checkout,
        settlement.original_total,
        final_total,
        manual_override,
    );
    // Merge, don't overwrite: a room change earlier in the stay may have
    // already written `room_stays` into this same column
    // (room_change.rs::change_room_tx), and checkout must not erase it.
    let pricing_snapshot = merge_pricing_snapshot(
        settlement.pricing_snapshot.as_deref(),
        "checkout_settlement",
        settlement_value,
    );

    // Re-derive `room_stays` from `room_calendar` — the live truth — before
    // the DELETE below wipes it. The snapshot `change_room_tx` wrote at move
    // time assumed the guest would stay every remaining *booked* night;
    // settlement may charge fewer than that (early checkout), so truncate
    // the per-room split down to `settled_nights`, walking rooms in
    // occupancy order and capping the running total. Reuses the same
    // `merge_pricing_snapshot` helper as `checkout_settlement` above — no
    // second snapshot writer.
    let room_stays_from_calendar = room_calendar_stays_tx(tx, &req.booking_id).await?;
    let truncated_room_stays =
        truncate_room_stays_to_settled_nights(&room_stays_from_calendar, settlement.settled_nights);
    let pricing_snapshot = merge_pricing_snapshot(
        Some(&pricing_snapshot),
        "room_stays",
        room_stays_to_json(&truncated_room_stays),
    );

    let result = sqlx::query(
        "UPDATE bookings
         SET status = ?, actual_checkout = ?, nights = ?, total_price = ?, pricing_snapshot = ?
         WHERE id = ? AND status = ?",
    )
    .bind(status::booking::CHECKED_OUT)
    .bind(&actual_checkout)
    .bind(settlement.settled_nights)
    .bind(final_total)
    .bind(pricing_snapshot)
    .bind(&req.booking_id)
    .bind(status::booking::ACTIVE)
    .execute(&mut **tx)
    .await
    .map_err(BookingError::from)
    .map_err(mark_write_db_error)?;
    ensure_one_row_affected(
        result,
        format!("booking {} is no longer active", req.booking_id),
    )?;

    let result = sqlx::query("UPDATE rooms SET status = ? WHERE id = ? AND status = ?")
        .bind(status::room::VACANT)
        .bind(&settlement.room_id)
        .bind(status::room::OCCUPIED)
        .execute(&mut **tx)
        .await
        .map_err(BookingError::from)
        .map_err(mark_write_db_error)?;
    ensure_one_row_affected(
        result,
        format!("room {} is no longer occupied", settlement.room_id),
    )?;

    sqlx::query("DELETE FROM room_calendar WHERE booking_id = ?")
        .bind(&req.booking_id)
        .execute(&mut **tx)
        .await
        .map_err(BookingError::from)
        .map_err(mark_write_db_error)?;

    Ok(CheckOutResponse {
        ok: true,
        booking_id: req.booking_id,
        room_id: settlement.room_id,
        actual_checkout,
        final_total,
    })
}

pub async fn check_out_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    req: CheckOutRequest,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    validate_non_negative_booking_money(req.final_total, "final_total").map_err(|error| {
        map_check_out_command_error(error).with_request_id(ctx.request_id.clone())
    })?;

    let hash_payload = build_check_out_hash_payload(&req);
    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("stay.check_out.v1")),
        ("booking_present", json!(true)),
        ("settlement_mode", json!(req.settlement_mode)),
        ("final_total_vnd", json!(req.final_total)),
    ])?;
    let summary = CommandLedgerSummary::new("Check out")?.with_aggregate_ref(
        "booking",
        "booking",
        None::<String>,
    )?;
    let request = WriteCommandRequest::new_sanitized(hash_payload.clone(), ledger_intent, summary)?
        .with_primary_aggregate_key(format!("booking:{}", req.booking_id))
        .with_lock_key_deriver(check_out_initial_lock_keys_from_payload)
        .with_success_summary(CommandLedgerResultSummary::success("Checked out")?)
        .with_outbox_event(OutboxEventSpec::new(
            "booking.checked_out",
            OutboxAggregateKeySource::response_field("booking", "booking_id"),
            &["bookings", "rooms", "folio"],
        )?);

    let pool_for_lookup = pool.clone();
    let booking_id_for_lookup = req.booking_id.clone();
    let origin_key = stay_command_origin_key(ctx);

    WriteCommandExecutor::new(pool.clone())
        .execute_with_resolved_guard(
            ctx,
            request,
            move || async move {
                let (guard, room_id) = lock_booking_and_read_room(
                    &pool_for_lookup,
                    &booking_id_for_lookup,
                    FolioLock::Include,
                    map_check_out_command_error,
                )
                .await?;
                let guard = guard
                    .acquire_next(
                        crate::aggregate_locks::global_manager(),
                        [crate::aggregate_locks::room_key(&room_id)?],
                    )
                    .await?;
                let lock_keys = guard.keys().to_vec();

                Ok(ResolvedWriteCommandGuard::new((guard, room_id), lock_keys))
            },
            move |tx, resolved| {
                Box::pin(async move {
                    let (_guard, locked_room_id) = resolved;
                    let response =
                        check_out_tx(tx, req, Local::now(), &locked_room_id, Some(origin_key))
                            .await
                            .map_err(map_check_out_command_error)?;
                    serde_json::to_value(&response).map_err(system_error)
                })
            },
        )
        .await
}

async fn extend_stay_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    locked_room_id: &str,
    origin_key: Option<String>,
) -> BookingResult<Booking> {
    let booking = sqlx::query(
        "SELECT room_id, nights, total_price, expected_checkout, pricing_type, guests, status
         FROM bookings WHERE id = ?",
    )
    .bind(booking_id)
    .fetch_optional(&mut **tx)
    .await?;

    let booking = booking.ok_or_else(|| {
        BookingError::not_found(format!("Không tìm thấy booking đang active {}", booking_id))
    })?;

    let booking_status: String = booking.get("status");
    if booking_status != status::booking::ACTIVE {
        return Err(invalid_state_transition(format!(
            "booking {booking_id} is not active for extend-stay (status: {booking_status})"
        )));
    }

    let room_id: String = booking.get("room_id");
    ensure_locked_room_matches_booking(
        locked_room_id,
        &room_id,
        format!("booking {booking_id} changed rooms before extend-stay"),
    )?;
    let current_nights: i32 = booking.get("nights");
    let current_total = read_money_vnd_or_zero(&booking, "total_price");
    let old_expected_checkout: String = booking.get("expected_checkout");
    let pricing_type = booking
        .get::<Option<String>, _>("pricing_type")
        .unwrap_or_else(|| "nightly".to_string());
    let guests: Option<i32> = booking.get("guests");

    let old_expected = parse_booking_datetime(&old_expected_checkout)?;
    let new_expected = old_expected + Duration::days(1);
    let extension_date = old_expected.date_naive();

    let room_exists = sqlx::query_scalar::<_, String>("SELECT id FROM rooms WHERE id = ? LIMIT 1")
        .bind(&room_id)
        .fetch_optional(&mut **tx)
        .await?;
    if room_exists.is_none() {
        return Err(BookingError::not_found(format!(
            "Không tìm thấy phòng {}",
            room_id
        )));
    }

    let conflict = sqlx::query(
        "SELECT booking_id FROM room_calendar WHERE room_id = ? AND date = ? AND booking_id != ? LIMIT 1",
    )
    .bind(&room_id)
    .bind(extension_date.format("%Y-%m-%d").to_string())
    .bind(booking_id)
    .fetch_optional(&mut **tx)
    .await?;

    if conflict.is_some() {
        return Err(BookingError::conflict(format!(
            "Phòng {} đã có lịch cho ngày {}",
            room_id,
            extension_date.format("%Y-%m-%d")
        )));
    }

    let incremental_pricing = calculate_stay_price_tx(
        tx,
        &room_id,
        &old_expected_checkout,
        &new_expected.to_rfc3339(),
        &pricing_type,
        guests,
    )
    .await?;
    let incremental_total = incremental_pricing.total;

    let new_total = current_total
        .checked_add(incremental_total)
        .ok_or_else(|| BookingError::validation("total_price overflowed".to_string()))?;
    let new_checkout = new_expected.to_rfc3339();

    // `scheduled_checkout` phải đi theo `expected_checkout`: timeline vẽ thanh
    // booking bằng `scheduled_checkout || expected_checkout`, nên bỏ quên cột này
    // là khách đặt trước gia hạn bao nhiêu lần thanh vẫn đứng ở ngày trả cũ.
    // CASE giữ NULL cho khách walk-in — họ không có ngày đặt trước để dời.
    let new_scheduled_checkout = new_expected.date_naive().format("%Y-%m-%d").to_string();

    let result = sqlx::query(
        "UPDATE bookings
         SET nights = ?, total_price = ?, expected_checkout = ?,
             scheduled_checkout = CASE
                 WHEN scheduled_checkout IS NULL THEN NULL ELSE ?
             END
         WHERE id = ? AND status = ? AND expected_checkout = ?",
    )
    .bind(current_nights + 1)
    .bind(new_total)
    .bind(&new_checkout)
    .bind(&new_scheduled_checkout)
    .bind(booking_id)
    .bind(status::booking::ACTIVE)
    .bind(&old_expected_checkout)
    .execute(&mut **tx)
    .await
    .map_err(BookingError::from)
    .map_err(mark_write_db_error)?;
    ensure_one_row_affected(
        result,
        format!("booking {booking_id} changed before extend-stay"),
    )?;

    sqlx::query(
        "INSERT INTO room_calendar (room_id, date, booking_id, status)
         VALUES (?, ?, ?, ?)",
    )
    .bind(&room_id)
    .bind(extension_date.format("%Y-%m-%d").to_string())
    .bind(booking_id)
    .bind(status::calendar::OCCUPIED)
    .execute(&mut **tx)
    .await
    .map_err(|error| map_room_calendar_insert_error(error, extension_date))
    .map_err(mark_write_db_error)?;

    if let Some(origin_key) = origin_key {
        let origin = OriginSideEffect::new(origin_key, 0)?;
        record_charge_with_origin_tx(
            tx,
            booking_id,
            incremental_total,
            "Extended stay +1 night",
            Local::now().to_rfc3339(),
            &origin,
        )
        .await
        .map_err(mark_write_db_error)?;
    } else {
        record_charge_tx(
            tx,
            booking_id,
            incremental_total,
            "Extended stay +1 night",
            Local::now().to_rfc3339(),
        )
        .await
        .map_err(mark_write_db_error)?;
    }

    fetch_booking_tx(tx, booking_id).await
}

pub async fn extend_stay_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    booking_id: &str,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let hash_payload = build_extend_stay_hash_payload(booking_id);
    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("stay.extend.v1")),
        ("booking_present", json!(true)),
        ("operation", json!("add_one_night")),
    ])?;
    let summary = CommandLedgerSummary::new("Extend stay")?.with_aggregate_ref(
        "booking",
        "booking",
        None::<String>,
    )?;
    let request = WriteCommandRequest::new_sanitized(hash_payload, ledger_intent, summary)?
        .with_primary_aggregate_key(format!("booking:{booking_id}"))
        .with_lock_key_deriver(extend_stay_initial_lock_keys_from_payload)
        .with_success_summary(CommandLedgerResultSummary::success("Stay extended")?)
        .with_outbox_event(OutboxEventSpec::new(
            "booking.stay_extended",
            OutboxAggregateKeySource::response_field("booking", "id"),
            &["bookings", "rooms", "folio"],
        )?);

    let pool_for_lookup = pool.clone();
    let booking_id_for_lookup = booking_id.to_string();
    let booking_id_for_service = booking_id.to_string();
    let origin_key = stay_command_origin_key(ctx);

    WriteCommandExecutor::new(pool.clone())
        .execute_with_resolved_guard(
            ctx,
            request,
            move || async move {
                let (guard, room_id) = lock_booking_and_read_room(
                    &pool_for_lookup,
                    &booking_id_for_lookup,
                    FolioLock::Include,
                    map_extend_stay_command_error,
                )
                .await?;
                let guard = guard
                    .acquire_next(
                        crate::aggregate_locks::global_manager(),
                        [crate::aggregate_locks::room_key(&room_id)?],
                    )
                    .await?;
                let lock_keys = guard.keys().to_vec();

                Ok(ResolvedWriteCommandGuard::new((guard, room_id), lock_keys))
            },
            move |tx, resolved| {
                Box::pin(async move {
                    let (_guard, locked_room_id) = resolved;
                    let booking = extend_stay_tx(
                        tx,
                        &booking_id_for_service,
                        &locked_room_id,
                        Some(origin_key),
                    )
                    .await
                    .map_err(map_extend_stay_command_error)?;
                    serde_json::to_value(&booking).map_err(system_error)
                })
            },
        )
        .await
}

pub async fn shorten_stay_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    booking_id: &str,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    // Đọc `expected_checkout` NGOÀI transaction, trước bất cứ bước khoá/claim
    // nào — đây là giá trị "trước khi khoá" mà `shorten_stay_tx` sẽ dùng để
    // chốt optimistic-concurrency guard ở UPDATE cuối cùng. Xem chú thích tại
    // `lookup_booking_expected_checkout` và bên trong `shorten_stay_tx`.
    //
    // Đây KHÔNG mâu thuẫn với #206 ("đọc phòng sau khi đã khoá booking"). #206
    // cấm đọc `room_id` trước khoá vì kết quả cũ khiến ta khoá NHẦM phòng rồi
    // đi tiếp — hỏng theo kiểu mở toang. Còn giá trị dưới đây cố ý được phép cũ:
    // nó là ảnh chụp "thứ lễ tân đang nhìn thấy", và cũ thì `AND
    // expected_checkout = ?` khớp 0 dòng nên lệnh dừng lại — hỏng theo kiểu đóng
    // chặt. Dời nó xuống sau pha 1 (hoặc vào trong transaction) là giết đúng cái
    // guard đó: khi ấy nó chỉ đọc lại chính giá trị mình vừa khoá.
    let locked_expected_checkout = lookup_booking_expected_checkout(pool, booking_id)
        .await
        .map_err(map_extend_stay_command_error)?;

    let hash_payload = build_shorten_stay_hash_payload(booking_id);
    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("stay.shorten.v1")),
        ("booking_present", json!(true)),
        ("operation", json!("remove_one_night")),
    ])?;
    let summary = CommandLedgerSummary::new("Shorten stay")?.with_aggregate_ref(
        "booking",
        "booking",
        None::<String>,
    )?;
    let request = WriteCommandRequest::new_sanitized(hash_payload, ledger_intent, summary)?
        .with_primary_aggregate_key(format!("booking:{booking_id}"))
        .with_lock_key_deriver(shorten_stay_initial_lock_keys_from_payload)
        .with_success_summary(CommandLedgerResultSummary::success("Stay shortened")?)
        .with_outbox_event(OutboxEventSpec::new(
            "booking.stay_shortened",
            OutboxAggregateKeySource::response_field("booking", "id"),
            &["bookings", "rooms", "folio"],
        )?);

    let pool_for_lookup = pool.clone();
    let booking_id_for_lookup = booking_id.to_string();
    let booking_id_for_service = booking_id.to_string();
    let origin_key = stay_command_origin_key(ctx);

    WriteCommandExecutor::new(pool.clone())
        .execute_with_resolved_guard(
            ctx,
            request,
            move || async move {
                let (guard, room_id) = lock_booking_and_read_room(
                    &pool_for_lookup,
                    &booking_id_for_lookup,
                    FolioLock::Include,
                    map_extend_stay_command_error,
                )
                .await?;
                let guard = guard
                    .acquire_next(
                        crate::aggregate_locks::global_manager(),
                        [crate::aggregate_locks::room_key(&room_id)?],
                    )
                    .await?;
                let lock_keys = guard.keys().to_vec();

                Ok(ResolvedWriteCommandGuard::new((guard, room_id), lock_keys))
            },
            move |tx, resolved| {
                Box::pin(async move {
                    let (_guard, locked_room_id) = resolved;
                    let booking = shorten_stay_tx(
                        tx,
                        &booking_id_for_service,
                        &locked_room_id,
                        &locked_expected_checkout,
                        Some(origin_key),
                    )
                    .await
                    .map_err(map_extend_stay_command_error)?;
                    serde_json::to_value(&booking).map_err(system_error)
                })
            },
        )
        .await
}

pub async fn set_booking_rate_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    booking_id: &str,
    rate_per_night: MoneyVnd,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    // Đọc `total_price` NGOÀI transaction, trước bất cứ bước khoá/claim nào —
    // giá trị "trước khi khoá" mà `set_booking_rate_tx` dùng để chốt
    // optimistic-concurrency guard ở UPDATE cuối cùng. Cùng kỹ thuật, và cùng
    // lý do vì sao nó không phạm vào #206, với `shorten_stay_idempotent` phía
    // trên.
    let locked_total_price = lookup_booking_total_price(pool, booking_id)
        .await
        .map_err(map_extend_stay_command_error)?;

    let hash_payload = build_set_booking_rate_hash_payload(booking_id, rate_per_night);
    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("stay.set_rate.v1")),
        ("booking_present", json!(true)),
        ("operation", json!("set_booking_rate")),
    ])?;
    let summary = CommandLedgerSummary::new("Set booking rate")?.with_aggregate_ref(
        "booking",
        "booking",
        None::<String>,
    )?;
    let request = WriteCommandRequest::new_sanitized(hash_payload, ledger_intent, summary)?
        .with_primary_aggregate_key(format!("booking:{booking_id}"))
        .with_lock_key_deriver(set_booking_rate_initial_lock_keys_from_payload)
        .with_success_summary(CommandLedgerResultSummary::success("Booking rate set")?)
        .with_outbox_event(OutboxEventSpec::new(
            "booking.rate_changed",
            OutboxAggregateKeySource::response_field("booking", "id"),
            &["bookings", "folio"],
        )?);

    let booking_id_for_guard = booking_id.to_string();
    let booking_id_for_service = booking_id.to_string();
    let origin_key = stay_command_origin_key(ctx);

    WriteCommandExecutor::new(pool.clone())
        .execute_with_resolved_guard(
            ctx,
            request,
            // KHÔNG lấy khoá phòng, khác hẳn ba lệnh stay còn lại.
            //
            // `set_booking_rate_tx` không đọc và không ghi một mẩu trạng thái
            // phòng nào: nó sửa `bookings.total_price`/`rate_overridden_at` rồi
            // ghi một dòng chênh lệch vào folio. Khoá `booking:` đã đủ để xếp
            // hàng với check_out / extend_stay / shorten_stay / change_room —
            // cả bốn đều lấy `booking:` ở pha 1 kể từ #206 — nên khoá phòng ở
            // đây không bảo vệ thêm bất cứ bất biến nào, mà chỉ chặn oan những
            // lệnh thật sự thuộc về phòng (ví dụ `update_housekeeping`) và bắt
            // ta đọc thêm một vòng `room_id` không ai dùng.
            //
            // Bỏ bớt khoá không tạo nguy cơ kẹt: thứ tự toàn cục là theo HẠNG
            // (group < booking < folio < room), và cầm một tập con của một thứ
            // tự toàn phần thì luôn an toàn. Bộ khoá này cũng khớp đúng thứ mà
            // `set_booking_rate_initial_lock_keys_from_payload` đã khai lúc
            // claim, nên `lock_keys_json` không đổi giữa hai bước.
            move || async move {
                let guard = crate::aggregate_locks::global_manager()
                    .acquire([
                        crate::aggregate_locks::booking_key(&booking_id_for_guard)?,
                        crate::aggregate_locks::folio_key(&booking_id_for_guard)?,
                    ])
                    .await?;
                let lock_keys = guard.keys().to_vec();

                Ok(ResolvedWriteCommandGuard::new(guard, lock_keys))
            },
            move |tx, _guard| {
                Box::pin(async move {
                    let booking = set_booking_rate_tx(
                        tx,
                        &booking_id_for_service,
                        rate_per_night,
                        locked_total_price,
                        Some(origin_key),
                    )
                    .await
                    .map_err(map_extend_stay_command_error)?;
                    serde_json::to_value(&booking).map_err(system_error)
                })
            },
        )
        .await
}

#[cfg(test)]
pub async fn extend_stay(pool: &Pool<Sqlite>, booking_id: &str) -> BookingResult<Booking> {
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

    let _booking = extend_stay_tx(&mut tx, booking_id, &locked_room_id, None).await?;

    tx.commit().await.map_err(BookingError::from)?;

    fetch_booking(
        pool,
        booking_id,
        format!("Không tìm thấy booking {}", booking_id),
        read_money_vnd_or_zero,
    )
    .await
}

async fn shorten_stay_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    locked_room_id: &str,
    locked_expected_checkout: &str,
    origin_key: Option<String>,
) -> BookingResult<Booking> {
    let booking = sqlx::query(
        "SELECT room_id, nights, total_price, paid_amount, check_in_at, expected_checkout, status
         FROM bookings WHERE id = ?",
    )
    .bind(booking_id)
    .fetch_optional(&mut **tx)
    .await?;

    let booking = booking.ok_or_else(|| {
        BookingError::not_found(format!("Không tìm thấy booking đang active {}", booking_id))
    })?;

    let booking_status: String = booking.get("status");
    if booking_status != status::booking::ACTIVE {
        return Err(invalid_state_transition(
            "Booking không còn đang lưu trú nên không thể rút đêm — vui lòng tải lại trang",
        ));
    }

    let room_id: String = booking.get("room_id");
    ensure_locked_room_matches_booking(
        locked_room_id,
        &room_id,
        "Booking đã đổi phòng trước khi rút đêm — vui lòng tải lại trang và thử lại",
    )?;
    let current_nights: i32 = booking.get("nights");
    let current_total = read_money_vnd_or_zero(&booking, "total_price");
    let paid_amount = read_money_vnd_or_zero(&booking, "paid_amount");
    let check_in_at: String = booking.get("check_in_at");
    // `old_expected_checkout` (đọc lại TRONG transaction này) dùng cho phép tính
    // ngày tháng bên dưới — luôn phản ánh đúng dữ liệu hiện tại nên phép tính
    // luôn đúng. `locked_expected_checkout` (tham số, đọc TRƯỚC khi mở
    // transaction) mới là giá trị đưa vào guard `AND expected_checkout = ?` của
    // UPDATE cuối hàm: nó chốt lại "cái tôi thấy lúc quyết định rút đêm", nên
    // nếu một actor khác đổi checkout của booking này giữa lúc đọc trước khoá
    // và lúc UPDATE thật sự chạy, UPDATE khớp 0 dòng thay vì âm thầm ghi đè lên
    // một checkout mà lễ tân chưa từng thấy.
    let old_expected_checkout: String = booking.get("expected_checkout");

    let old_expected = parse_booking_datetime(&old_expected_checkout)?;
    let check_in = parse_booking_datetime(&check_in_at)?;
    let new_expected = old_expected - Duration::days(1);
    let freed_date = new_expected.date_naive();

    // Kiểm tra quá khứ trước: một booking 1 đêm đã quá hạn (checkout đã ở
    // quá khứ) phải báo dùng Check-out, không phải "tối thiểu 1 đêm" — nếu
    // không đổi thứ tự, ca đó luôn rơi vào nhánh tối thiểu 1 đêm bên dưới
    // trước khi chạm tới đây, vì freed_date trùng ngày check-in.
    if freed_date < Local::now().date_naive() {
        return Err(BookingError::validation(
            "Ngày trả phòng mới sẽ rơi vào quá khứ — dùng Check-out để cho khách rời phòng"
                .to_string(),
        ));
    }

    if freed_date <= check_in.date_naive() {
        return Err(BookingError::validation(
            "Lưu trú tối thiểu 1 đêm".to_string(),
        ));
    }

    // Tiền hoàn lấy từ trung bình của CHÍNH booking này, không hỏi pricing engine.
    // Pricing engine trả giá hôm nay, còn đêm này đã thu theo giá lúc đặt; nâng giá
    // phòng giữa chừng sẽ khiến ta hoàn nhiều hơn số đã thu. Không có bản ghi giá
    // từng đêm để tra (pricing_snapshot chỉ ghi lúc check-out).
    //
    // Bất đối xứng có chủ đích với `extend_stay_tx` (phía trên): extend tính đêm
    // thêm theo giá HÔM NAY từ pricing engine, còn shorten hoàn theo giá TRUNG
    // BÌNH của chính booking. Hai công thức chỉ trùng nhau khi mọi đêm cùng giá.
    // Nếu extend diễn ra giữa một đợt tăng giá rồi bị shorten rút lại, khoản hoàn
    // sẽ thấp hơn khoản mới thu — đây là đánh đổi sản phẩm đã được chấp nhận,
    // không phải bug.
    //
    // Chia số nguyên luôn làm tròn về 0, nên khoản hoàn không bao giờ vượt quá
    // trung bình thật; phần dư (tối đa `nights - 1` VND trên toàn bộ vòng rút)
    // luôn ở lại phía khách sạn. Với booking giá không đều theo từng đêm, mức
    // trung bình này cũng có thể hoàn NHIỀU hơn giá thật của một đêm rẻ cụ thể —
    // cũng là đánh đổi sản phẩm đã chấp nhận, không phải lỗi.
    //
    // `bookings.nights` không có CHECK constraint ở schema, nên đây là một giá
    // trị DB chưa được kiểm chứng: nights = 0 sẽ panic chia cho 0, nights âm sẽ
    // vẫn "tính" ra được nhưng bịa tiền (removed_total âm khiến new_total tăng
    // thay vì giảm). Không đường đi nào trong code hôm nay có thể tạo ra giá trị
    // như vậy cho booking đang active, nhưng chặn tường minh ở đây để phòng thủ
    // theo chiều sâu (defence in depth), giống cách `recognized_room_revenue_amount_sql`
    // ở `queries/booking/revenue_queries.rs` bảo vệ cùng phép chia này.
    if current_nights <= 0 {
        return Err(BookingError::validation(format!(
            "Số đêm hiện tại của booking không hợp lệ ({current_nights}), không thể tính tiền hoàn"
        )));
    }
    let removed_total = current_total / i64::from(current_nights);

    let new_total = crate::pricing::checked_sub_money(current_total, removed_total, "total_price")
        .map_err(BookingError::validation)?;

    // Đảo ngược quyết định trước đó: rút đêm từng được phép đưa tổng tiền
    // xuống dưới số khách đã trả, với giả định lễ tân sẽ hoàn tiền mặt tại
    // quầy. Giả định đó sai — check_out_tx từ chối thẳng khi already_paid >
    // final_total, nên booking rơi vào trạng thái không có lối thoát nào cho
    // tới khi bị undo hoặc check-out ở mức tổng tiền chưa giảm (thổi phồng
    // doanh thu). Chặn ngay tại đây, trước UPDATE, để không tạo ra trạng thái
    // đó nữa.
    if new_total < paid_amount {
        return Err(BookingError::validation(format!(
            "Khách đã thanh toán {paid_amount}đ, cao hơn tổng tiền {new_total}đ sau khi rút đêm này — cần xử lý hoàn tiền cho khách trước khi rút đêm"
        )));
    }

    let new_checkout = new_expected.to_rfc3339();

    // Đối xứng với `extend_stay` ở trên: `scheduled_checkout` phải đi theo
    // `expected_checkout`, vì timeline vẽ thanh booking bằng
    // `scheduled_checkout || expected_checkout`. Bỏ quên cột này là lễ tân bấm
    // "−1 đêm", drawer đổi ngày, còn thanh trên lịch phòng đứng nguyên ở ngày
    // trả cũ — chủ khách sạn tưởng phòng còn bận thêm một đêm nên không bán.
    // CASE giữ NULL cho khách vãng lai — họ không có ngày đặt trước để dời.
    let new_scheduled_checkout = new_expected.date_naive().format("%Y-%m-%d").to_string();

    let result = sqlx::query(
        "UPDATE bookings
         SET nights = ?, total_price = ?, expected_checkout = ?,
             scheduled_checkout = CASE
                 WHEN scheduled_checkout IS NULL THEN NULL ELSE ?
             END
         WHERE id = ? AND status = ? AND expected_checkout = ?",
    )
    .bind(current_nights - 1)
    .bind(new_total)
    .bind(&new_checkout)
    .bind(&new_scheduled_checkout)
    .bind(booking_id)
    .bind(status::booking::ACTIVE)
    .bind(locked_expected_checkout)
    .execute(&mut **tx)
    .await
    .map_err(BookingError::from)
    .map_err(mark_write_db_error)?;
    ensure_one_row_affected(
        result,
        "Booking vừa được cập nhật bởi thao tác khác — vui lòng tải lại trang và thử lại",
    )?;

    let calendar_result =
        sqlx::query("DELETE FROM room_calendar WHERE room_id = ? AND date = ? AND booking_id = ?")
            .bind(&room_id)
            .bind(freed_date.format("%Y-%m-%d").to_string())
            .bind(booking_id)
            .execute(&mut **tx)
            .await
            .map_err(BookingError::from)
            .map_err(mark_write_db_error)?;
    ensure_one_row_affected(
        calendar_result,
        format!("lịch phòng của booking {booking_id} đã thay đổi trước khi rút đêm"),
    )?;

    let credit = 0i64.checked_sub(removed_total).ok_or_else(|| {
        BookingError::validation("Số tiền hoàn vượt giới hạn tính toán".to_string())
    })?;

    if let Some(origin_key) = origin_key {
        let origin = OriginSideEffect::new(origin_key, 0)?;
        record_charge_with_origin_tx(
            tx,
            booking_id,
            credit,
            "Shortened stay -1 night",
            Local::now().to_rfc3339(),
            &origin,
        )
        .await
        .map_err(mark_write_db_error)?;
    } else {
        record_charge_tx(
            tx,
            booking_id,
            credit,
            "Shortened stay -1 night",
            Local::now().to_rfc3339(),
        )
        .await
        .map_err(mark_write_db_error)?;
    }

    fetch_booking_tx(tx, booking_id).await
}

#[cfg(test)]
pub async fn shorten_stay(pool: &Pool<Sqlite>, booking_id: &str) -> BookingResult<Booking> {
    let locked_room_id = lookup_booking_room_id(pool, booking_id).await?;
    let locked_expected_checkout = lookup_booking_expected_checkout(pool, booking_id).await?;
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

    let _booking = shorten_stay_tx(
        &mut tx,
        booking_id,
        &locked_room_id,
        &locked_expected_checkout,
        None,
    )
    .await?;

    tx.commit().await.map_err(BookingError::from)?;

    fetch_booking(
        pool,
        booking_id,
        format!("Không tìm thấy booking {}", booking_id),
        read_money_vnd_or_zero,
    )
    .await
}

/// Trần chống gõ nhầm thừa số 0, không phải giới hạn nghiệp vụ.
pub const MAX_RATE_PER_NIGHT_VND: MoneyVnd = 100_000_000;

async fn set_booking_rate_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    rate_per_night: MoneyVnd,
    locked_total_price: MoneyVnd,
    origin_key: Option<String>,
) -> BookingResult<Booking> {
    if rate_per_night <= 0 || rate_per_night > MAX_RATE_PER_NIGHT_VND {
        return Err(BookingError::validation(
            "Giá mỗi đêm không hợp lệ".to_string(),
        ));
    }

    let booking =
        sqlx::query("SELECT nights, total_price, paid_amount, status FROM bookings WHERE id = ?")
            .bind(booking_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| {
                BookingError::not_found(format!(
                    "Không tìm thấy booking đang active {}",
                    booking_id
                ))
            })?;

    let booking_status: String = booking.get("status");
    if booking_status != status::booking::ACTIVE {
        return Err(invalid_state_transition(
            "Booking không còn đang lưu trú nên không thể đổi giá — vui lòng tải lại trang",
        ));
    }

    let nights: i32 = booking.get("nights");
    // `current_total` (đọc lại TRONG transaction này) dùng để tính `new_total`
    // và khoản charge chênh lệch bên dưới — luôn phản ánh đúng dữ liệu hiện
    // tại. `locked_total_price` (tham số, đọc TRƯỚC khi mở transaction) mới là
    // giá trị đưa vào guard `AND total_price = ?` của UPDATE cuối hàm — cùng lý
    // do với `locked_expected_checkout` ở `shorten_stay_tx`.
    let current_total = read_money_vnd_or_zero(&booking, "total_price");
    let paid_amount = read_money_vnd_or_zero(&booking, "paid_amount");

    // `bookings.nights` không có CHECK constraint ở schema, nên đây là một giá
    // trị DB chưa được kiểm chứng: nights = 0 sẽ khiến tổng tiền mới biến
    // thành 0 thay vì phản ánh giá mới thật, còn nights âm sẽ vẫn "nhân" ra
    // được nhưng lật dấu tổng tiền (giá dương x đêm âm = tổng âm). Không
    // đường đi nào trong code hôm nay có thể tạo ra giá trị như vậy cho
    // booking đang active, nhưng chặn tường minh ở đây để phòng thủ theo
    // chiều sâu (defence in depth), giống guard tương ứng ở `shorten_stay_tx`
    // phía trên.
    if nights <= 0 {
        return Err(BookingError::validation(format!(
            "Số đêm hiện tại của booking không hợp lệ ({nights}), không thể tính giá mới"
        )));
    }

    let new_total =
        crate::pricing::checked_mul_money(rate_per_night, i64::from(nights), "total_price")
            .map_err(BookingError::validation)?;

    // Đảo ngược quyết định trước đó: hạ giá từng được phép đưa tổng tiền
    // xuống dưới số khách đã trả, với giả định lễ tân sẽ hoàn tiền mặt tại
    // quầy. Giả định đó sai — check_out_tx từ chối thẳng khi already_paid >
    // final_total, nên booking rơi vào trạng thái không có lối thoát nào cho
    // tới khi bị undo hoặc check-out ở mức tổng tiền chưa giảm (thổi phồng
    // doanh thu). Chặn ngay tại đây, trước UPDATE, để không tạo ra trạng thái
    // đó nữa. Cùng một guard, cùng lý do, với `shorten_stay_tx` phía trên.
    if new_total < paid_amount {
        return Err(BookingError::validation(format!(
            "Khách đã thanh toán {paid_amount}đ, cao hơn tổng tiền mới {new_total}đ — cần xử lý hoàn tiền cho khách trước khi đổi giá"
        )));
    }

    let overridden_at = Local::now().to_rfc3339();
    let result = sqlx::query(
        "UPDATE bookings
         SET total_price = ?, rate_overridden_at = ?
         WHERE id = ? AND status = ? AND total_price = ?",
    )
    .bind(new_total)
    .bind(&overridden_at)
    .bind(booking_id)
    .bind(status::booking::ACTIVE)
    .bind(locked_total_price)
    .execute(&mut **tx)
    .await
    .map_err(BookingError::from)
    .map_err(mark_write_db_error)?;
    ensure_one_row_affected(
        result,
        "Booking vừa được cập nhật bởi thao tác khác — vui lòng tải lại trang và thử lại",
    )?;

    let delta = crate::pricing::checked_sub_money(new_total, current_total, "rate_delta")
        .map_err(BookingError::validation)?;

    if delta != 0 {
        let old_rate = current_total / i64::from(nights);
        let note = format!("Đổi giá: {} → {}", old_rate, rate_per_night);

        if let Some(origin_key) = origin_key {
            let origin = OriginSideEffect::new(origin_key, 0)?;
            record_charge_with_origin_tx(
                tx,
                booking_id,
                delta,
                note,
                Local::now().to_rfc3339(),
                &origin,
            )
            .await
            .map_err(mark_write_db_error)?;
        } else {
            record_charge_tx(tx, booking_id, delta, note, Local::now().to_rfc3339())
                .await
                .map_err(mark_write_db_error)?;
        }
    }

    fetch_booking_tx(tx, booking_id).await
}

#[cfg(test)]
pub async fn set_booking_rate(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    rate_per_night: MoneyVnd,
) -> BookingResult<Booking> {
    let locked_total_price = lookup_booking_total_price(pool, booking_id).await?;
    let _lock_guard = crate::aggregate_locks::global_manager()
        .acquire([crate::aggregate_locks::booking_key(booking_id)
            .map_err(|error| BookingError::validation(error.message))?])
        .await
        .map_err(|error| BookingError::validation(error.message))?;

    let mut tx = begin_immediate_tx(pool).await?;
    let _booking = set_booking_rate_tx(
        &mut tx,
        booking_id,
        rate_per_night,
        locked_total_price,
        None,
    )
    .await?;
    tx.commit().await.map_err(BookingError::from)?;

    fetch_booking(
        pool,
        booking_id,
        format!("Không tìm thấy booking {}", booking_id),
        read_money_vnd_or_zero,
    )
    .await
}

pub const MAX_BOOKING_NOTES_LEN: usize = 2_000;

fn validate_and_trim_booking_notes(notes: Option<String>) -> BookingResult<Option<String>> {
    let trimmed = notes
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    if let Some(ref value) = trimmed {
        if value.chars().count() > MAX_BOOKING_NOTES_LEN {
            return Err(BookingError::validation(format!(
                "Ghi chú tối đa {} ký tự",
                MAX_BOOKING_NOTES_LEN
            )));
        }
    }

    Ok(trimmed)
}

async fn update_booking_notes_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    trimmed_notes: Option<&str>,
) -> BookingResult<Booking> {
    let result = sqlx::query("UPDATE bookings SET notes = ? WHERE id = ? AND status = ?")
        .bind(trimmed_notes)
        .bind(booking_id)
        .bind(status::booking::ACTIVE)
        .execute(&mut **tx)
        .await
        .map_err(BookingError::from)
        .map_err(mark_write_db_error)?;
    ensure_one_row_affected(
        result,
        format!("Không tìm thấy booking đang active {booking_id}"),
    )?;

    fetch_booking_tx(tx, booking_id).await
}

fn build_update_booking_notes_hash_payload(
    booking_id: &str,
    notes_present: bool,
) -> serde_json::Value {
    json!({
        "schema": "stay.update_notes.v1",
        "booking_id": booking_id,
        "notes_present": notes_present,
    })
}

fn update_booking_notes_lock_keys_from_payload(
    hash_payload: &serde_json::Value,
) -> CommandResult<Vec<String>> {
    let booking_id = hash_payload
        .get("booking_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| system_error("update-booking-notes lock payload missing booking_id"))?;
    Ok(vec![crate::aggregate_locks::booking_key(booking_id)?])
}

/// Ghi chú không đụng tiền và không đụng lịch phòng, nên không cần bộ máy
/// idempotency đầy đủ (không vào `write_manifest.rs`, không vào
/// `command_recovery.rs`, không cần idempotency key từ phía gọi) — theo tiền
/// lệ của `add_folio_line`. Vẫn cần một actor đã đăng nhập và một dòng trong
/// command ledger, vì ghi chú này in thẳng lên hoá đơn của khách: "Ai đã sửa
/// ghi chú này?" phải trả lời được mà không cần bảng log riêng.
/// `WriteCommandContext::new_internal` tự sinh idempotency key nội bộ — phía
/// gọi (Tauri command) không cần biết tới khái niệm idempotency key.
pub async fn update_booking_notes_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    booking_id: &str,
    notes: Option<String>,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let trimmed = validate_and_trim_booking_notes(notes).map_err(|error| {
        map_extend_stay_command_error(error).with_request_id(ctx.request_id.clone())
    })?;

    let notes_present = trimmed.is_some();
    let hash_payload = build_update_booking_notes_hash_payload(booking_id, notes_present);
    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("stay.update_notes.v1")),
        ("booking_present", json!(true)),
        ("notes_present", json!(notes_present)),
    ])?;
    let summary = CommandLedgerSummary::new("Update booking notes")?.with_aggregate_ref(
        "booking",
        "booking",
        None::<String>,
    )?;
    let request = WriteCommandRequest::new_sanitized(hash_payload, ledger_intent, summary)?
        .with_primary_aggregate_key(format!("booking:{booking_id}"))
        .with_lock_key_deriver(update_booking_notes_lock_keys_from_payload)
        .with_success_summary(CommandLedgerResultSummary::success(
            "Booking notes updated",
        )?);

    let booking_id_for_service = booking_id.to_string();

    WriteCommandExecutor::new(pool.clone())
        .execute_atomic(ctx, request, move |tx| {
            Box::pin(async move {
                let booking =
                    update_booking_notes_tx(tx, &booking_id_for_service, trimmed.as_deref())
                        .await
                        .map_err(map_extend_stay_command_error)?;
                serde_json::to_value(&booking).map_err(system_error)
            })
        })
        .await
}

#[cfg(test)]
pub async fn update_booking_notes(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    notes: Option<String>,
) -> BookingResult<Booking> {
    let trimmed = validate_and_trim_booking_notes(notes)?;

    let _lock_guard = crate::aggregate_locks::global_manager()
        .acquire([crate::aggregate_locks::booking_key(booking_id)
            .map_err(|error| BookingError::validation(error.message))?])
        .await
        .map_err(|error| BookingError::validation(error.message))?;

    let mut tx = begin_immediate_tx(pool).await?;

    let booking = update_booking_notes_tx(&mut tx, booking_id, trimmed.as_deref()).await?;

    tx.commit().await.map_err(BookingError::from)?;

    Ok(booking)
}

fn validate_check_in_request(req: &CheckInRequest) -> BookingResult<()> {
    if req.guests.is_empty() {
        return Err(BookingError::validation(
            "Phải có ít nhất 1 khách".to_string(),
        ));
    }
    if req.nights <= 0 {
        return Err(BookingError::validation(
            "Number of nights must be greater than 0".to_string(),
        ));
    }
    if let Some(paid_amount) = req.paid_amount {
        validate_non_negative_booking_money(paid_amount, "paid_amount")?;
    }

    // Chặn giá tay ngoài biên NGAY TẠI ĐÂY, trước khi bất cứ phép nhân nào có
    // cơ hội chạy trên một `rate` chưa kiểm biên — nếu không, `checked_mul_money`
    // có thể ăn một giá trị tràn số / không an toàn và trả thẳng ra người dùng
    // một thông báo TIẾNG ANH (qua command error mapper), hoặc — với giá âm —
    // để lọt xuống một guard khác đọc nhầm một tổng âm ra như một mức giá.
    // Cùng thứ tự với `set_booking_rate_tx`: biên trước, phép nhân sau.
    //
    // Guard thu-quá-tổng đã chuyển sang `check_in_tx`, sau khi `total_price`
    // được biết ở CẢ hai nhánh (giá tay lẫn giá engine) — ở đây (chưa mở
    // transaction, chưa gọi engine) chỉ biết được tổng khi có giá tay, nên
    // không thể chặn đường không-override, con đường mọi lượt check-in bình
    // thường đi qua.
    if let Some(rate) = req.rate_override_per_night {
        if rate <= 0 || rate > MAX_RATE_PER_NIGHT_VND {
            return Err(BookingError::validation(
                "Giá mỗi đêm không hợp lệ".to_string(),
            ));
        }
    }

    Ok(())
}

async fn insert_occupied_calendar_rows(
    tx: &mut Transaction<'_, Sqlite>,
    room_id: &str,
    booking_id: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> BookingResult<()> {
    insert_room_calendar_rows(
        tx,
        room_id,
        booking_id,
        start_date,
        end_date,
        status::calendar::OCCUPIED,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_locked_room_matches_booking, mark_write_db_error, validate_check_in_request,
    };
    use crate::domain::booking::BookingError;
    use crate::models::{CheckInRequest, CreateGuestRequest};

    fn check_in_request_with_rate(rate: crate::money::MoneyVnd) -> CheckInRequest {
        CheckInRequest {
            room_id: "room-x".to_string(),
            guests: vec![CreateGuestRequest {
                guest_type: None,
                full_name: "Khách".to_string(),
                doc_number: "DOC-X".to_string(),
                dob: None,
                gender: None,
                nationality: None,
                address: None,
                visa_expiry: None,
                scan_path: None,
                phone: None,
            }],
            nights: 1,
            source: None,
            notes: None,
            paid_amount: Some(100_000),
            pricing_type: None,
            rate_override_per_night: Some(rate),
        }
    }

    /// `validate_check_in_request` gọi thẳng, KHÔNG qua `check_in_tx` — vì
    /// `check_in_tx` giữ một bản sao (cố ý, cùng thông báo) của guard biên này
    /// làm lưới chặn tầng transaction; đi qua `check_in()`/`check_in_idempotent`
    /// sẽ khiến một hồi quy CHỈ ở `validate_check_in_request` bị bản sao đó che
    /// mất — cả hai đường đều trả về "Giá mỗi đêm không hợp lệ" nên bên ngoài
    /// không phân biệt được. Gọi trực tiếp hàm này là cách duy nhất kiểm được
    /// đúng phần vừa sửa (thứ tự biên-trước-nhân trong CHÍNH hàm này).
    #[test]
    fn validate_check_in_request_rejects_bad_rates_before_any_multiply() {
        // Đủ lớn để vượt biên an toàn số nguyên (giả sử biên bị gỡ, phép nhân
        // ở dưới sẽ ăn giá trị này và trả "total_price must be a safe integer
        // VND value" — English) — cùng lúc đủ lớn để vượt luôn MAX_RATE_PER_NIGHT_VND.
        let huge = 9_500_000_000_000_000_i64;
        // Âm: nhân với đêm dương vẫn ra một số "hợp lệ" (không tràn số), nên
        // nếu biên bị gỡ, request sẽ lọt qua phép nhân — bug thật đang chặn.
        let negative = -500_000_i64;

        for rate in [huge, negative] {
            let error = validate_check_in_request(&check_in_request_with_rate(rate)).unwrap_err();
            assert_eq!(
                error,
                BookingError::validation("Giá mỗi đêm không hợp lệ".to_string()),
                "giá {rate} phải bị chặn bằng thông báo biên tiếng Việt, nhận được: {error:?}"
            );
        }
    }

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
    }

    #[test]
    fn ensure_locked_room_matches_booking_rejects_stale_room_lock() {
        let result = ensure_locked_room_matches_booking(
            "room-old",
            "room-new",
            "booking booking-1 changed rooms before checkout",
        );

        assert!(
            matches!(result, Err(BookingError::Conflict(message)) if message.contains(crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION) && message.contains("booking booking-1 changed rooms before checkout"))
        );
    }
}
