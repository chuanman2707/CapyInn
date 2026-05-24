use sqlx::{Pool, Sqlite, Transaction};

use crate::{domain::booking::BookingResult, money::MoneyVnd};

pub async fn seed_room(pool: &Pool<Sqlite>, room_id: &str) -> BookingResult<()> {
    sqlx::query(
        "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status)
         VALUES (?, ?, ?, ?, 0, 250000, 2, 0, 'vacant')",
    )
    .bind(room_id)
    .bind(format!("Room {}", room_id))
    .bind("standard")
    .bind(1_i32)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn seed_booking_for_origin_tests(
    pool: &Pool<Sqlite>,
    room_id: &str,
) -> BookingResult<String> {
    let guest_id = uuid::Uuid::new_v4().to_string();
    let booking_id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO guests (id, guest_type, full_name, doc_number, created_at)
         VALUES (?, 'domestic', 'Test Guest', 'DOC', '2026-04-27T08:00:00+07:00')",
    )
    .bind(&guest_id)
    .execute(pool)
    .await?;
    sqlx::query(
        "INSERT INTO bookings (
            id, room_id, primary_guest_id, check_in_at, expected_checkout,
            nights, total_price, paid_amount, status, created_at
         ) VALUES (?, ?, ?, '2026-04-27', '2026-04-28', 1, 250000, 0, 'active', '2026-04-27T08:00:00+07:00')",
    )
    .bind(&booking_id)
    .bind(room_id)
    .bind(&guest_id)
    .execute(pool)
    .await?;
    Ok(booking_id)
}

