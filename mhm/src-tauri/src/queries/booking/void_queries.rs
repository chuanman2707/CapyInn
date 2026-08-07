//! Dữ liệu cho hộp xác nhận xoá. Chỉ đọc — không mutate gì.
//!
//! Con số ở đây là con số DUY NHẤT được hiển thị cho người bấm. Frontend không
//! tính lại: tính hai nơi là cách chắc chắn nhất để hộp thoại hứa 500.000₫
//! trong khi backend gỡ đi một số khác.

use chrono::{Local, NaiveDate};
use sqlx::{Pool, Row, Sqlite};

use crate::{
    db::row::{get_money_vnd, get_optional_money_vnd},
    models::{status, VoidBookingPreview},
};

/// Ngày lịch địa phương của một mốc `check_in_at`: 10 ký tự đầu, cùng quy ước
/// với `db::local_day::local_date_sql` — cột này mang cả hai dạng tuỳ đường
/// ghi, mốc đủ giờ (`"...T10:00:00+07:00"`, lượt đã nhận phòng) lẫn ngày trần
/// (`"2026-04-20"`, lượt mới đặt), và cả hai đều lấy đúng bằng cách cắt 10 ký
/// tự thay vì parse RFC3339 nguyên chuỗi (dạng ngày trần không phải RFC3339
/// hợp lệ, `parse_from_rfc3339` sẽ lỗi ngay).
fn local_calendar_date(value: &str) -> Option<NaiveDate> {
    value
        .get(0..10)
        .and_then(|slice| NaiveDate::parse_from_str(slice, "%Y-%m-%d").ok())
}

/// `#[allow(dead_code)]`: chưa lệnh nào gọi tới (Task 8 nối dây). Xem cùng ghi
/// chú ở `VoidBookingRequest`/`VoidBookingResponse` trong `models.rs`.
#[allow(dead_code)]
pub async fn load_void_preview(
    pool: &Pool<Sqlite>,
    booking_id: &str,
) -> Result<VoidBookingPreview, sqlx::Error> {
    let row = sqlx::query(
        "SELECT b.id, b.room_id, b.status, b.check_in_at,
                b.actual_checkout, b.nights, b.total_price, b.deposit_amount,
                b.is_audited, b.group_id, r.status AS room_status,
                COALESCE(g.full_name, '') AS guest_name
         FROM bookings b
         LEFT JOIN rooms r ON r.id = b.room_id
         LEFT JOIN guests g ON g.id = b.primary_guest_id
         WHERE b.id = ?",
    )
    .bind(booking_id)
    .fetch_one(pool)
    .await?;

    let previous_status: String = row.get("status");
    let nights_total: i32 = row.get("nights");
    let total_price = get_money_vnd(&row, "total_price");
    let deposit_amount = get_optional_money_vnd(&row, "deposit_amount").unwrap_or(0);
    let room_status: Option<String> = row.get("room_status");
    let group_id: Option<String> = row.get("group_id");
    let check_in_at: String = row.get("check_in_at");
    let actual_checkout: Option<String> = row.get("actual_checkout");

    let today = Local::now().date_naive();
    let check_in_date = local_calendar_date(&check_in_at);

    // Doanh thu tiền phòng phân bổ theo đêm, và một đêm được ghi vào sổ NGAY
    // KHI bắt đầu — khớp với `recognized_room_revenue_amount_sql`
    // (`revenue_queries.rs`): ngày check-in đã tính 1 đêm dù đêm đó chưa qua.
    // Vì vậy cộng thêm 1 vào số ngày đã trôi qua kể từ check-in, không chỉ đếm
    // số ngày trôi qua suông — thiếu +1 thì lượt nhận phòng NGAY HÔM NAY báo 0
    // đêm ghi nhận thay vì 1, và "đang ở đêm 2 trên 3" báo 1 thay vì 2.
    let nights_recognized = match previous_status.as_str() {
        status::booking::CHECKED_OUT => nights_total,
        status::booking::ACTIVE => match check_in_date {
            Some(date) => {
                let elapsed_days = (today - date).num_days().max(0);
                let nights_total_clamped = i64::from(nights_total).max(0);
                (elapsed_days + 1).min(nights_total_clamped) as i32
            }
            None => 0,
        },
        _ => 0,
    };

    // Chia nguyên: phần dư (tối đa `nights_total - 1` đồng) rơi mất chứ không
    // được làm tròn lên. Chấp nhận được — đây là con số cho hộp xác nhận đọc
    // trước khi xoá, không phải bút toán sổ sách; và làm tròn lên sẽ có ngày
    // gỡ NHIỀU hơn số tiền lượt đó thực sự còn đóng góp.
    let revenue_impact = match previous_status.as_str() {
        status::booking::CHECKED_OUT => total_price,
        status::booking::ACTIVE if nights_total > 0 => {
            total_price * i64::from(nights_recognized) / i64::from(nights_total)
        }
        status::booking::BOOKED => deposit_amount,
        _ => 0,
    };

    // Ngày mà con số trên đang được tính vào: lượt đã trả phòng gắn với đúng
    // ngày trả phòng; lượt đang ở gắn với HÔM NAY — ngày mà phần "đã ghi nhận"
    // còn đúng, sẽ trôi tiếp nếu không xoá hôm nay; lượt đặt trước (không có
    // ngày ghi nhận thật cho tiền cọc) và mọi trạng thái khác rơi về ngày nhận
    // phòng dự kiến — cột không NULL được nên vẫn cần một giá trị hợp lệ.
    let fallback_date = check_in_at.get(0..10).unwrap_or("").to_string();
    let revenue_date = match previous_status.as_str() {
        status::booking::CHECKED_OUT => actual_checkout
            .as_deref()
            .and_then(|value| value.get(0..10))
            .map(str::to_string)
            .unwrap_or_else(|| fallback_date.clone()),
        status::booking::ACTIVE => today.format("%Y-%m-%d").to_string(),
        _ => fallback_date,
    };

    // Chỉ nhánh checked_out có chỗ `void_booking_tx` âm thầm bỏ qua UPDATE
    // rooms khi phòng không còn ở trạng thái nó chờ (đã bán lại) — active luôn
    // ép về trống hoặc lỗi to, booked chưa từng đụng rooms. Nên đây là nhánh
    // duy nhất "trạng thái phòng giữ nguyên" là một cảnh báo thật.
    let room_was_reused = previous_status == status::booking::CHECKED_OUT
        && room_status.as_deref() == Some(status::room::OCCUPIED);

    Ok(VoidBookingPreview {
        booking_id: row.get("id"),
        guest_name: row.get("guest_name"),
        room_id: row.get("room_id"),
        previous_status,
        revenue_impact,
        revenue_date,
        nights_recognized,
        nights_total,
        is_audited: row.get::<i32, _>("is_audited") == 1,
        room_was_reused,
        is_group_booking: group_id.is_some(),
    })
}
