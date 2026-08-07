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
use sqlx::{Row, Sqlite, Transaction};

use crate::{
    domain::booking::{BookingError, BookingResult},
    models::{status, VoidBookingRequest, VoidBookingResponse},
};

use super::{
    stay_lifecycle::mark_write_db_error,
    support::{ensure_one_row_affected, invalid_state_transition},
};

/// Trạng thái nào được phép xoá. `cancelled` và `no_show` đã là kết cục cuối,
/// xoá thêm không thay đổi gì trong báo cáo mà chỉ làm rối lịch sử.
// `#[allow(dead_code)]` trên cả hàm này lẫn `void_booking_tx`: chưa command
// nào gọi tới ngoài test (Task 8 nối dây). Bỏ khi Task 8 xong — xem cùng ghi
// chú ở `VoidBookingRequest`/`VoidBookingResponse` trong models.rs.
#[allow(dead_code)]
fn ensure_voidable(current_status: &str) -> BookingResult<()> {
    match current_status {
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
            "booking {} changed rooms before void",
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

    Ok(VoidBookingResponse {
        ok: true,
        booking_id: req.booking_id.clone(),
        room_id,
        previous_status,
        voided_at,
    })
}