pub async fn seed_pricing_rule(
    pool: &Pool<Sqlite>,
    room_type: &str,
    daily_rate: MoneyVnd,
) -> BookingResult<()> {
    let now = "2026-04-15T10:00:00+07:00";

    sqlx::query(
        "INSERT INTO pricing_rules (
            id, room_type, hourly_rate, overnight_rate, daily_rate,
            overnight_start, overnight_end, daily_checkin, daily_checkout,
            early_checkin_surcharge_pct, late_checkout_surcharge_pct,
            weekend_uplift_pct, created_at, updated_at
        ) VALUES (?, ?, 0, 0, ?, '22:00', '11:00', '14:00', '12:00', 0, 0, 0, ?, ?)",
    )
    .bind(format!("rule-{}", room_type))
    .bind(room_type)
    .bind(daily_rate)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn seed_pricing_rule_tx(
    tx: &mut Transaction<'_, Sqlite>,
    room_type: &str,
    daily_rate: MoneyVnd,
) -> BookingResult<()> {
    let now = "2026-04-15T10:00:00+07:00";

    sqlx::query(
        "INSERT INTO pricing_rules (
            id, room_type, hourly_rate, overnight_rate, daily_rate,
            overnight_start, overnight_end, daily_checkin, daily_checkout,
            early_checkin_surcharge_pct, late_checkout_surcharge_pct,
            weekend_uplift_pct, created_at, updated_at
        ) VALUES (?, ?, 0, 0, ?, '22:00', '11:00', '14:00', '12:00', 0, 0, 0, ?, ?)",
    )
    .bind(format!("rule-{}", room_type))
    .bind(room_type)
    .bind(daily_rate)
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn seed_special_date(
    pool: &Pool<Sqlite>,
    date: &str,
    uplift_pct: f64,
) -> BookingResult<()> {
    let now = "2026-04-15T10:00:00+07:00";

    sqlx::query(
        "INSERT INTO special_dates (id, date, label, uplift_pct, created_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(format!("special-date-{}", date))
    .bind(date)
    .bind("Holiday uplift")
    .bind(uplift_pct)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn seed_special_date_tx(
    tx: &mut Transaction<'_, Sqlite>,
    date: &str,
    uplift_pct: f64,
) -> BookingResult<()> {
    let now = "2026-04-15T10:00:00+07:00";

    sqlx::query(
        "INSERT INTO special_dates (id, date, label, uplift_pct, created_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(format!("special-date-{}", date))
    .bind(date)
    .bind("Holiday uplift")
    .bind(uplift_pct)
    .bind(now)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

pub async fn seed_active_booking(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    room_id: &str,
) -> BookingResult<()> {
    let guest_id = format!("guest-{}", booking_id);
    let now = "2026-04-15T10:00:00+07:00";

    sqlx::query(
        "INSERT INTO guests (
            id, guest_type, full_name, doc_number, dob, gender, nationality,
            address, visa_expiry, scan_path, phone, created_at
        ) VALUES (?, 'domestic', ?, ?, NULL, NULL, NULL, NULL, NULL, NULL, NULL, ?)",
    )
    .bind(&guest_id)
    .bind(format!("Guest {}", booking_id))
    .bind(format!("DOC-{}", booking_id))
    .bind(now)
    .execute(pool)
    .await?;

    sqlx::query(
        "INSERT INTO bookings (
            id, room_id, primary_guest_id, check_in_at, expected_checkout,
            actual_checkout, nights, total_price, paid_amount, status,
            source, notes, created_by, booking_type, pricing_type, pricing_snapshot, created_at
        ) VALUES (?, ?, ?, ?, ?, NULL, ?, ?, NULL, 'active', ?, ?, ?, 'walk-in', 'nightly', NULL, ?)",
    )
    .bind(booking_id)
    .bind(room_id)
    .bind(&guest_id)
    .bind(now)
    .bind("2026-04-16T10:00:00+07:00")
    .bind(1_i64)
    .bind(250_000)
    .bind("walk-in")
    .bind("seed booking")
    .bind("seed-user")
    .bind(now)
    .execute(pool)
    .await?;

    sqlx::query("INSERT INTO booking_guests (booking_id, guest_id) VALUES (?, ?)")
        .bind(booking_id)
        .bind(&guest_id)
        .execute(pool)
        .await?;

    sqlx::query("UPDATE rooms SET status = 'occupied' WHERE id = ?")
        .bind(room_id)
        .execute(pool)
        .await?;

    sqlx::query(
        "INSERT INTO room_calendar (room_id, date, booking_id, status) VALUES (?, '2026-04-15', ?, 'occupied')",
    )
    .bind(room_id)
    .bind(booking_id)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn seed_active_booking_with_room(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    room_id: &str,
) -> BookingResult<()> {
    seed_room(pool, room_id).await?;
    seed_active_booking(pool, booking_id, room_id).await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub async fn seed_active_booking_with_terms(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    room_id: &str,
    check_in_at: &str,
    expected_checkout: &str,
    nights: i64,
    total_price: MoneyVnd,
    paid_amount: Option<i64>,
) -> BookingResult<()> {
    seed_active_booking(pool, booking_id, room_id).await?;

    sqlx::query(
        "UPDATE bookings
         SET check_in_at = ?, expected_checkout = ?, nights = ?, total_price = ?, paid_amount = ?
         WHERE id = ?",
    )
    .bind(check_in_at)
    .bind(expected_checkout)
    .bind(nights)
    .bind(total_price)
    .bind(paid_amount)
    .bind(booking_id)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn seed_booked_reservation(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    room_id: &str,
) -> BookingResult<()> {
    let guest_id = format!("guest-{}", booking_id);
    let guest_name = format!("Reserved Guest {}", booking_id);
    let now = "2026-04-15T10:00:00+07:00";
    let phone = "0901234567";
    let check_in = "2026-04-20";
    let check_out = "2026-04-22";
    let nights = 2_i64;
    let deposit = 50_000;
    let total_price = 500_000;

    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO guests (
            id, guest_type, full_name, doc_number, dob, gender, nationality,
            address, visa_expiry, scan_path, phone, created_at
        ) VALUES (?, 'domestic', ?, ?, NULL, NULL, NULL, NULL, NULL, NULL, ?, ?)",
    )
    .bind(&guest_id)
    .bind(&guest_name)
    .bind(format!("DOC-{}", booking_id))
    .bind(phone)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO bookings (
            id, room_id, primary_guest_id, check_in_at, expected_checkout,
            actual_checkout, nights, total_price, paid_amount, status,
            source, notes, created_by, booking_type, pricing_type,
            deposit_amount, guest_phone, scheduled_checkin, scheduled_checkout,
            pricing_snapshot, created_at
        ) VALUES (?, ?, ?, ?, ?, NULL, ?, ?, ?, 'booked', ?, ?, NULL, 'reservation', 'nightly', ?, ?, ?, ?, NULL, ?)",
    )
    .bind(booking_id)
    .bind(room_id)
    .bind(&guest_id)
    .bind(check_in)
    .bind(check_out)
    .bind(nights)
    .bind(total_price)
    .bind(deposit)
    .bind("phone")
    .bind("seed reservation")
    .bind(deposit)
    .bind(phone)
    .bind(check_in)
    .bind(check_out)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    sqlx::query("INSERT INTO booking_guests (booking_id, guest_id) VALUES (?, ?)")
        .bind(booking_id)
        .bind(&guest_id)
        .execute(&mut *tx)
        .await?;

    sqlx::query(
        "INSERT INTO room_calendar (room_id, date, booking_id, status) VALUES (?, '2026-04-20', ?, 'booked')",
    )
    .bind(room_id)
    .bind(booking_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "INSERT INTO room_calendar (room_id, date, booking_id, status) VALUES (?, '2026-04-21', ?, 'booked')",
    )
    .bind(room_id)
    .bind(booking_id)
    .execute(&mut *tx)
    .await?;

    if deposit > 0 {
        sqlx::query(
            "INSERT INTO transactions (id, booking_id, amount, type, note, created_at)
             VALUES (?, ?, ?, 'deposit', ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(booking_id)
        .bind(deposit)
        .bind("Reservation deposit")
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;

    Ok(())
}

pub async fn seed_transaction(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    amount: MoneyVnd,
    txn_type: &str,
    note: &str,
    created_at: &str,
) -> BookingResult<()> {
    sqlx::query(
        "INSERT INTO transactions (id, booking_id, amount, type, note, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(booking_id)
    .bind(amount)
    .bind(txn_type)
    .bind(note)
    .bind(created_at)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn seed_folio_line(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    amount: MoneyVnd,
    created_at: &str,
) -> BookingResult<()> {
    sqlx::query(
        "INSERT INTO folio_lines (id, booking_id, category, description, amount, created_by, created_at)
         VALUES (?, ?, 'mini-bar', 'Seed folio', ?, 'seed-user', ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(booking_id)
    .bind(amount)
    .bind(created_at)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn seed_expense(
    pool: &Pool<Sqlite>,
    category: &str,
    amount: MoneyVnd,
    expense_date: &str,
) -> BookingResult<()> {
    sqlx::query(
        "INSERT INTO expenses (id, category, amount, note, expense_date, created_at)
         VALUES (?, ?, ?, 'Seed expense', ?, ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(category)
    .bind(amount)
    .bind(expense_date)
    .bind(format!("{}T22:00:00+07:00", expense_date))
    .execute(pool)
    .await?;

    Ok(())
}

#[allow(dead_code)]
pub async fn seed_rooms(pool: &Pool<Sqlite>, room_ids: &[&str]) -> BookingResult<()> {
    for room_id in room_ids {
        seed_room(pool, room_id).await?;
    }
    Ok(())
}

/// Inserts one room and the default `standard` pricing rule; use `seed_rooms_with_price`
/// when seeding multiple standard rooms in the same pool.
#[allow(dead_code)]
pub async fn seed_room_with_price(
    pool: &Pool<Sqlite>,
    room_id: &str,
    daily_rate: MoneyVnd,
) -> BookingResult<()> {
    seed_room(pool, room_id).await?;
    seed_pricing_rule(pool, "standard", daily_rate).await?;
    Ok(())
}

pub async fn seed_rooms_with_price(
    pool: &Pool<Sqlite>,
    room_ids: &[&str],
    daily_rate: MoneyVnd,
) -> BookingResult<()> {
    seed_rooms(pool, room_ids).await?;
    seed_pricing_rule(pool, "standard", daily_rate).await?;
    Ok(())
}
