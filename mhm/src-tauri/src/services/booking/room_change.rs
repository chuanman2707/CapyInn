//! Room change read-side: which rooms a booking could move to, and the price
//! difference for each.
//!
//! `bookings.room_id` holds the guest's CURRENT room; `room_calendar` (one row
//! per room per night) holds the real history. When a guest moves mid-stay,
//! nights already slept stay on the old room and only nights from today
//! forward move. So "remaining nights" is derived from `room_calendar` rows
//! with `date >= today` — not from "today until checkout" — which matters for
//! future reservations that start days from now.

use chrono::NaiveDate;
use sqlx::{Pool, Row, Sqlite};

use crate::{
    domain::booking::{BookingError, BookingResult},
    models::{status, RoomChangeOption, RoomChangeOptions},
};

use super::{
    pricing_service::calculate_stay_price_tx,
    support::{begin_tx, read_money_vnd_or_zero},
};

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

#[allow(dead_code)]
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
    let last_night = NaiveDate::parse_from_str(&nights.to_date, "%Y-%m-%d")
        .expect("to_date lấy từ room_calendar nên luôn đúng dạng");
    let checkout = last_night + chrono::Duration::days(1);
    (
        format!("{}T14:00:00+07:00", nights.from_date),
        format!("{}T12:00:00+07:00", checkout.format("%Y-%m-%d")),
    )
}
