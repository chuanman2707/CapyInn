//! Gỡ một booking khỏi vòng đời vì nó được tạo do thao tác sai.
//!
//! Khác `cancel_reservation` ở bản chất: huỷ là một sự kiện kinh doanh có thật
//! (khách huỷ, có thể giữ cọc, ghi phí huỷ). Xoá ở đây nghĩa là "chưa từng xảy
//! ra" — không phí, không cọc, biến mất khỏi mọi báo cáo.
//!
//! Không dòng tiền nào bị xoá. `transactions` và `folio_lines` là append-only;
//! tiền biến mất khỏi báo cáo nhờ bộ lọc trạng thái trong `revenue_queries`,
//! không nhờ việc xoá dữ liệu.

use chrono::{DateTime, Local};
use serde_json::json;
use sqlx::{Pool, Row, Sqlite, Transaction};

use crate::{
    app_error::CommandResult,
    command_idempotency::{
        system_error, CommandLedgerResultSummary, CommandLedgerSummary, IdempotentCommandResult,
        ResolvedWriteCommandGuard, SanitizedLedgerIntent, WriteCommandContext,
        WriteCommandExecutor, WriteCommandRequest,
    },
    domain::booking::{BookingError, BookingResult},
    models::{status, VoidBookingRequest, VoidBookingResponse},
    outbox::{OutboxAggregateKeySource, OutboxEventSpec},
};

use super::{
    stay_lifecycle::{map_check_out_command_error as map_void_command_error, mark_write_db_error},
    support::{
        ensure_one_row_affected, invalid_state_transition, lock_booking_and_read_room, FolioLock,
    },
};

/// Trạng thái nào được phép xoá. `cancelled` và `no_show` đã là kết cục cuối,
/// xoá thêm không thay đổi gì trong báo cáo mà chỉ làm rối lịch sử.
// `#[allow(dead_code)]` trên cả hàm này lẫn `void_booking_tx`: chưa command
// nào gọi tới ngoài test (Task 8 nối dây). Bỏ khi Task 8 xong — xem cùng ghi
// chú ở `VoidBookingRequest`/`VoidBookingResponse` trong models.rs.
#[allow(dead_code)]
fn ensure_voidable(current_status: &str) -> BookingResult<()> {
    match current_status {
        // Nhận đủ ba trạng thái sống: `booked`, `active`, `checked_out` — thân
        // `void_booking_tx` biết dọn cho cả ba (xoá room_calendar cho cả ba,
        // trả phòng về vacant thêm cho active/checked_out, gỡ task dọn phòng
        // và huỷ hoá đơn thêm cho checked_out). Mỗi trạng thái chỉ được nhận
        // sau khi thân hàm biết dọn theo nó — đừng nới nhánh này ra trước.
        status::booking::BOOKED | status::booking::ACTIVE | status::booking::CHECKED_OUT => Ok(()),
        status::booking::VOIDED => Err(invalid_state_transition(
            "Lượt này đã được xóa rồi — vui lòng tải lại trang",
        )),
        other => Err(invalid_state_transition(format!(
            "Không xóa được lượt ở trạng thái {other}"
        ))),
    }
}

