use chrono::{Duration, Local};
use sqlx::{sqlite::SqlitePoolOptions, Pool, Row, Sqlite, Transaction};

use crate::{
    commands::reservations,
    domain::booking::{
        pricing::{calculate_stay_price, calculate_stay_price_tx},
        BookingError, BookingResult,
    },
    models::{
        CheckInRequest, CheckOutRequest, CreateGuestRequest, CreateReservationRequest,
        GroupCheckinRequest, GroupCheckoutRequest,
    },
    queries::booking::{audit_queries, billing_queries, revenue_queries},
};

use super::{
    audit_service,
    billing_service::{
        add_folio_line, record_cancellation_fee_tx, record_deposit_tx, record_payment,
        record_payment_tx,
    },
    group_lifecycle, guest_service, reservation_lifecycle, stay_lifecycle,
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
            group_id TEXT REFERENCES booking_groups(id),
            is_master_room INTEGER NOT NULL DEFAULT 0,
            is_audited INTEGER NOT NULL DEFAULT 0,
            pricing_snapshot TEXT,
            created_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create bookings table");

    sqlx::query(
        "CREATE TABLE booking_groups (
            id TEXT PRIMARY KEY,
            group_name TEXT NOT NULL,
            master_booking_id TEXT,
            organizer_name TEXT NOT NULL,
            organizer_phone TEXT,
            total_rooms INTEGER NOT NULL,
            status TEXT NOT NULL DEFAULT 'active',
            notes TEXT,
            created_by TEXT,
            created_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create booking_groups table");

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

    sqlx::query(
        "CREATE TABLE expenses (
            id TEXT PRIMARY KEY,
            category TEXT NOT NULL,
            amount REAL NOT NULL,
            note TEXT,
            expense_date TEXT NOT NULL,
            created_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create expenses table");

    sqlx::query(
        "CREATE TABLE folio_lines (
            id TEXT PRIMARY KEY,
            booking_id TEXT NOT NULL REFERENCES bookings(id),
            category TEXT NOT NULL,
            description TEXT NOT NULL,
            amount REAL NOT NULL,
            created_by TEXT,
            created_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create folio_lines table");

    sqlx::query(
        "CREATE TABLE night_audit_logs (
            id TEXT PRIMARY KEY,
            audit_date TEXT NOT NULL UNIQUE,
            total_revenue REAL NOT NULL DEFAULT 0,
            room_revenue REAL NOT NULL DEFAULT 0,
            folio_revenue REAL NOT NULL DEFAULT 0,
            total_expenses REAL NOT NULL DEFAULT 0,
            occupancy_pct REAL NOT NULL DEFAULT 0,
            rooms_sold INTEGER NOT NULL DEFAULT 0,
            total_rooms INTEGER NOT NULL DEFAULT 0,
            notes TEXT,
            created_by TEXT,
            created_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create night_audit_logs table");

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

pub async fn seed_pricing_rule_tx(
    tx: &mut Transaction<'_, Sqlite>,
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

    if deposit > 0.0 {
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

pub async fn seed_transaction(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    amount: f64,
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
    amount: f64,
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
    amount: f64,
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

pub fn minimal_group_checkin_request(room_ids: &[&str]) -> GroupCheckinRequest {
    let mut guests_per_room = std::collections::HashMap::new();
    if let Some(first_room) = room_ids.first() {
        guests_per_room.insert(
            (*first_room).to_string(),
            vec![CreateGuestRequest {
                guest_type: Some("domestic".to_string()),
                full_name: "Group Guest 1".to_string(),
                doc_number: "079111111111".to_string(),
                dob: None,
                gender: None,
                nationality: Some("VN".to_string()),
                address: None,
                visa_expiry: None,
                scan_path: None,
                phone: Some("0901111111".to_string()),
            }],
        );
    }

    GroupCheckinRequest {
        group_name: "Test Group".to_string(),
        organizer_name: "Organizer".to_string(),
        organizer_phone: Some("0902222222".to_string()),
        check_in_date: None,
        room_ids: room_ids
            .iter()
            .map(|room_id| (*room_id).to_string())
            .collect(),
        master_room_id: room_ids[0].to_string(),
        guests_per_room,
        nights: 2,
        source: Some("walk-in".to_string()),
        notes: Some("group test".to_string()),
        paid_amount: Some(100_000.0),
    }
}

#[tokio::test]
async fn create_guest_manifest_persists_primary_and_additional_guests() {
    let pool = test_pool().await;
    let mut request = minimal_checkin_request("R201");
    request.guests.push(CreateGuestRequest {
        guest_type: Some("foreign".to_string()),
        full_name: "Jane Doe".to_string(),
        doc_number: "P1234567".to_string(),
        dob: None,
        gender: Some("female".to_string()),
        nationality: Some("US".to_string()),
        address: Some("1 Test Street".to_string()),
        visa_expiry: None,
        scan_path: None,
        phone: Some("0909999999".to_string()),
    });

    let mut tx = pool.begin().await.unwrap();
    let manifest =
        guest_service::create_guest_manifest(&mut tx, &request.guests, "2026-04-15T10:00:00+07:00")
            .await
            .unwrap();

    assert_eq!(manifest.guest_ids.len(), 2);
    assert_eq!(manifest.primary_guest_id, manifest.guest_ids[0]);

    let rows = sqlx::query(
        "SELECT full_name, guest_type, doc_number, phone FROM guests ORDER BY full_name ASC",
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap();

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<String, _>("full_name"), "Jane Doe");
    assert_eq!(rows[0].get::<String, _>("guest_type"), "foreign");
    assert_eq!(rows[1].get::<String, _>("full_name"), "Nguyen Van A");
}

#[tokio::test]
async fn create_guest_manifest_rejects_empty_guest_list() {
    let pool = test_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let error = guest_service::create_guest_manifest(&mut tx, &[], "2026-04-15T10:00:00+07:00")
        .await
        .unwrap_err();

    assert_eq!(error.to_string(), "Phải có ít nhất 1 khách");
}

#[tokio::test]
async fn create_reservation_guest_manifest_defaults_blank_doc_number() {
    let pool = test_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let manifest = guest_service::create_reservation_guest_manifest(
        &mut tx,
        "Reservation Guest",
        None,
        Some("0901234567"),
        "2026-04-15T10:00:00+07:00",
    )
    .await
    .unwrap();

    let guest = sqlx::query("SELECT full_name, doc_number, phone FROM guests WHERE id = ?")
        .bind(&manifest.primary_guest_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();

    assert_eq!(manifest.guest_ids, vec![manifest.primary_guest_id.clone()]);
    assert_eq!(guest.get::<String, _>("full_name"), "Reservation Guest");
    assert_eq!(guest.get::<String, _>("doc_number"), "");
    assert_eq!(
        guest.get::<Option<String>, _>("phone"),
        Some("0901234567".to_string())
    );
}

#[tokio::test]
async fn group_checkin_creates_active_group_and_placeholder_guest_manifest() {
    let pool = test_pool().await;
    seed_room(&pool, "G101").await.unwrap();
    seed_room(&pool, "G102").await.unwrap();
    seed_pricing_rule(&pool, "standard", 250_000.0)
        .await
        .unwrap();

    let group = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        minimal_group_checkin_request(&["G101", "G102"]),
    )
    .await
    .unwrap();

    assert_eq!(group.status, "active");
    assert!(group.master_booking_id.is_some());

    let room_statuses =
        sqlx::query("SELECT id, status FROM rooms WHERE id IN ('G101', 'G102') ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(room_statuses[0].get::<String, _>("status"), "occupied");
    assert_eq!(room_statuses[1].get::<String, _>("status"), "occupied");

    let booking_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM bookings WHERE group_id = ? AND status = 'active'")
            .bind(&group.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(booking_count.0, 2);

    let paid_amounts = sqlx::query(
        "SELECT paid_amount, deposit_amount FROM bookings WHERE group_id = ? ORDER BY room_id",
    )
    .bind(&group.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(paid_amounts.len(), 2);
    assert_eq!(
        paid_amounts[0].get::<Option<f64>, _>("paid_amount"),
        Some(50_000.0)
    );
    assert_eq!(
        paid_amounts[1].get::<Option<f64>, _>("paid_amount"),
        Some(50_000.0)
    );
    assert_eq!(
        paid_amounts[0].get::<Option<f64>, _>("deposit_amount"),
        Some(0.0)
    );

    let placeholder = sqlx::query(
        "SELECT g.full_name, g.doc_number
         FROM guests g
         JOIN bookings b ON b.primary_guest_id = g.id
         WHERE b.group_id = ? AND b.room_id = 'G102'",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        placeholder.get::<String, _>("full_name"),
        "Khách đoàn Test Group - G102"
    );
    assert_eq!(placeholder.get::<String, _>("doc_number"), "");
}

#[tokio::test]
async fn group_checkin_reservation_blocks_calendar_and_tracks_deposit() {
    let pool = test_pool().await;
    seed_room(&pool, "G201").await.unwrap();
    seed_room(&pool, "G202").await.unwrap();
    seed_pricing_rule(&pool, "standard", 300_000.0)
        .await
        .unwrap();

    let mut req = minimal_group_checkin_request(&["G201", "G202"]);
    req.check_in_date = Some(
        (Local::now().date_naive() + Duration::days(2))
            .format("%Y-%m-%d")
            .to_string(),
    );
    req.paid_amount = Some(60_000.0);

    let group = group_lifecycle::group_checkin(&pool, None, req)
        .await
        .unwrap();

    assert_eq!(group.status, "booked");

    let room_statuses =
        sqlx::query("SELECT status FROM rooms WHERE id IN ('G201', 'G202') ORDER BY id")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(room_statuses[0].get::<String, _>("status"), "vacant");
    assert_eq!(room_statuses[1].get::<String, _>("status"), "vacant");

    let calendar_rows: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM room_calendar WHERE booking_id IN (SELECT id FROM bookings WHERE group_id = ?) AND status = 'booked'",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(calendar_rows.0, 4);

    let amounts = sqlx::query(
        "SELECT paid_amount, deposit_amount FROM bookings WHERE group_id = ? ORDER BY room_id",
    )
    .bind(&group.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        amounts[0].get::<Option<f64>, _>("paid_amount"),
        Some(30_000.0)
    );
    assert_eq!(
        amounts[0].get::<Option<f64>, _>("deposit_amount"),
        Some(30_000.0)
    );
}

#[tokio::test]
async fn group_checkin_rejects_duplicate_room_ids() {
    let pool = test_pool().await;
    seed_room(&pool, "G250").await.unwrap();
    seed_pricing_rule(&pool, "standard", 250_000.0)
        .await
        .unwrap();

    let error = group_lifecycle::group_checkin(
        &pool,
        None,
        minimal_group_checkin_request(&["G250", "G250"]),
    )
    .await
    .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Phòng không được lặp trong cùng một group"
    );
}

#[tokio::test]
async fn group_checkout_reassigns_master_and_updates_group_payment() {
    let pool = test_pool().await;
    seed_room(&pool, "G301").await.unwrap();
    seed_room(&pool, "G302").await.unwrap();
    seed_pricing_rule(&pool, "standard", 250_000.0)
        .await
        .unwrap();

    let group = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        minimal_group_checkin_request(&["G301", "G302"]),
    )
    .await
    .unwrap();

    let master_booking_id = group.master_booking_id.clone().unwrap();
    group_lifecycle::group_checkout(
        &pool,
        GroupCheckoutRequest {
            group_id: group.id.clone(),
            booking_ids: vec![master_booking_id.clone()],
            final_paid: Some(40_000.0),
        },
    )
    .await
    .unwrap();

    let group_row =
        sqlx::query("SELECT status, master_booking_id FROM booking_groups WHERE id = ?")
            .bind(&group.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(group_row.get::<String, _>("status"), "partial_checkout");
    assert_ne!(
        group_row.get::<Option<String>, _>("master_booking_id"),
        Some(master_booking_id.clone())
    );

    let checked_out = sqlx::query("SELECT status FROM bookings WHERE id = ?")
        .bind(&master_booking_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(checked_out.get::<String, _>("status"), "checked_out");

    let housekeeping_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM housekeeping WHERE room_id = 'G301'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(housekeeping_count.0, 1);

    let remaining_paid: (f64,) = sqlx::query_as(
        "SELECT paid_amount FROM bookings WHERE group_id = ? AND status = 'active' LIMIT 1",
    )
    .bind(&group.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(remaining_paid.0, 90_000.0);
}

#[tokio::test]
async fn group_checkout_clears_master_flag_when_group_completes() {
    let pool = test_pool().await;
    seed_room(&pool, "G401").await.unwrap();
    seed_pricing_rule(&pool, "standard", 250_000.0)
        .await
        .unwrap();

    let group = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        minimal_group_checkin_request(&["G401"]),
    )
    .await
    .unwrap();

    let master_booking_id = group.master_booking_id.clone().unwrap();
    group_lifecycle::group_checkout(
        &pool,
        GroupCheckoutRequest {
            group_id: group.id.clone(),
            booking_ids: vec![master_booking_id.clone()],
            final_paid: None,
        },
    )
    .await
    .unwrap();

    let group_row =
        sqlx::query("SELECT master_booking_id, status FROM booking_groups WHERE id = ?")
            .bind(&group.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        group_row.get::<Option<String>, _>("master_booking_id"),
        None
    );
    assert_eq!(group_row.get::<String, _>("status"), "completed");

    let booking_row = sqlx::query("SELECT is_master_room FROM bookings WHERE id = ?")
        .bind(&master_booking_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(booking_row.get::<i64, _>("is_master_room"), 0);
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
    seed_booked_reservation(&pool, "B103", "R103")
        .await
        .unwrap();

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
    seed_booked_reservation(&pool, "B104", "R104")
        .await
        .unwrap();

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
    seed_pricing_rule_tx(&mut tx, "standard", 600_000.0)
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
async fn calculate_stay_price_matches_tx_path_and_applies_special_date_uplift() {
    let pool = test_pool().await;
    seed_room(&pool, "R149").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();
    seed_special_date(&pool, "2026-04-20", 10.0).await.unwrap();

    let pool_pricing = calculate_stay_price(
        &pool,
        "R149",
        "2026-04-20T10:00:00+07:00",
        "2026-04-22T10:00:00+07:00",
        "nightly",
    )
    .await
    .unwrap();

    assert_eq!(pool_pricing.total, 1_320_000.0);
    assert_eq!(pool_pricing.base_amount, 1_200_000.0);
    assert_eq!(pool_pricing.surcharge_amount, 120_000.0);
    assert_eq!(pool_pricing.weekend_amount, 0.0);
    assert_eq!(pool_pricing.breakdown.len(), 2);
    assert_eq!(pool_pricing.breakdown[0].amount, 1_200_000.0);
    assert!(pool_pricing.breakdown[0].label.contains("night(s)"));
    assert!(pool_pricing
        .breakdown
        .iter()
        .any(|line| line.label == "Holiday surcharge"));

    let mut tx = pool.begin().await.unwrap();
    let tx_pricing = calculate_stay_price_tx(
        &mut tx,
        "R149",
        "2026-04-20T10:00:00+07:00",
        "2026-04-22T10:00:00+07:00",
        "nightly",
    )
    .await
    .unwrap();

    assert_eq!(tx_pricing.pricing_type, pool_pricing.pricing_type);
    assert_eq!(tx_pricing.base_amount, pool_pricing.base_amount);
    assert_eq!(tx_pricing.surcharge_amount, pool_pricing.surcharge_amount);
    assert_eq!(tx_pricing.weekend_amount, pool_pricing.weekend_amount);
    assert_eq!(tx_pricing.total, pool_pricing.total);

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn calculate_stay_price_tx_returns_not_found_for_missing_room() {
    let pool = test_pool().await;
    let mut tx = pool.begin().await.unwrap();

    let error = calculate_stay_price_tx(
        &mut tx,
        "missing-room",
        "2026-04-20T10:00:00+07:00",
        "2026-04-22T10:00:00+07:00",
        "nightly",
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        BookingError::NotFound(message) if message.contains("Không tìm thấy phòng missing-room")
    ));

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn calculate_stay_price_tx_returns_datetime_parse_for_invalid_check_in() {
    let pool = test_pool().await;
    seed_room(&pool, "R153").await.unwrap();
    let mut tx = pool.begin().await.unwrap();

    let error = calculate_stay_price_tx(
        &mut tx,
        "R153",
        "not-a-datetime",
        "2026-04-22T10:00:00+07:00",
        "nightly",
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        BookingError::DateTimeParse(message) if message.contains("Invalid check-in datetime")
    ));

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn calculate_stay_price_tx_reads_uncommitted_room_base_price() {
    let pool = test_pool().await;
    seed_room(&pool, "R151").await.unwrap();

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE rooms SET base_price = ? WHERE id = ?")
        .bind(600_000.0_f64)
        .bind("R151")
        .execute(&mut *tx)
        .await
        .unwrap();

    let pricing = calculate_stay_price_tx(
        &mut tx,
        "R151",
        "2026-04-20T10:00:00+07:00",
        "2026-04-22T10:00:00+07:00",
        "nightly",
    )
    .await
    .unwrap();

    assert_eq!(pricing.total, 1_200_000.0);

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn calculate_stay_price_tx_reads_uncommitted_special_date() {
    let pool = test_pool().await;
    seed_room(&pool, "R152").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();

    let mut tx = pool.begin().await.unwrap();
    seed_special_date_tx(&mut tx, "2026-04-20", 10.0)
        .await
        .unwrap();

    let pricing = calculate_stay_price_tx(
        &mut tx,
        "R152",
        "2026-04-20T10:00:00+07:00",
        "2026-04-22T10:00:00+07:00",
        "nightly",
    )
    .await
    .unwrap();

    assert_eq!(pricing.total, 1_320_000.0);

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn calculate_stay_price_returns_not_found_for_missing_room() {
    let pool = test_pool().await;

    let error = calculate_stay_price(
        &pool,
        "missing-room",
        "2026-04-20T10:00:00+07:00",
        "2026-04-22T10:00:00+07:00",
        "nightly",
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        BookingError::NotFound(message) if message.contains("Không tìm thấy phòng missing-room")
    ));
}

#[tokio::test]
async fn calculate_stay_price_returns_datetime_parse_for_invalid_check_in() {
    let pool = test_pool().await;
    seed_room(&pool, "R153").await.unwrap();

    let error = calculate_stay_price(
        &pool,
        "R153",
        "not-a-datetime",
        "2026-04-22T10:00:00+07:00",
        "nightly",
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        BookingError::DateTimeParse(message) if message.contains("Invalid check-in datetime")
    ));
}

#[tokio::test]
async fn create_reservation_blocks_calendar_and_posts_deposit() {
    let pool = test_pool().await;
    seed_room(&pool, "R160").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();

    let booking =
        reservation_lifecycle::create_reservation(&pool, minimal_reservation_request("R160"))
            .await
            .unwrap();

    assert_eq!(booking.room_id, "R160");
    assert_eq!(booking.status, "booked");
    assert_eq!(booking.total_price, 1_200_000.0);
    assert_eq!(booking.paid_amount, 50_000.0);

    let calendar_days: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM room_calendar WHERE booking_id = ? AND status = 'booked'",
    )
    .bind(&booking.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(calendar_days.0, 2);

    let room = sqlx::query("SELECT status FROM rooms WHERE id = ?")
        .bind("R160")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(room.get::<String, _>("status"), "vacant");

    let deposit = sqlx::query(
        "SELECT type, amount, note FROM transactions WHERE booking_id = ? AND type = 'deposit' LIMIT 1",
    )
    .bind(&booking.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(deposit.get::<String, _>("type"), "deposit");
    assert_eq!(deposit.get::<f64, _>("amount"), 50_000.0);
    assert_eq!(deposit.get::<String, _>("note"), "Reservation deposit");
}

#[tokio::test]
async fn create_reservation_rejects_inconsistent_nights_input() {
    let pool = test_pool().await;
    seed_room(&pool, "R160A").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();

    let error = reservation_lifecycle::create_reservation(
        &pool,
        CreateReservationRequest {
            room_id: "R160A".to_string(),
            guest_name: "Nguyen Van B".to_string(),
            guest_phone: Some("0900000001".to_string()),
            guest_doc_number: Some("079000000001".to_string()),
            check_in_date: "2026-04-20".to_string(),
            check_out_date: "2026-04-22".to_string(),
            nights: 3,
            deposit_amount: Some(50_000.0),
            source: Some("phone".to_string()),
            notes: Some("test reservation".to_string()),
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        crate::domain::booking::BookingError::Validation(_)
    ));
}

#[tokio::test]
async fn cancel_reservation_releases_calendar_and_keeps_fee_record() {
    let pool = test_pool().await;
    seed_room(&pool, "R161").await.unwrap();
    seed_booked_reservation(&pool, "B161", "R161")
        .await
        .unwrap();

    sqlx::query("UPDATE rooms SET status = 'booked' WHERE id = ?")
        .bind("R161")
        .execute(&pool)
        .await
        .unwrap();

    reservation_lifecycle::cancel_reservation(&pool, "B161")
        .await
        .unwrap();

    let booking = sqlx::query("SELECT status, paid_amount FROM bookings WHERE id = ?")
        .bind("B161")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(booking.get::<String, _>("status"), "cancelled");
    assert_eq!(booking.get::<Option<f64>, _>("paid_amount"), Some(50_000.0));

    let remaining_calendar: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM room_calendar WHERE booking_id = ?")
            .bind("B161")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(remaining_calendar.0, 0);

    let room = sqlx::query("SELECT status FROM rooms WHERE id = ?")
        .bind("R161")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(room.get::<String, _>("status"), "vacant");

    let fee = sqlx::query(
        "SELECT type, amount, note FROM transactions WHERE booking_id = ? AND type = 'cancellation_fee' LIMIT 1",
    )
    .bind("B161")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(fee.get::<String, _>("type"), "cancellation_fee");
    assert_eq!(fee.get::<f64, _>("amount"), 50_000.0);
    assert_eq!(
        fee.get::<String, _>("note"),
        "Deposit retained (cancellation)"
    );
}

#[tokio::test]
async fn do_create_reservation_returns_service_booking_and_leaves_room_vacant() {
    let pool = test_pool().await;
    seed_room(&pool, "R162").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();

    let booking =
        reservations::do_create_reservation(&pool, None, minimal_reservation_request("R162"))
            .await
            .unwrap();

    assert_eq!(booking.room_id, "R162");
    assert_eq!(booking.status, "booked");
    assert_eq!(booking.total_price, 1_200_000.0);
    assert_eq!(booking.paid_amount, 50_000.0);

    let room = sqlx::query("SELECT status FROM rooms WHERE id = ?")
        .bind("R162")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(room.get::<String, _>("status"), "vacant");

    let calendar_days: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM room_calendar WHERE booking_id = ? AND status = 'booked'",
    )
    .bind(&booking.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(calendar_days.0, 2);
}

#[tokio::test]
async fn do_cancel_reservation_cleans_legacy_booked_room_state() {
    let pool = test_pool().await;
    seed_room(&pool, "R163").await.unwrap();
    seed_booked_reservation(&pool, "B163", "R163")
        .await
        .unwrap();

    sqlx::query("UPDATE rooms SET status = 'booked' WHERE id = ?")
        .bind("R163")
        .execute(&pool)
        .await
        .unwrap();

    reservations::do_cancel_reservation(&pool, None, "B163")
        .await
        .unwrap();

    let room = sqlx::query("SELECT status FROM rooms WHERE id = ?")
        .bind("R163")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(room.get::<String, _>("status"), "vacant");

    let remaining_calendar: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM room_calendar WHERE booking_id = ?")
            .bind("B163")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(remaining_calendar.0, 0);
}

#[tokio::test]
async fn confirm_reservation_reprices_and_marks_room_occupied() {
    let pool = test_pool().await;
    seed_room(&pool, "R164").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();
    seed_booked_reservation(&pool, "B164", "R164")
        .await
        .unwrap();

    let today = Local::now().date_naive();
    let scheduled_checkin = today + Duration::days(2);
    let scheduled_checkout = today + Duration::days(5);
    let scheduled_checkin_str = scheduled_checkin.format("%Y-%m-%d").to_string();
    let scheduled_checkout_str = scheduled_checkout.format("%Y-%m-%d").to_string();

    sqlx::query(
        "UPDATE bookings
         SET check_in_at = ?, expected_checkout = ?, scheduled_checkin = ?, scheduled_checkout = ?, nights = ?, total_price = ?
         WHERE id = ?",
    )
    .bind(&scheduled_checkin_str)
    .bind(&scheduled_checkout_str)
    .bind(&scheduled_checkin_str)
    .bind(&scheduled_checkout_str)
    .bind(3_i64)
    .bind(1_800_000.0_f64)
    .bind("B164")
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query("DELETE FROM room_calendar WHERE booking_id = ?")
        .bind("B164")
        .execute(&pool)
        .await
        .unwrap();

    let mut date = scheduled_checkin;
    while date < scheduled_checkout {
        sqlx::query(
            "INSERT INTO room_calendar (room_id, date, booking_id, status) VALUES (?, ?, ?, 'booked')",
        )
        .bind("R164")
        .bind(date.format("%Y-%m-%d").to_string())
        .bind("B164")
        .execute(&pool)
        .await
        .unwrap();
        date += Duration::days(1);
    }

    let booking = reservation_lifecycle::confirm_reservation(&pool, "B164")
        .await
        .unwrap();

    assert_eq!(booking.status, "active");
    assert_eq!(booking.paid_amount, 50_000.0);
    assert_eq!(booking.expected_checkout, scheduled_checkout_str);
    assert_eq!(booking.nights, 5);
    assert_eq!(booking.total_price, 3_000_000.0);
    assert!(booking.check_in_at.contains('T'));

    let room = sqlx::query("SELECT status FROM rooms WHERE id = ?")
        .bind("R164")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(room.get::<String, _>("status"), "occupied");

    let calendar_rows = sqlx::query(
        "SELECT date, status FROM room_calendar WHERE booking_id = ? ORDER BY date ASC",
    )
    .bind("B164")
    .fetch_all(&pool)
    .await
    .unwrap();
    let actual_dates: Vec<String> = calendar_rows.iter().map(|row| row.get("date")).collect();
    let actual_statuses: Vec<String> = calendar_rows.iter().map(|row| row.get("status")).collect();
    let expected_dates: Vec<String> = (0..5)
        .map(|offset| {
            (today + Duration::days(offset))
                .format("%Y-%m-%d")
                .to_string()
        })
        .collect();
    assert_eq!(actual_dates, expected_dates);
    assert!(actual_statuses.iter().all(|status| status == "occupied"));

    let charge = sqlx::query(
        "SELECT type, amount, note FROM transactions WHERE booking_id = ? AND type = 'charge' LIMIT 1",
    )
    .bind("B164")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(charge.get::<String, _>("type"), "charge");
    assert_eq!(charge.get::<f64, _>("amount"), 3_000_000.0);
    assert_eq!(charge.get::<String, _>("note"), "Room charge (reservation)");
}

#[tokio::test]
async fn confirm_reservation_rejects_no_show_calendar_rows() {
    let pool = test_pool().await;
    seed_room(&pool, "R165").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();
    seed_booked_reservation(&pool, "B165", "R165")
        .await
        .unwrap();

    sqlx::query("UPDATE room_calendar SET status = ? WHERE booking_id = ?")
        .bind("no_show")
        .bind("B165")
        .execute(&pool)
        .await
        .unwrap();

    let error = reservation_lifecycle::confirm_reservation(&pool, "B165")
        .await
        .unwrap_err();

    assert!(matches!(
        &error,
        crate::domain::booking::BookingError::Conflict(_)
    ));
    assert!(error.to_string().contains("B165"));
}

#[tokio::test]
async fn confirm_reservation_late_arrival_persists_effective_checkout() {
    let pool = test_pool().await;
    seed_room(&pool, "R165A").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();
    seed_booked_reservation(&pool, "B165A", "R165A")
        .await
        .unwrap();

    let today = Local::now().date_naive();
    let scheduled_checkin = today - Duration::days(2);
    let scheduled_checkout = today;
    let scheduled_checkin_str = scheduled_checkin.format("%Y-%m-%d").to_string();
    let scheduled_checkout_str = scheduled_checkout.format("%Y-%m-%d").to_string();
    let effective_checkout_str = (today + Duration::days(1)).format("%Y-%m-%d").to_string();

    sqlx::query(
        "UPDATE bookings
         SET check_in_at = ?, expected_checkout = ?, scheduled_checkin = ?, scheduled_checkout = ?, nights = ?, total_price = ?
         WHERE id = ?",
    )
    .bind(&scheduled_checkin_str)
    .bind(&scheduled_checkout_str)
    .bind(&scheduled_checkin_str)
    .bind(&scheduled_checkout_str)
    .bind(2_i64)
    .bind(1_200_000.0_f64)
    .bind("B165A")
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query("DELETE FROM room_calendar WHERE booking_id = ?")
        .bind("B165A")
        .execute(&pool)
        .await
        .unwrap();

    let booking = reservation_lifecycle::confirm_reservation(&pool, "B165A")
        .await
        .unwrap();

    assert_eq!(booking.status, "active");
    assert_eq!(booking.nights, 1);
    assert_eq!(booking.expected_checkout, effective_checkout_str);
    assert_eq!(booking.total_price, 600_000.0);

    let calendar_rows = sqlx::query(
        "SELECT date, status FROM room_calendar WHERE booking_id = ? ORDER BY date ASC",
    )
    .bind("B165A")
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(calendar_rows.len(), 1);
    assert_eq!(
        calendar_rows[0].get::<String, _>("date"),
        today.format("%Y-%m-%d").to_string()
    );
    assert_eq!(calendar_rows[0].get::<String, _>("status"), "occupied");
}

#[tokio::test]
async fn confirm_reservation_preserves_extra_precheckin_payment() {
    let pool = test_pool().await;
    seed_room(&pool, "R165B").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();
    seed_booked_reservation(&pool, "B165B", "R165B")
        .await
        .unwrap();

    sqlx::query(
        "INSERT INTO transactions (id, booking_id, amount, type, note, created_at)
         VALUES (?, ?, ?, 'payment', ?, ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind("B165B")
    .bind(25_000.0_f64)
    .bind("Extra pre-check-in payment")
    .bind("2026-04-15T10:00:00+07:00")
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query("UPDATE bookings SET paid_amount = ? WHERE id = ?")
        .bind(75_000.0_f64)
        .bind("B165B")
        .execute(&pool)
        .await
        .unwrap();

    let booking = reservation_lifecycle::confirm_reservation(&pool, "B165B")
        .await
        .unwrap();

    assert_eq!(booking.paid_amount, 75_000.0);
}

#[tokio::test]
async fn modify_reservation_rewrites_booked_calendar_range() {
    let pool = test_pool().await;
    seed_room(&pool, "R166").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();
    seed_booked_reservation(&pool, "B166", "R166")
        .await
        .unwrap();

    let booking = reservation_lifecycle::modify_reservation(
        &pool,
        crate::models::ModifyReservationRequest {
            booking_id: "B166".to_string(),
            new_check_in_date: "2026-04-23".to_string(),
            new_check_out_date: "2026-04-26".to_string(),
            new_nights: 3,
        },
    )
    .await
    .unwrap();

    assert_eq!(booking.status, "booked");
    assert_eq!(booking.check_in_at, "2026-04-23");
    assert_eq!(booking.expected_checkout, "2026-04-26");
    assert_eq!(booking.nights, 3);
    assert_eq!(booking.total_price, 1_800_000.0);

    let booking_row =
        sqlx::query("SELECT scheduled_checkin, scheduled_checkout FROM bookings WHERE id = ?")
            .bind("B166")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        booking_row.get::<Option<String>, _>("scheduled_checkin"),
        Some("2026-04-23".to_string())
    );
    assert_eq!(
        booking_row.get::<Option<String>, _>("scheduled_checkout"),
        Some("2026-04-26".to_string())
    );

    let calendar_rows = sqlx::query(
        "SELECT date, status FROM room_calendar WHERE booking_id = ? ORDER BY date ASC",
    )
    .bind("B166")
    .fetch_all(&pool)
    .await
    .unwrap();
    let actual_dates: Vec<String> = calendar_rows.iter().map(|row| row.get("date")).collect();
    let actual_statuses: Vec<String> = calendar_rows.iter().map(|row| row.get("status")).collect();
    assert_eq!(
        actual_dates,
        vec![
            "2026-04-23".to_string(),
            "2026-04-24".to_string(),
            "2026-04-25".to_string(),
        ]
    );
    assert!(actual_statuses.iter().all(|status| status == "booked"));
}

#[tokio::test]
async fn modify_reservation_rejects_inconsistent_nights_input() {
    let pool = test_pool().await;
    seed_room(&pool, "R166A").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();
    seed_booked_reservation(&pool, "B166A", "R166A")
        .await
        .unwrap();

    let error = reservation_lifecycle::modify_reservation(
        &pool,
        crate::models::ModifyReservationRequest {
            booking_id: "B166A".to_string(),
            new_check_in_date: "2026-04-23".to_string(),
            new_check_out_date: "2026-04-26".to_string(),
            new_nights: 2,
        },
    )
    .await
    .unwrap_err();

    assert!(matches!(
        error,
        crate::domain::booking::BookingError::Validation(_)
    ));
}

#[tokio::test]
async fn do_modify_reservation_returns_service_booking_without_app_handle() {
    let pool = test_pool().await;
    seed_room(&pool, "R167").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();
    seed_booked_reservation(&pool, "B167", "R167")
        .await
        .unwrap();

    let booking = reservations::do_modify_reservation(
        &pool,
        None,
        crate::models::ModifyReservationRequest {
            booking_id: "B167".to_string(),
            new_check_in_date: "2026-04-24".to_string(),
            new_check_out_date: "2026-04-26".to_string(),
            new_nights: 2,
        },
    )
    .await
    .unwrap();

    assert_eq!(booking.status, "booked");
    assert_eq!(booking.check_in_at, "2026-04-24");
    assert_eq!(booking.expected_checkout, "2026-04-26");
    assert_eq!(booking.nights, 2);
    assert_eq!(booking.total_price, 1_200_000.0);

    let calendar_days: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM room_calendar WHERE booking_id = ? AND status = 'booked'",
    )
    .bind("B167")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(calendar_days.0, 2);
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

    let booking =
        sqlx::query("SELECT status, actual_checkout, paid_amount FROM bookings WHERE id = ?")
            .bind("B202")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(booking.get::<String, _>("status"), "checked_out");
    assert!(booking
        .get::<Option<String>, _>("actual_checkout")
        .is_some());
    assert_eq!(booking.get::<Option<f64>, _>("paid_amount"), None);

    let payment_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND type = 'payment'",
    )
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

    let calendar_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM room_calendar WHERE booking_id = ?")
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

    let extended_day =
        sqlx::query("SELECT status FROM room_calendar WHERE room_id = ? AND date = '2026-04-16'")
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

#[tokio::test]
async fn revenue_queries_use_recognized_room_revenue_and_ignore_payments() {
    let pool = test_pool().await;
    seed_room(&pool, "R301").await.unwrap();
    seed_active_booking(&pool, "B301", "R301").await.unwrap();
    seed_transaction(
        &pool,
        "B301",
        250_000.0,
        "charge",
        "Room charge",
        "2026-04-15T10:00:00+07:00",
    )
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B301",
        120_000.0,
        "payment",
        "Cash received",
        "2026-04-15T10:05:00+07:00",
    )
    .await
    .unwrap();
    seed_folio_line(&pool, "B301", 50_000.0, "2026-04-15T11:00:00+07:00")
        .await
        .unwrap();

    let dashboard = revenue_queries::load_dashboard_stats_for_date(&pool, "2026-04-15")
        .await
        .unwrap();
    let stats = revenue_queries::load_revenue_stats(
        &pool,
        "2026-04-15T00:00:00+07:00",
        "2026-04-15T23:59:59+07:00",
    )
    .await
    .unwrap();

    assert_eq!(dashboard.revenue_today, 300_000.0);
    assert_eq!(stats.total_revenue, 300_000.0);
    assert_eq!(stats.rooms_sold, 1);
    assert_eq!(stats.daily_revenue.len(), 1);
    assert_eq!(stats.daily_revenue[0].date, "2026-04-15");
    assert_eq!(stats.daily_revenue[0].revenue, 300_000.0);
}

#[tokio::test]
async fn analytics_breakdowns_reconcile_to_total_revenue() {
    let pool = test_pool().await;
    seed_room(&pool, "R302").await.unwrap();
    seed_active_booking(&pool, "B302", "R302").await.unwrap();
    seed_transaction(
        &pool,
        "B302",
        250_000.0,
        "charge",
        "Room charge",
        "2026-04-15T10:00:00+07:00",
    )
    .await
    .unwrap();
    seed_folio_line(&pool, "B302", 25_000.0, "2026-04-15T12:00:00+07:00")
        .await
        .unwrap();

    let analytics = revenue_queries::load_analytics(&pool, "2026-04-15", "2026-04-15", 1)
        .await
        .unwrap();

    assert_eq!(analytics.total_revenue, 275_000.0);
    assert_eq!(analytics.occupancy_rate, 100.0);
    assert_eq!(analytics.adr, 250_000.0);
    assert_eq!(analytics.revpar, 250_000.0);
    assert_eq!(analytics.daily_revenue.len(), 1);
    assert_eq!(analytics.revenue_by_source.len(), 1);
    assert_eq!(analytics.revenue_by_source[0].name, "walk-in");
    assert_eq!(analytics.revenue_by_source[0].value, 275_000.0);
    assert_eq!(analytics.top_rooms.len(), 1);
    assert_eq!(analytics.top_rooms[0].room, "R302");
    assert_eq!(analytics.top_rooms[0].revenue, 275_000.0);
}

#[tokio::test]
async fn revenue_queries_include_cancellation_fees_in_recognized_revenue() {
    let pool = test_pool().await;
    seed_room(&pool, "R305").await.unwrap();
    seed_booked_reservation(&pool, "B305", "R305")
        .await
        .unwrap();
    sqlx::query("UPDATE bookings SET status = 'cancelled' WHERE id = ?")
        .bind("B305")
        .execute(&pool)
        .await
        .unwrap();
    seed_transaction(
        &pool,
        "B305",
        50_000.0,
        "cancellation_fee",
        "Retained deposit",
        "2026-04-15T14:00:00+07:00",
    )
    .await
    .unwrap();

    let stats = revenue_queries::load_revenue_stats(
        &pool,
        "2026-04-15T00:00:00+07:00",
        "2026-04-15T23:59:59+07:00",
    )
    .await
    .unwrap();
    let export_rows = audit_queries::load_booking_export_rows(&pool, "2026-04-01", "2026-04-30")
        .await
        .unwrap();
    let cancelled_row = export_rows.iter().find(|row| row.id == "B305").unwrap();

    assert_eq!(stats.total_revenue, 50_000.0);
    assert_eq!(cancelled_row.charge_total, 0.0);
    assert_eq!(cancelled_row.cancellation_fee_total, 50_000.0);
    assert_eq!(cancelled_row.recognized_revenue, 50_000.0);
}

#[tokio::test]
async fn run_night_audit_uses_canonical_room_and_folio_revenue() {
    let pool = test_pool().await;
    seed_room(&pool, "R303").await.unwrap();
    seed_active_booking(&pool, "B303", "R303").await.unwrap();
    sqlx::query(
        "UPDATE bookings
         SET nights = 2, total_price = 500000, expected_checkout = '2026-04-17T10:00:00+07:00'
         WHERE id = ?",
    )
    .bind("B303")
    .execute(&pool)
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B303",
        500_000.0,
        "charge",
        "Room charge",
        "2026-04-15T10:00:00+07:00",
    )
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B303",
        90_000.0,
        "payment",
        "Cash received",
        "2026-04-16T10:05:00+07:00",
    )
    .await
    .unwrap();
    seed_folio_line(&pool, "B303", 40_000.0, "2026-04-16T13:00:00+07:00")
        .await
        .unwrap();
    seed_expense(&pool, "electricity", 10_000.0, "2026-04-16")
        .await
        .unwrap();

    let log = audit_service::run_night_audit(
        &pool,
        "2026-04-16",
        Some("Checked and closed".to_string()),
        "admin-1",
    )
    .await
    .unwrap();

    assert_eq!(log.audit_date, "2026-04-16");
    assert_eq!(log.room_revenue, 250_000.0);
    assert_eq!(log.folio_revenue, 40_000.0);
    assert_eq!(log.total_revenue, 290_000.0);
    assert_eq!(log.total_expenses, 10_000.0);
    assert_eq!(log.rooms_sold, 1);
    assert_eq!(log.total_rooms, 1);

    let audited: i32 = sqlx::query_scalar("SELECT is_audited FROM bookings WHERE id = ?")
        .bind("B303")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(audited, 1);
}

#[tokio::test]
async fn billing_and_export_queries_preserve_canonical_revenue_columns() {
    let pool = test_pool().await;
    seed_room(&pool, "R304").await.unwrap();
    seed_active_booking(&pool, "B304", "R304").await.unwrap();
    seed_transaction(
        &pool,
        "B304",
        250_000.0,
        "charge",
        "Room charge",
        "2026-04-15T10:00:00+07:00",
    )
    .await
    .unwrap();

    let line = add_folio_line(
        &pool,
        "B304",
        "laundry",
        "Laundry bundle",
        35_000.0,
        Some("staff-1"),
    )
    .await
    .unwrap();
    let folio_lines = billing_queries::list_folio_lines(&pool, "B304")
        .await
        .unwrap();
    let export_rows = audit_queries::load_booking_export_rows(&pool, "2026-04-01", "2026-04-30")
        .await
        .unwrap();

    assert_eq!(line.amount, 35_000.0);
    assert_eq!(folio_lines.len(), 1);
    assert_eq!(folio_lines[0].category, "laundry");
    assert_eq!(export_rows.len(), 1);
    assert_eq!(export_rows[0].room_price, 250_000.0);
    assert_eq!(export_rows[0].charge_total, 250_000.0);
    assert_eq!(export_rows[0].cancellation_fee_total, 0.0);
    assert_eq!(export_rows[0].folio_total, 35_000.0);
    assert_eq!(export_rows[0].recognized_revenue, 285_000.0);
}
