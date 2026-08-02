//! Room change read-side: which rooms a booking could move to, and the price
//! difference for each.
//!
//! `bookings.room_id` holds the guest's CURRENT room; `room_calendar` (one row
//! per room per night) holds the real history. When a guest moves mid-stay,
//! nights already slept stay on the old room and only nights from today
//! forward move. So "remaining nights" is derived from `room_calendar` rows
//! with `date >= today` — not from "today until checkout" — which matters for
//! future reservations that start days from now.

use chrono::{Local, NaiveDate};
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
    models::{status, RoomChangeOption, RoomChangeOptions},
    outbox::{OutboxAggregateKeySource, OutboxEventSpec},
};

use super::{
    billing_service::{record_charge_tx, record_charge_with_origin_tx},
    pricing_service::calculate_stay_price_tx,
    stay_lifecycle::fetch_booking_tx,
    support::{
        begin_tx, ensure_one_row_affected, invalid_state_transition, lookup_booking_room_id,
        read_money_vnd_or_zero,
    },
};

// `change_room` (the test-only pool-level wrapper below) is the only other
// caller of `change_room_tx`; production code goes through
// `change_room_idempotent` below, which `commands/rooms.rs` wires up.
#[cfg(test)]
use super::support::{begin_immediate_tx, fetch_booking};
#[cfg(test)]
use crate::models::Booking;

/// Dải đêm sẽ được chuyển: các dòng `room_calendar` của booking có ngày >= `today`.
///
/// Không lấy "từ hôm nay tới hết kỳ": đặt trước nhận phòng tuần sau thì dải phải
/// bắt đầu từ ngày nhận phòng, nếu không sẽ tính tiền cho những đêm không tồn tại.
struct RemainingNights {
    from_date: String,
    to_date: String,
    remaining: i32,
    stayed: i32,
}

async fn remaining_nights(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    today: NaiveDate,
) -> BookingResult<RemainingNights> {
    let today_str = today.format("%Y-%m-%d").to_string();

    let row = sqlx::query(
        "SELECT MIN(date) AS from_date, MAX(date) AS to_date, COUNT(*) AS remaining
         FROM room_calendar
         WHERE booking_id = ? AND date >= ?",
    )
    .bind(booking_id)
    .bind(&today_str)
    .fetch_one(pool)
    .await?;

    let remaining: i32 = row.get("remaining");
    if remaining == 0 {
        return Err(BookingError::validation(
            "Không còn đêm nào để chuyển phòng".to_string(),
        ));
    }

    let stayed: i32 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM room_calendar WHERE booking_id = ? AND date < ?",
    )
    .bind(booking_id)
    .bind(&today_str)
    .fetch_one(pool)
    .await?;

    Ok(RemainingNights {
        from_date: row.get("from_date"),
        to_date: row.get("to_date"),
        remaining,
        stayed,
    })
}

async fn guest_count(pool: &Pool<Sqlite>, booking_id: &str) -> BookingResult<i32> {
    let count: i32 =
        sqlx::query_scalar("SELECT COUNT(*) FROM booking_guests WHERE booking_id = ?")
            .bind(booking_id)
            .fetch_one(pool)
            .await?;
    Ok(count.max(1))
}