#[allow(dead_code)]
pub async fn void_booking_tx(
    tx: &mut Transaction<'_, Sqlite>,
    req: &VoidBookingRequest,
    actor_id: &str,
    now: DateTime<Local>,
    locked_room_id: &str,
) -> BookingResult<VoidBookingResponse> {
    let row = sqlx::query("SELECT room_id, status, group_id FROM bookings WHERE id = ?")
        .bind(&req.booking_id)
        .fetch_optional(&mut **tx)
        .await?
        .ok_or_else(|| {
            BookingError::not_found(format!("Không tìm thấy booking {}", req.booking_id))
        })?;

    let room_id: String = row.get("room_id");
    let previous_status: String = row.get("status");
    let group_id: Option<String> = row.get("group_id");

    if group_id.is_some() {
        return Err(BookingError::validation(
            "Lượt này thuộc đoàn — chưa hỗ trợ xóa từng phòng".to_string(),
        ));
    }

    ensure_voidable(&previous_status)?;

    if locked_room_id != room_id {
        return Err(invalid_state_transition(format!(
            "booking {} đã đổi phòng trước khi xóa — vui lòng tải lại trang",
            req.booking_id
        )));
    }

    let voided_at = now.to_rfc3339();

    let result = sqlx::query(
        "UPDATE bookings
         SET status = ?, voided_at = ?, voided_by = ?, void_reason = ?
         WHERE id = ? AND status = ?",
    )
    .bind(status::booking::VOIDED)
    .bind(&voided_at)
    .bind(actor_id)
    .bind(&req.reason)
    .bind(&req.booking_id)
    .bind(&previous_status)
    .execute(&mut **tx)
    .await
    .map_err(BookingError::from)
    .map_err(mark_write_db_error)?;
    ensure_one_row_affected(
        result,
        "Lượt vừa thay đổi bởi thao tác khác — vui lòng tải lại trang",
    )?;

    sqlx::query("DELETE FROM room_calendar WHERE booking_id = ?")
        .bind(&req.booking_id)
        .execute(&mut **tx)
        .await
        .map_err(BookingError::from)
        .map_err(mark_write_db_error)?;

    match previous_status.as_str() {
        status::booking::ACTIVE => {
            let result = sqlx::query("UPDATE rooms SET status = ? WHERE id = ? AND status = ?")
                .bind(status::room::VACANT)
                .bind(&room_id)
                .bind(status::room::OCCUPIED)
                .execute(&mut **tx)
                .await
                .map_err(BookingError::from)
                .map_err(mark_write_db_error)?;
            ensure_one_row_affected(
                result,
                format!(
                    "Phòng {room_id} vừa đổi trạng thái bởi thao tác khác — vui lòng tải lại trang"
                ),
            )?;
        }
        status::booking::CHECKED_OUT => {
            // 0 dòng ảnh hưởng ở ĐÂY là hợp lệ, khác mọi chỗ khác trong repo:
            // phòng có thể đã được bán lại cho khách khác giữa lúc trả phòng và
            // lúc phát hiện nhập sai. Khách mới không được bị đá ra. Đừng "sửa
            // lỗi" chỗ này thành `ensure_one_row_affected`.
            sqlx::query("UPDATE rooms SET status = ? WHERE id = ? AND status = ?")
                .bind(status::room::VACANT)
                .bind(&room_id)
                .bind(status::room::CLEANING)
                .execute(&mut **tx)
                .await
                .map_err(BookingError::from)
                .map_err(mark_write_db_error)?;

            // `housekeeping` không có cột `booking_id`. `check_out_tx` chèn task
            // với `triggered_at` bằng đúng `actual_checkout` của booking, nên bộ
            // ba (phòng, thời điểm, chưa dọn xong) nhận diện được task do chính
            // lượt này sinh ra — và chỉ nó.
            sqlx::query(
                "DELETE FROM housekeeping
                 WHERE room_id = ?
                   AND cleaned_at IS NULL
                   AND triggered_at = (
                       SELECT actual_checkout FROM bookings WHERE id = ?
                   )",
            )
            .bind(&room_id)
            .bind(&req.booking_id)
            .execute(&mut **tx)
            .await
            .map_err(BookingError::from)
            .map_err(mark_write_db_error)?;

            // Hoá đơn đã xuất giữ nguyên số — dãy số không được thủng lỗ không
            // giải thích được. Cột `status` đã có sẵn, mặc định 'issued'.
            sqlx::query("UPDATE invoices SET status = 'voided' WHERE booking_id = ?")
                .bind(&req.booking_id)
                .execute(&mut **tx)
                .await
                .map_err(BookingError::from)
                .map_err(mark_write_db_error)?;
        }
        _ => {}
    }

    Ok(VoidBookingResponse {
        ok: true,
        booking_id: req.booking_id.clone(),
        room_id,
        previous_status,
        voided_at,
    })
}

// `#[allow(dead_code)]` trên cả ba hàm dưới đây (`build_void_hash_payload`,
// `void_initial_lock_keys_from_payload`, `void_booking_idempotent`): cùng lý
// do như `void_booking_tx` ở trên — chưa command nào gọi tới ngoài test (Task
// 8 nối dây). Bỏ khi Task 8 xong.
#[allow(dead_code)]
fn build_void_hash_payload(req: &VoidBookingRequest) -> serde_json::Value {
    json!({
        "booking_id": req.booking_id,
        "reason_present": req.reason.is_some(),
    })
}

#[allow(dead_code)]
fn void_initial_lock_keys_from_payload(
    hash_payload: &serde_json::Value,
) -> CommandResult<Vec<String>> {
    let booking_id = hash_payload
        .get("booking_id")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| system_error("void lock payload missing booking_id"))?;
    Ok(vec![
        crate::aggregate_locks::booking_key(booking_id)?,
        crate::aggregate_locks::folio_key(booking_id)?,
    ])
}

#[allow(dead_code)]
pub async fn void_booking_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    req: VoidBookingRequest,
    actor_id: String,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let hash_payload = build_void_hash_payload(&req);
    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("schema", json!("stay.void.v1")),
        ("booking_present", json!(true)),
        ("reason_present", json!(req.reason.is_some())),
    ])?;
    let summary = CommandLedgerSummary::new("Void booking")?.with_aggregate_ref(
        "booking",
        "booking",
        None::<String>,
    )?;
    let request = WriteCommandRequest::new_sanitized(hash_payload.clone(), ledger_intent, summary)?
        .with_primary_aggregate_key(format!("booking:{}", req.booking_id))
        .with_lock_key_deriver(void_initial_lock_keys_from_payload)
        .with_success_summary(CommandLedgerResultSummary::success("Booking voided")?)
        .with_outbox_event(OutboxEventSpec::new(
            "booking.voided",
            OutboxAggregateKeySource::response_field("booking", "booking_id"),
            &["bookings", "rooms", "housekeeping"],
        )?);

    let pool_for_lookup = pool.clone();
    let booking_id_for_lookup = req.booking_id.clone();

    WriteCommandExecutor::new(pool.clone())
        .execute_with_resolved_guard(
            ctx,
            request,
            move || async move {
                let (guard, room_id) = lock_booking_and_read_room(
                    &pool_for_lookup,
                    &booking_id_for_lookup,
                    FolioLock::Include,
                    map_void_command_error,
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
                        void_booking_tx(tx, &req, &actor_id, Local::now(), &locked_room_id)
                            .await
                            .map_err(map_void_command_error)?;
                    serde_json::to_value(&response).map_err(system_error)
                })
            },
        )
        .await
}
