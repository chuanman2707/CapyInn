use sqlx::{sqlite::SqlitePoolOptions, Pool, Row, Sqlite};

use crate::{
    domain::booking::{pricing::calculate_stay_price_tx, BookingResult},
    models::{CheckInRequest, CheckOutRequest, CreateGuestRequest, CreateReservationRequest},
};

use super::{
    billing_service::{
        record_cancellation_fee_tx, record_deposit_tx, record_payment, record_payment_tx,
    },
    stay_lifecycle,
};

pub async fn test_pool() -> Pool<Sqlite> {
    let database_url = format!(
        "sqlite://file:{}?mode=memory&cache=shared",
        uuid::Uuid::new_v4()
    );

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("failed to open sqlite test pool");

    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("failed to enable foreign keys");

    sqlx::query(
        "CREATE TABLE rooms (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            type TEXT NOT NULL,
            floor INTEGER NOT NULL,
            has_balcony INTEGER NOT NULL DEFAULT 0,
            base_price REAL NOT NULL DEFAULT 0,
            max_guests INTEGER NOT NULL DEFAULT 2,
            extra_person_fee REAL NOT NULL DEFAULT 0,
            status TEXT NOT NULL DEFAULT 'vacant'
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create rooms table");

    sqlx::query(
        "CREATE TABLE guests (
            id TEXT PRIMARY KEY,
            guest_type TEXT NOT NULL DEFAULT 'domestic',
            full_name TEXT NOT NULL,
            doc_number TEXT NOT NULL,
            dob TEXT,
            gender TEXT,
            nationality TEXT,
            address TEXT,
            visa_expiry TEXT,
            scan_path TEXT,
            phone TEXT,
            created_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create guests table");

    sqlx::query(
        "CREATE TABLE bookings (
            id TEXT PRIMARY KEY,
            room_id TEXT NOT NULL REFERENCES rooms(id),
            primary_guest_id TEXT NOT NULL REFERENCES guests(id),
            check_in_at TEXT NOT NULL,
            expected_checkout TEXT NOT NULL,
            actual_checkout TEXT,
            nights INTEGER NOT NULL,
            total_price REAL NOT NULL,
            paid_amount REAL,
            status TEXT NOT NULL,
            source TEXT,
            notes TEXT,
            created_by TEXT,
            booking_type TEXT DEFAULT 'walk-in',
            pricing_type TEXT DEFAULT 'nightly',
            deposit_amount REAL,
            guest_phone TEXT,
            scheduled_checkin TEXT,
            scheduled_checkout TEXT,
            pricing_snapshot TEXT,
            created_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create bookings table");

    sqlx::query(
        "CREATE TABLE booking_guests (
            booking_id TEXT NOT NULL REFERENCES bookings(id),
            guest_id TEXT NOT NULL REFERENCES guests(id),
            PRIMARY KEY (booking_id, guest_id)
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create booking_guests table");

    sqlx::query(
        "CREATE TABLE transactions (
            id TEXT PRIMARY KEY,
            booking_id TEXT NOT NULL REFERENCES bookings(id),
            amount REAL NOT NULL,
            type TEXT NOT NULL,
            note TEXT,
            created_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create transactions table");

    sqlx::query(
        "CREATE TABLE housekeeping (
            id TEXT PRIMARY KEY,
            room_id TEXT NOT NULL REFERENCES rooms(id),
            status TEXT NOT NULL DEFAULT 'needs_cleaning',
            note TEXT,
            triggered_at TEXT NOT NULL,
            cleaned_at TEXT,
            created_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create housekeeping table");

    sqlx::query(
        "CREATE TABLE room_calendar (
            room_id TEXT NOT NULL REFERENCES rooms(id),
            date TEXT NOT NULL,
            booking_id TEXT REFERENCES bookings(id),
            status TEXT NOT NULL DEFAULT 'booked',
            PRIMARY KEY (room_id, date)
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create room_calendar table");

    sqlx::query(
        "CREATE TABLE pricing_rules (
            id TEXT PRIMARY KEY,
            room_type TEXT NOT NULL UNIQUE,
            hourly_rate REAL NOT NULL DEFAULT 0,
            overnight_rate REAL NOT NULL DEFAULT 0,
            daily_rate REAL NOT NULL DEFAULT 0,
            overnight_start TEXT NOT NULL DEFAULT '22:00',
            overnight_end TEXT NOT NULL DEFAULT '11:00',
            daily_checkin TEXT NOT NULL DEFAULT '14:00',
            daily_checkout TEXT NOT NULL DEFAULT '12:00',
            early_checkin_surcharge_pct REAL NOT NULL DEFAULT 30,
            late_checkout_surcharge_pct REAL NOT NULL DEFAULT 30,
            weekend_uplift_pct REAL NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create pricing_rules table");

    sqlx::query(
        "CREATE TABLE special_dates (
            id TEXT PRIMARY KEY,
            date TEXT NOT NULL UNIQUE,
            label TEXT NOT NULL DEFAULT '',
            uplift_pct REAL NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create special_dates table");

    pool
}

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

pub async fn seed_pricing_rule(
    pool: &Pool<Sqlite>,
    room_type: &str,
    daily_rate: f64,
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

pub async fn seed_active_booking(pool: &Pool<Sqlite>, booking_id: &str, room_id: &str) -> BookingResult<()> {
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
    .bind(250_000.0_f64)
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
    let deposit = 50_000.0_f64;
    let total_price = 500_000.0_f64;

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
    .execute(pool)
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
    .execute(pool)
    .await?;

    sqlx::query("INSERT INTO booking_guests (booking_id, guest_id) VALUES (?, ?)")
        .bind(booking_id)
        .bind(&guest_id)
        .execute(pool)
        .await?;

    sqlx::query("UPDATE rooms SET status = 'booked' WHERE id = ?")
        .bind(room_id)
        .execute(pool)
        .await?;

    sqlx::query(
        "INSERT INTO room_calendar (room_id, date, booking_id, status) VALUES (?, '2026-04-20', ?, 'booked')",
    )
    .bind(room_id)
    .bind(booking_id)
    .execute(pool)
    .await?;

    sqlx::query(
        "INSERT INTO room_calendar (room_id, date, booking_id, status) VALUES (?, '2026-04-21', ?, 'booked')",
    )
    .bind(room_id)
    .bind(booking_id)
    .execute(pool)
    .await?;

    Ok(())
}

pub fn minimal_checkin_request(room_id: &str) -> CheckInRequest {
    CheckInRequest {
        room_id: room_id.to_string(),
        guests: vec![CreateGuestRequest {
            guest_type: Some("domestic".to_string()),
            full_name: "Nguyen Van A".to_string(),
            doc_number: "079123456789".to_string(),
            dob: None,
            gender: None,
            nationality: Some("VN".to_string()),
            address: None,
            visa_expiry: None,
            scan_path: None,
            phone: Some("0900000000".to_string()),
        }],
        nights: 2,
        source: Some("walk-in".to_string()),
        notes: Some("test check-in".to_string()),
        paid_amount: None,
        pricing_type: Some("nightly".to_string()),
    }
}

pub fn minimal_reservation_request(room_id: &str) -> CreateReservationRequest {
    CreateReservationRequest {
        room_id: room_id.to_string(),
        guest_name: "Nguyen Van B".to_string(),
        guest_phone: Some("0900000001".to_string()),
        guest_doc_number: Some("079000000001".to_string()),
        check_in_date: "2026-04-20".to_string(),
        check_out_date: "2026-04-22".to_string(),
        nights: 2,
        deposit_amount: Some(50_000.0),
        source: Some("phone".to_string()),
        notes: Some("test reservation".to_string()),
    }
}

#[tokio::test]
async fn record_payment_updates_paid_amount_cache() {
    let pool = test_pool().await;
    seed_room(&pool, "R101").await.unwrap();
    seed_active_booking(&pool, "B101", "R101").await.unwrap();

    record_payment(&pool, "B101", 25_000.0, "deposit")
        .await
        .unwrap();

    let booking = sqlx::query("SELECT paid_amount FROM bookings WHERE id = ?")
        .bind("B101")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(booking.get::<Option<f64>, _>("paid_amount"), Some(25_000.0));

    let txn = sqlx::query("SELECT type, amount, note FROM transactions WHERE booking_id = ?")
        .bind("B101")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(txn.get::<String, _>("type"), "payment");
    assert_eq!(txn.get::<f64, _>("amount"), 25_000.0);
    assert_eq!(txn.get::<String, _>("note"), "deposit");
}

#[tokio::test]
async fn record_payment_tx_can_compose_inside_outer_transaction() {
    let pool = test_pool().await;
    seed_room(&pool, "R102").await.unwrap();
    seed_active_booking(&pool, "B102", "R102").await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    record_payment_tx(&mut tx, "B102", 12_500.0, "deposit")
        .await
        .unwrap();

    let booking = sqlx::query("SELECT paid_amount FROM bookings WHERE id = ?")
        .bind("B102")
        .fetch_one(&mut *tx)
        .await
        .unwrap();

    assert_eq!(booking.get::<Option<f64>, _>("paid_amount"), Some(12_500.0));

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn record_deposit_tx_updates_paid_amount_cache() {
    let pool = test_pool().await;
    seed_room(&pool, "R103").await.unwrap();
    seed_booked_reservation(&pool, "B103", "R103").await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    record_deposit_tx(&mut tx, "B103", 25_000.0, "extra deposit")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let booking = sqlx::query("SELECT paid_amount FROM bookings WHERE id = ?")
        .bind("B103")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(booking.get::<Option<f64>, _>("paid_amount"), Some(75_000.0));

    let txn = sqlx::query(
        "SELECT type, amount, note FROM transactions WHERE booking_id = ? AND note = ?",
    )
    .bind("B103")
    .bind("extra deposit")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(txn.get::<String, _>("type"), "deposit");
    assert_eq!(txn.get::<f64, _>("amount"), 25_000.0);
    assert_eq!(txn.get::<String, _>("note"), "extra deposit");
}

#[tokio::test]
async fn record_cancellation_fee_tx_does_not_change_paid_amount() {
    let pool = test_pool().await;
    seed_room(&pool, "R104").await.unwrap();
    seed_booked_reservation(&pool, "B104", "R104").await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    record_cancellation_fee_tx(&mut tx, "B104", 25_000.0, "retained deposit")
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let booking = sqlx::query("SELECT paid_amount FROM bookings WHERE id = ?")
        .bind("B104")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(booking.get::<Option<f64>, _>("paid_amount"), Some(50_000.0));

    let txn = sqlx::query(
        "SELECT type, amount, note FROM transactions WHERE booking_id = ? AND note = ?",
    )
    .bind("B104")
    .bind("retained deposit")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(txn.get::<String, _>("type"), "cancellation_fee");
    assert_eq!(txn.get::<f64, _>("amount"), 25_000.0);
    assert_eq!(txn.get::<String, _>("note"), "retained deposit");
}

#[tokio::test]
async fn calculate_stay_price_tx_reads_uncommitted_pricing_rule() {
    let pool = test_pool().await;
    seed_room(&pool, "R150").await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO pricing_rules (
            id, room_type, hourly_rate, overnight_rate, daily_rate,
            overnight_start, overnight_end, daily_checkin, daily_checkout,
            early_checkin_surcharge_pct, late_checkout_surcharge_pct,
            weekend_uplift_pct, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, '22:00', '11:00', '14:00', '12:00', 0, 0, 0, ?, ?)",
    )
    .bind("rule-standard")
    .bind("standard")
    .bind(50_000.0_f64)
    .bind(150_000.0_f64)
    .bind(600_000.0_f64)
    .bind("2026-04-15T10:00:00+07:00")
    .bind("2026-04-15T10:00:00+07:00")
    .execute(&mut *tx)
    .await
    .unwrap();

    let pricing = calculate_stay_price_tx(
        &mut tx,
        "R150",
        "2026-04-15T10:00:00+07:00",
        "2026-04-17T10:00:00+07:00",
        "nightly",
    )
    .await
    .unwrap();

    assert_eq!(pricing.total, 1_200_000.0);

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn check_in_posts_charge_and_marks_room_occupied() {
    let pool = test_pool().await;
    seed_room(&pool, "R201").await.unwrap();

    let booking = stay_lifecycle::check_in(
        &pool,
        minimal_checkin_request("R201"),
        Some("user-1".to_string()),
    )
    .await
    .unwrap();

    let room = sqlx::query("SELECT status FROM rooms WHERE id = ?")
        .bind("R201")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(room.get::<String, _>("status"), "occupied");

    let charge = sqlx::query(
        "SELECT type, amount FROM transactions WHERE booking_id = ? AND type = 'charge' LIMIT 1",
    )
    .bind(&booking.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(charge.get::<String, _>("type"), "charge");
    assert_eq!(charge.get::<f64, _>("amount"), booking.total_price);

    let calendar_days: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM room_calendar WHERE booking_id = ? AND status = 'occupied'",
    )
    .bind(&booking.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(calendar_days.0, 2);
}

#[tokio::test]
async fn check_out_allows_outstanding_balance_if_final_paid_absent() {
    let pool = test_pool().await;
    seed_room(&pool, "R202").await.unwrap();
    seed_active_booking(&pool, "B202", "R202").await.unwrap();

    stay_lifecycle::check_out(
        &pool,
        CheckOutRequest {
            booking_id: "B202".to_string(),
            final_paid: None,
        },
    )
    .await
    .unwrap();

    let booking = sqlx::query(
        "SELECT status, actual_checkout, paid_amount FROM bookings WHERE id = ?",
    )
    .bind("B202")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(booking.get::<String, _>("status"), "checked_out");
    assert!(booking.get::<Option<String>, _>("actual_checkout").is_some());
    assert_eq!(booking.get::<Option<f64>, _>("paid_amount"), None);

    let payment_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND type = 'payment'")
            .bind("B202")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(payment_count.0, 0);

    let room = sqlx::query("SELECT status FROM rooms WHERE id = ?")
        .bind("R202")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(room.get::<String, _>("status"), "cleaning");

    let housekeeping_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM housekeeping WHERE room_id = ?")
            .bind("R202")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(housekeeping_count.0, 1);

    let calendar_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM room_calendar WHERE booking_id = ?",
    )
    .bind("B202")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(calendar_count.0, 0);
}

#[tokio::test]
async fn extend_stay_uses_existing_expected_checkout() {
    let pool = test_pool().await;
    seed_room(&pool, "R203").await.unwrap();
    seed_active_booking(&pool, "B203", "R203").await.unwrap();

    let booking = stay_lifecycle::extend_stay(&pool, "B203").await.unwrap();

    assert_eq!(booking.nights, 2);
    assert_eq!(booking.expected_checkout, "2026-04-17T10:00:00+07:00");
    assert_eq!(booking.total_price, 500_000.0);

    let extended_day = sqlx::query(
        "SELECT status FROM room_calendar WHERE room_id = ? AND date = '2026-04-16'",
    )
    .bind("R203")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(extended_day.get::<String, _>("status"), "occupied");

    let charge = sqlx::query(
        "SELECT amount FROM transactions WHERE booking_id = ? AND note = 'Extended stay +1 night'",
    )
    .bind("B203")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(charge.get::<f64, _>("amount"), 250_000.0);
}