pub async fn load_options(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    today: NaiveDate,
) -> BookingResult<RoomChangeOptions> {
    let booking = sqlx::query(
        "SELECT b.room_id, b.status, b.pricing_type, r.name AS room_name
         FROM bookings b JOIN rooms r ON r.id = b.room_id
         WHERE b.id = ?",
    )
    .bind(booking_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| BookingError::not_found(format!("Không tìm thấy booking {booking_id}")))?;

    let booking_status: String = booking.get("status");
    if booking_status != status::booking::ACTIVE && booking_status != status::booking::BOOKED {
        return Err(BookingError::validation(format!(
            "Booking {booking_id} không ở trạng thái chuyển phòng được (đang: {booking_status})"
        )));
    }

    let current_room_id: String = booking.get("room_id");
    let current_room_name: String = booking.get("room_name");
    let pricing_type = booking
        .get::<Option<String>, _>("pricing_type")
        .unwrap_or_else(|| "nightly".to_string());

    let nights = remaining_nights(pool, booking_id, today).await?;
    let guests = guest_count(pool, booking_id).await?;

    // Phòng trống suốt dải đêm còn lại. Dòng của chính booking này không tính là vướng.
    // Phòng đang dọn chỉ bị loại khi khách vào ngay tối nay. Với đặt trước tuần
    // sau, tình trạng dọn dẹp hôm nay không nói lên điều gì về tuần sau.
    let moving_in_today = nights.from_date == today.format("%Y-%m-%d").to_string();

    let candidates = sqlx::query(
        "SELECT r.id, r.name, r.type, r.floor, r.base_price, r.max_guests
         FROM rooms r
         WHERE r.id != ?
           AND r.max_guests >= ?
           AND (? = 0 OR r.status = 'vacant')
           AND NOT EXISTS (
             SELECT 1 FROM room_calendar rc
             WHERE rc.room_id = r.id
               AND rc.date >= ? AND rc.date <= ?
               AND (rc.booking_id IS NULL OR rc.booking_id != ?)
           )
         ORDER BY r.floor, r.id",
    )
    .bind(&current_room_id)
    .bind(guests)
    .bind(i32::from(moving_in_today))
    .bind(&nights.from_date)
    .bind(&nights.to_date)
    .bind(booking_id)
    .fetch_all(pool)
    .await?;

    let (range_start, range_end) = pricing_range(&nights);
    let mut tx = begin_tx(pool).await?;
    let current_price = calculate_stay_price_tx(
        &mut tx,
        &current_room_id,
        &range_start,
        &range_end,
        &pricing_type,
        Some(guests),
    )
    .await?
    .total;

    let mut rooms = Vec::with_capacity(candidates.len());
    for row in candidates {
        let room_id: String = row.get("id");
        let candidate_price = calculate_stay_price_tx(
            &mut tx,
            &room_id,
            &range_start,
            &range_end,
            &pricing_type,
            Some(guests),
        )
        .await?
        .total;

        rooms.push(RoomChangeOption {
            room_id,
            name: row.get("name"),
            room_type: row.get("type"),
            floor: row.get("floor"),
            base_price: read_money_vnd_or_zero(&row, "base_price"),
            max_guests: row.get("max_guests"),
            price_difference: candidate_price - current_price,
        });
    }
    tx.rollback().await.map_err(BookingError::from)?;

    Ok(RoomChangeOptions {
        booking_id: booking_id.to_string(),
        current_room_id,
        current_room_name,
        from_date: nights.from_date,
        to_date: nights.to_date,
        nights_remaining: nights.remaining,
        nights_stayed: nights.stayed,
        guest_count: guests,
        rooms,
    })
}

/// `calculate_stay_price_tx` nhận mốc nhận phòng và trả phòng, không nhận đêm đầu/cuối.
/// Đêm cuối là `to_date` nên mốc trả phòng là ngày kế tiếp.
fn pricing_range(nights: &RemainingNights) -> (String, String) {
    date_range_for_pricing(&nights.from_date, &nights.to_date)
}

/// Như `pricing_range`, nhưng nhận thẳng cặp ngày thay vì `RemainingNights` —
/// dùng ở nơi đã có sẵn `from_date`/`to_date` mà không cần dựng cả struct.
fn date_range_for_pricing(from_date: &str, to_date: &str) -> (String, String) {
    let last_night = NaiveDate::parse_from_str(to_date, "%Y-%m-%d")
        .expect("to_date lấy từ room_calendar nên luôn đúng dạng");
    let checkout = last_night + chrono::Duration::days(1);
    (
        format!("{from_date}T14:00:00+07:00"),
        format!("{}T12:00:00+07:00", checkout.format("%Y-%m-%d")),
    )
}

