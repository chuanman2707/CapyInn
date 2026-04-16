use chrono::{Duration, Local, NaiveDate};
use sqlx::{Pool, Row, Sqlite, Transaction};

use crate::{
    domain::booking::{pricing::calculate_stay_price_tx, BookingError, BookingResult},
    models::{status, Booking, CheckInRequest, CheckOutRequest},
};

use super::{
    billing_service::{record_charge_tx, record_payment_tx},
    guest_service::{create_guest_manifest, link_booking_guests},
    support::{begin_tx, parse_booking_datetime},
};

pub async fn check_in(
    pool: &Pool<Sqlite>,
    req: CheckInRequest,
    user_id: Option<String>,
) -> BookingResult<Booking> {
    validate_check_in_request(&req)?;

    let now = Local::now();
    let check_in_at = now.to_rfc3339();
    let expected_checkout = (now + Duration::days(req.nights as i64)).to_rfc3339();
    let checkin_date = now.date_naive();
    let checkout_date = (now + Duration::days(req.nights as i64)).date_naive();
    let pricing_type = req
        .pricing_type
        .clone()
        .unwrap_or_else(|| "nightly".to_string());

    let mut tx = begin_tx(pool).await?;

    let room = sqlx::query("SELECT id, status FROM rooms WHERE id = ?")
        .bind(&req.room_id)
        .fetch_optional(&mut *tx)
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
    .fetch_all(&mut *tx)
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

    let pricing = calculate_stay_price_tx(
        &mut tx,
        &req.room_id,
        &check_in_at,
        &expected_checkout,
        &pricing_type,
    )
    .await?;

    let booking_id = uuid::Uuid::new_v4().to_string();
    let guest_manifest = create_guest_manifest(&mut tx, &req.guests, &check_in_at).await?;

    sqlx::query(
        "INSERT INTO bookings (
            id, room_id, primary_guest_id, check_in_at, expected_checkout,
            actual_checkout, nights, total_price, paid_amount, status, source,
            notes, created_by, booking_type, pricing_type, pricing_snapshot, created_at
        ) VALUES (?, ?, ?, ?, ?, NULL, ?, ?, 0, ?, ?, ?, ?, 'walk-in', ?, NULL, ?)",
    )
    .bind(&booking_id)
    .bind(&req.room_id)
    .bind(&guest_manifest.primary_guest_id)
    .bind(&check_in_at)
    .bind(&expected_checkout)
    .bind(req.nights)
    .bind(pricing.total)
    .bind(status::booking::ACTIVE)
    .bind(req.source.as_deref().unwrap_or("walk-in"))
    .bind(&req.notes)
    .bind(user_id.as_deref())
    .bind(&pricing_type)
    .bind(&check_in_at)
    .execute(&mut *tx)
    .await?;

    link_booking_guests(&mut tx, &booking_id, &guest_manifest.guest_ids).await?;

    record_charge_tx(
        &mut tx,
        &booking_id,
        pricing.total,
        "Tiền phòng",
        check_in_at.clone(),
    )
    .await?;

    if let Some(paid_amount) = req.paid_amount.filter(|amount| *amount > 0.0) {
        record_payment_tx(&mut tx, &booking_id, paid_amount, "Thanh toán khi check-in").await?;
    }

    insert_occupied_calendar_rows(
        &mut tx,
        &req.room_id,
        &booking_id,
        checkin_date,
        checkout_date,
    )
    .await?;

    sqlx::query("UPDATE rooms SET status = ? WHERE id = ?")
        .bind(status::room::OCCUPIED)
        .bind(&req.room_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await.map_err(BookingError::from)?;

    fetch_booking(pool, &booking_id).await
}

pub async fn check_out(pool: &Pool<Sqlite>, req: CheckOutRequest) -> BookingResult<()> {
    let mut tx = begin_tx(pool).await?;

    let booking =
        sqlx::query("SELECT room_id, paid_amount FROM bookings WHERE id = ? AND status = ?")
            .bind(&req.booking_id)
            .bind(status::booking::ACTIVE)
            .fetch_optional(&mut *tx)
            .await?;

    let booking = booking.ok_or_else(|| {
        BookingError::not_found(format!(
            "Không tìm thấy booking đang active {}",
            req.booking_id
        ))
    })?;

    let room_id: String = booking.get("room_id");
    let already_paid = read_f64(&booking, "paid_amount");

    if let Some(final_paid) = req.final_paid {
        let delta = final_paid - already_paid;
        if delta > 0.0 {
            record_payment_tx(&mut tx, &req.booking_id, delta, "Thanh toán khi check-out").await?;
        }
    }

    let actual_checkout = Local::now().to_rfc3339();

    sqlx::query("UPDATE bookings SET status = ?, actual_checkout = ? WHERE id = ?")
        .bind(status::booking::CHECKED_OUT)
        .bind(&actual_checkout)
        .bind(&req.booking_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query("UPDATE rooms SET status = ? WHERE id = ?")
        .bind(status::room::CLEANING)
        .bind(&room_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "INSERT INTO housekeeping (id, room_id, status, triggered_at, created_at)
         VALUES (?, ?, 'needs_cleaning', ?, ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(&room_id)
    .bind(&actual_checkout)
    .bind(&actual_checkout)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM room_calendar WHERE booking_id = ?")
        .bind(&req.booking_id)
        .execute(&mut *tx)
        .await?;

    tx.commit().await.map_err(BookingError::from)?;

    Ok(())
}

pub async fn extend_stay(pool: &Pool<Sqlite>, booking_id: &str) -> BookingResult<Booking> {
    let mut tx = begin_tx(pool).await?;

    let booking = sqlx::query(
        "SELECT room_id, nights, total_price, expected_checkout, pricing_type
         FROM bookings WHERE id = ? AND status = ?",
    )
    .bind(booking_id)
    .bind(status::booking::ACTIVE)
    .fetch_optional(&mut *tx)
    .await?;

    let booking = booking.ok_or_else(|| {
        BookingError::not_found(format!("Không tìm thấy booking đang active {}", booking_id))
    })?;

    let room_id: String = booking.get("room_id");
    let current_nights: i32 = booking.get("nights");
    let current_total = read_f64(&booking, "total_price");
    let old_expected_checkout: String = booking.get("expected_checkout");
    let pricing_type = booking
        .get::<Option<String>, _>("pricing_type")
        .unwrap_or_else(|| "nightly".to_string());

    let old_expected = parse_booking_datetime(&old_expected_checkout)?;
    let new_expected = old_expected + Duration::days(1);
    let extension_date = old_expected.date_naive();

    let room_exists = sqlx::query_scalar::<_, String>("SELECT id FROM rooms WHERE id = ? LIMIT 1")
        .bind(&room_id)
        .fetch_optional(&mut *tx)
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
    .fetch_optional(&mut *tx)
    .await?;

    if conflict.is_some() {
        return Err(BookingError::conflict(format!(
            "Phòng {} đã có lịch cho ngày {}",
            room_id,
            extension_date.format("%Y-%m-%d")
        )));
    }

    let incremental_pricing = calculate_stay_price_tx(
        &mut tx,
        &room_id,
        &old_expected_checkout,
        &new_expected.to_rfc3339(),
        &pricing_type,
    )
    .await?;

    let new_total = current_total + incremental_pricing.total;
    let new_checkout = new_expected.to_rfc3339();

    sqlx::query(
        "UPDATE bookings SET nights = ?, total_price = ?, expected_checkout = ? WHERE id = ?",
    )
    .bind(current_nights + 1)
    .bind(new_total)
    .bind(&new_checkout)
    .bind(booking_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT OR REPLACE INTO room_calendar (room_id, date, booking_id, status)
         VALUES (?, ?, ?, ?)",
    )
    .bind(&room_id)
    .bind(extension_date.format("%Y-%m-%d").to_string())
    .bind(booking_id)
    .bind(status::calendar::OCCUPIED)
    .execute(&mut *tx)
    .await?;

    record_charge_tx(
        &mut tx,
        booking_id,
        incremental_pricing.total,
        "Extended stay +1 night",
        Local::now().to_rfc3339(),
    )
    .await?;

    tx.commit().await.map_err(BookingError::from)?;

    fetch_booking(pool, booking_id).await
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

    Ok(())
}

async fn insert_occupied_calendar_rows(
    tx: &mut Transaction<'_, Sqlite>,
    room_id: &str,
    booking_id: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> BookingResult<()> {
    let mut date = start_date;
    while date < end_date {
        sqlx::query(
            "INSERT OR REPLACE INTO room_calendar (room_id, date, booking_id, status)
             VALUES (?, ?, ?, ?)",
        )
        .bind(room_id)
        .bind(date.format("%Y-%m-%d").to_string())
        .bind(booking_id)
        .bind(status::calendar::OCCUPIED)
        .execute(&mut **tx)
        .await?;
        date += Duration::days(1);
    }

    Ok(())
}

async fn fetch_booking(pool: &Pool<Sqlite>, booking_id: &str) -> BookingResult<Booking> {
    let row = sqlx::query(
        "SELECT id, room_id, primary_guest_id, check_in_at, expected_checkout,
                actual_checkout, nights, total_price, paid_amount, status,
                source, notes, created_at
         FROM bookings WHERE id = ?",
    )
    .bind(booking_id)
    .fetch_optional(pool)
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
        total_price: read_f64(&row, "total_price"),
        paid_amount: read_f64(&row, "paid_amount"),
        status: row.get("status"),
        source: row.get("source"),
        notes: row.get("notes"),
        created_at: row.get("created_at"),
    })
}

fn read_f64(row: &sqlx::sqlite::SqliteRow, column: &str) -> f64 {
    row.try_get::<Option<f64>, _>(column)
        .ok()
        .flatten()
        .or_else(|| {
            row.try_get::<Option<i64>, _>(column)
                .ok()
                .flatten()
                .map(|value| value as f64)
        })
        .unwrap_or(0.0)
}