/// Chuyển các đêm còn lại (từ `today` trở đi) của `booking_id` sang `new_room_id`,
/// đổi `bookings.room_id`, và cập nhật trạng thái hai phòng.
///
/// Đêm đã ở giữ nguyên trên phòng cũ — chỉ những dòng `room_calendar` có
/// `date >= today` mới chuyển. Việc này giữ báo cáo công suất và hóa đơn đúng
/// sự thật.
///
/// Khi `keep_price == false`, khoản chênh lệch giữa giá phòng mới và phòng cũ
/// cho các đêm còn lại (cùng khoảng ngày nên các hệ số cuối tuần/lễ tự triệt
/// tiêu, chỉ còn lại phần chênh theo hạng phòng) được cộng vào `total_price` và
/// ghi một dòng `transactions`. Đây KHÔNG phải định giá lại toàn bộ kỳ ở: nó
/// cộng thêm phần chênh lên trên mức giá khách đang trả, nên khách đang có giá
/// ưu đãi vẫn giữ nguyên ưu đãi đó.
pub(super) async fn change_room_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
    new_room_id: &str,
    keep_price: bool,
    reason: Option<&str>,
    today: NaiveDate,
    origin_key: Option<String>,
) -> BookingResult<()> {
    let today_str = today.format("%Y-%m-%d").to_string();

    let booking =
        sqlx::query("SELECT room_id, status, notes, total_price FROM bookings WHERE id = ?")
            .bind(booking_id)
            .fetch_optional(&mut **tx)
            .await?
            .ok_or_else(|| {
                BookingError::not_found(format!("Không tìm thấy booking {booking_id}"))
            })?;

    let booking_status: String = booking.get("status");
    if booking_status != status::booking::ACTIVE && booking_status != status::booking::BOOKED {
        return Err(invalid_state_transition(format!(
            "booking {booking_id} is not movable (status: {booking_status})"
        )));
    }

    let old_room_id: String = booking.get("room_id");
    if old_room_id == new_room_id {
        return Err(BookingError::validation(
            "Phòng mới trùng phòng hiện tại".to_string(),
        ));
    }

    // Dải đêm sẽ chuyển.
    let range = sqlx::query(
        "SELECT MIN(date) AS from_date, MAX(date) AS to_date, COUNT(*) AS remaining
         FROM room_calendar WHERE booking_id = ? AND date >= ?",
    )
    .bind(booking_id)
    .bind(&today_str)
    .fetch_one(&mut **tx)
    .await?;
    let remaining: i32 = range.get("remaining");
    if remaining == 0 {
        return Err(BookingError::validation(
            "Không còn đêm nào để chuyển phòng".to_string(),
        ));
    }
    let from_date: String = range.get("from_date");
    let to_date: String = range.get("to_date");

    // Kiểm lại trong transaction, không tin danh sách giao diện gửi lên.
    let conflict: Option<String> = sqlx::query_scalar(
        "SELECT date FROM room_calendar
         WHERE room_id = ? AND date >= ? AND date <= ? AND booking_id != ?
         ORDER BY date LIMIT 1",
    )
    .bind(new_room_id)
    .bind(&from_date)
    .bind(&to_date)
    .bind(booking_id)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(date) = conflict {
        return Err(BookingError::conflict(format!(
            "Phòng {new_room_id} đã có lịch cho ngày {date}"
        )));
    }

    let guests: i32 = sqlx::query_scalar("SELECT COUNT(*) FROM booking_guests WHERE booking_id = ?")
        .bind(booking_id)
        .fetch_one(&mut **tx)
        .await?;
    let max_guests: Option<i32> = sqlx::query_scalar("SELECT max_guests FROM rooms WHERE id = ?")
        .bind(new_room_id)
        .fetch_optional(&mut **tx)
        .await?;
    let max_guests = max_guests
        .ok_or_else(|| BookingError::not_found(format!("Không tìm thấy phòng {new_room_id}")))?;
    if guests.max(1) > max_guests {
        return Err(BookingError::validation(format!(
            "Phòng {new_room_id} chỉ chứa tối đa {max_guests} khách"
        )));
    }

    // Tiền chênh lệch phải tính trước khi đổi room_id: cần old_room_id và
    // total_price đọc lúc mở hàm.
    if !keep_price {
        let pricing_type: String = sqlx::query_scalar(
            "SELECT COALESCE(pricing_type, 'nightly') FROM bookings WHERE id = ?",
        )
        .bind(booking_id)
        .fetch_one(&mut **tx)
        .await?;

        let (range_start, range_end) = date_range_for_pricing(&from_date, &to_date);

        let old_price = calculate_stay_price_tx(
            tx,
            &old_room_id,
            &range_start,
            &range_end,
            &pricing_type,
            Some(guests.max(1)),
        )
        .await?
        .total;
        let new_price = calculate_stay_price_tx(
            tx,
            new_room_id,
            &range_start,
            &range_end,
            &pricing_type,
            Some(guests.max(1)),
        )
        .await?
        .total;

        let difference = new_price - old_price;
        if difference != 0 {
            let current_total = read_money_vnd_or_zero(&booking, "total_price");
            let new_total = current_total
                .checked_add(difference)
                .ok_or_else(|| BookingError::validation("total_price overflowed".to_string()))?;

            let result = sqlx::query(
                "UPDATE bookings SET total_price = ? WHERE id = ? AND status = ? AND total_price = ?",
            )
            .bind(new_total)
            .bind(booking_id)
            .bind(&booking_status)
            .bind(current_total)
            .execute(&mut **tx)
            .await?;
            ensure_one_row_affected(
                result,
                format!("booking {booking_id} changed before room-change repricing"),
            )?;

            let note =
                format!("Chuyển phòng {old_room_id} → {new_room_id}: chênh {remaining} đêm");
            let created_at = Local::now().to_rfc3339();
            match &origin_key {
                Some(key) => {
                    let origin = OriginSideEffect::new(key.clone(), 0)?;
                    record_charge_with_origin_tx(
                        tx, booking_id, difference, note, created_at, &origin,
                    )
                    .await?;
                }
                None => {
                    record_charge_tx(tx, booking_id, difference, note, created_at).await?;
                }
            }
        }
    }

    // Chuyển đêm từ hôm nay trở đi.
    sqlx::query("UPDATE room_calendar SET room_id = ? WHERE booking_id = ? AND date >= ?")
        .bind(new_room_id)
        .bind(booking_id)
        .bind(&today_str)
        .execute(&mut **tx)
        .await?;

    let mut note_line = format!(
        "Chuyển phòng {old_room_id} → {new_room_id} ngày {}",
        today.format("%d/%m/%Y")
    );
    if let Some(reason) = reason.map(str::trim).filter(|value| !value.is_empty()) {
        note_line.push_str(&format!(" ({reason})"));
    }
    let existing_notes: Option<String> = booking.get("notes");
    let merged_notes = match existing_notes.as_deref().map(str::trim) {
        Some(existing) if !existing.is_empty() => format!("{existing} | {note_line}"),
        _ => note_line,
    };

    let result = sqlx::query(
        "UPDATE bookings SET room_id = ?, notes = ? WHERE id = ? AND status = ? AND room_id = ?",
    )
    .bind(new_room_id)
    .bind(&merged_notes)
    .bind(booking_id)
    .bind(&booking_status)
    .bind(&old_room_id)
    .execute(&mut **tx)
    .await?;
    ensure_one_row_affected(
        result,
        format!("booking {booking_id} changed before room-change"),
    )?;

    // Trạng thái phòng chỉ đổi khi khách đang ở thật. Mirrors the check-out /
    // check-in room-status transitions (stay_lifecycle.rs): every UPDATE
    // carries the old-state condition and is checked with
    // ensure_one_row_affected, so a room that already changed status under us
    // fails loudly instead of silently overwriting it.
    if booking_status == status::booking::ACTIVE {
        let result = sqlx::query("UPDATE rooms SET status = ? WHERE id = ? AND status = ?")
            .bind(status::room::CLEANING)
            .bind(&old_room_id)
            .bind(status::room::OCCUPIED)
            .execute(&mut **tx)
            .await?;
        ensure_one_row_affected(
            result,
            format!("room {old_room_id} is no longer occupied"),
        )?;

        let now = Local::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO housekeeping (id, room_id, status, note, triggered_at, cleaned_at, created_at)
             VALUES (?, ?, 'needs_cleaning', ?, ?, NULL, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(&old_room_id)
        .bind(format!("Khách chuyển sang phòng {new_room_id}"))
        .bind(&now)
        .bind(&now)
        .execute(&mut **tx)
        .await?;

        let result = sqlx::query("UPDATE rooms SET status = ? WHERE id = ? AND status = ?")
            .bind(status::room::OCCUPIED)
            .bind(new_room_id)
            .bind(status::room::VACANT)
            .execute(&mut **tx)
            .await?;
        ensure_one_row_affected(
            result,
            format!("room {new_room_id} is no longer vacant"),
        )?;
    }

    Ok(())
}

pub(super) fn map_change_room_command_error(error: BookingError) -> CommandError {
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

fn change_room_initial_lock_keys_from_payload(
    payload: &serde_json::Value,
) -> CommandResult<Vec<String>> {
    let booking_id = payload
        .get("booking_id")
        .and_then(|value| value.as_str())
        .ok_or_else(|| system_error("change-room lock payload missing booking_id"))?;
    Ok(vec![crate::aggregate_locks::booking_key(booking_id)?])
}

/// Idempotent wrapper around `change_room_tx`, wired into the same
/// idempotency key → aggregate lock → command ledger → recovery queue →
/// outbox event pipeline as `extend_stay_idempotent` (`stay_lifecycle.rs`).
///
/// The lock set is four keys — booking, old room, new room, folio — resolved
/// only once the booking's current room is known (the old room is not part of
/// the request payload). `aggregate_locks::canonicalize_lock_keys` sorts and
/// dedupes the resolved set, so two opposing room-change commands (A→B and
/// B→A running concurrently) naturally acquire their locks in the same order
/// and cannot deadlock each other.
pub async fn change_room_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    booking_id: &str,
    new_room_id: &str,
    keep_price: bool,
    reason: Option<String>,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let hash_payload = json!({
        "schema": "stay.change_room.v1",
        "booking_id": booking_id,
        "new_room_id": new_room_id,
        "keep_price": keep_price,
        "reason": reason,
    });
    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("stay.change_room.v1")),
        ("booking_present", json!(true)),
        ("operation", json!("change_room")),
        ("keep_price", json!(keep_price)),
        ("reason_present", json!(reason.is_some())),
    ])?;
    let summary = CommandLedgerSummary::new("Change room")?.with_aggregate_ref(
        "booking",
        "booking",
        None::<String>,
    )?;
    let request = WriteCommandRequest::new_sanitized(hash_payload.clone(), ledger_intent, summary)?
        .with_primary_aggregate_key(format!("booking:{booking_id}"))
        .with_lock_key_deriver(change_room_initial_lock_keys_from_payload)
        .with_success_summary(CommandLedgerResultSummary::success("Room changed")?)
        .with_outbox_event(OutboxEventSpec::new(
            "booking.room_changed",
            OutboxAggregateKeySource::response_field("booking", "id"),
            &["bookings", "rooms", "folio"],
        )?);

    let pool_for_lookup = pool.clone();
    let booking_id_for_lookup = booking_id.to_string();
    let booking_id_for_service = booking_id.to_string();
    let new_room_for_lookup = new_room_id.to_string();
    let new_room_for_service = new_room_id.to_string();
    let origin_key = format!("{}:{}", ctx.command_name, ctx.idempotency_key);

    WriteCommandExecutor::new(pool.clone())
        .execute_with_resolved_guard(
            ctx,
            request,
            move || async move {
                let old_room_id =
                    lookup_booking_room_id(&pool_for_lookup, &booking_id_for_lookup)
                        .await
                        .map_err(map_change_room_command_error)?;
                let lock_keys = vec![
                    crate::aggregate_locks::booking_key(&booking_id_for_lookup)?,
                    crate::aggregate_locks::room_key(&old_room_id)?,
                    crate::aggregate_locks::room_key(&new_room_for_lookup)?,
                    crate::aggregate_locks::folio_key(&booking_id_for_lookup)?,
                ];
                let guard = crate::aggregate_locks::global_manager()
                    .acquire(lock_keys.clone())
                    .await?;
                Ok(ResolvedWriteCommandGuard::new(guard, lock_keys))
            },
            move |tx, _resolved| {
                Box::pin(async move {
                    change_room_tx(
                        tx,
                        &booking_id_for_service,
                        &new_room_for_service,
                        keep_price,
                        reason.as_deref(),
                        Local::now().date_naive(),
                        Some(origin_key),
                    )
                    .await
                    .map_err(map_change_room_command_error)?;

                    let booking = fetch_booking_tx(tx, &booking_id_for_service)
                        .await
                        .map_err(map_change_room_command_error)?;
                    serde_json::to_value(&booking).map_err(system_error)
                })
            },
        )
        .await
}

#[cfg(test)]
pub async fn change_room(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    new_room_id: &str,
    keep_price: bool,
    reason: Option<&str>,
    today: NaiveDate,
) -> BookingResult<Booking> {
    let mut tx = begin_immediate_tx(pool).await?;
    change_room_tx(
        &mut tx,
        booking_id,
        new_room_id,
        keep_price,
        reason,
        today,
        None,
    )
    .await?;
    tx.commit().await.map_err(BookingError::from)?;

    fetch_booking(
        pool,
        booking_id,
        format!("Không tìm thấy booking {booking_id}"),
        read_money_vnd_or_zero,
    )
    .await
}
