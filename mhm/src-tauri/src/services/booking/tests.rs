use chrono::{Duration, Local, NaiveDate, TimeZone};
use sqlx::{sqlite::SqlitePoolOptions, Pool, Row, Sqlite, Transaction};

use crate::{
    commands::reservations,
    domain::booking::{
        pricing::{calculate_stay_price, calculate_stay_price_tx},
        BookingError, BookingResult, OriginSideEffect,
    },
    models::{
        CheckInRequest, CheckOutRequest, CheckoutSettlementMode, CheckoutSettlementPreviewRequest,
        CreateGuestRequest, CreateReservationRequest, GroupCheckinRequest, GroupCheckoutRequest,
    },
    queries::booking::{audit_queries, billing_queries, revenue_queries},
};

use super::{
    audit_service,
    billing_service::{
        add_folio_line, add_folio_line_idempotent, record_cancellation_fee_tx, record_deposit_tx,
        record_deposit_with_origin_tx, record_payment, record_payment_tx,
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
            origin_idempotency_key TEXT,
            origin_transaction_ordinal INTEGER NOT NULL DEFAULT 0,
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
            origin_idempotency_key TEXT,
            origin_line_ordinal INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create folio_lines table");

    sqlx::query(
        "CREATE UNIQUE INDEX transactions_origin_idem_uq
         ON transactions (booking_id, origin_idempotency_key, origin_transaction_ordinal)
         WHERE origin_idempotency_key IS NOT NULL AND origin_idempotency_key != ''",
    )
    .execute(&pool)
    .await
    .expect("failed to create transactions origin index");

    sqlx::query(
        "CREATE UNIQUE INDEX folio_lines_origin_idem_uq
         ON folio_lines (booking_id, origin_idempotency_key, origin_line_ordinal)
         WHERE origin_idempotency_key IS NOT NULL AND origin_idempotency_key != ''",
    )
    .execute(&pool)
    .await
    .expect("failed to create folio_lines origin index");

    sqlx::query(
        "CREATE UNIQUE INDEX transactions_origin_command_uq
         ON transactions (origin_idempotency_key, origin_transaction_ordinal)
         WHERE origin_idempotency_key IS NOT NULL AND origin_idempotency_key != ''",
    )
    .execute(&pool)
    .await
    .expect("failed to create transactions origin command index");

    sqlx::query(
        "CREATE UNIQUE INDEX folio_lines_origin_command_uq
         ON folio_lines (origin_idempotency_key, origin_line_ordinal)
         WHERE origin_idempotency_key IS NOT NULL AND origin_idempotency_key != ''",
    )
    .execute(&pool)
    .await
    .expect("failed to create folio_lines origin command index");

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

    sqlx::query(
        "CREATE TABLE command_idempotency (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            idempotency_key TEXT NOT NULL,
            command_name TEXT NOT NULL,
            request_id TEXT,
            actor_type TEXT NOT NULL DEFAULT 'system',
            actor_id TEXT,
            client_id TEXT,
            session_id TEXT,
            channel_id TEXT,
            issued_at TEXT,
            request_hash TEXT NOT NULL,
            intent_json TEXT NOT NULL,
            summary_json TEXT NOT NULL DEFAULT '{}',
            primary_aggregate_key TEXT,
            lock_keys_json TEXT NOT NULL,
            status TEXT NOT NULL,
            claim_token TEXT NOT NULL,
            response_json TEXT,
            result_summary_json TEXT,
            error_code TEXT,
            error_json TEXT,
            error_summary_json TEXT,
            retryable INTEGER NOT NULL DEFAULT 0,
            lease_expires_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            completed_at TEXT,
            last_attempt_at TEXT,
            UNIQUE(command_name, idempotency_key)
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create command_idempotency table");

    pool
}

async fn shared_file_test_pools(label: &str) -> (Pool<Sqlite>, Pool<Sqlite>, std::path::PathBuf) {
    let db_path =
        std::env::temp_dir().join(format!("capyinn-{label}-{}.sqlite", uuid::Uuid::new_v4()));
    let database_url = format!("sqlite://{}?mode=rwc", db_path.display());

    let pool_a = SqlitePoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("failed to open first sqlite file test pool");

    crate::db::run_migrations(&pool_a)
        .await
        .expect("failed to run migrations for shared file test pool");

    let pool_b = SqlitePoolOptions::new()
        .max_connections(2)
        .connect(&database_url)
        .await
        .expect("failed to open second sqlite file test pool");

    (pool_a, pool_b, db_path)
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

#[allow(clippy::too_many_arguments)]
pub async fn seed_active_booking_with_terms(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    room_id: &str,
    check_in_at: &str,
    expected_checkout: &str,
    nights: i64,
    total_price: f64,
    paid_amount: Option<f64>,
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

pub fn rich_group_checkin_request(
    room_ids: &[&str],
    master_room_id: &str,
    paid_amount: Option<f64>,
) -> GroupCheckinRequest {
    let mut guests_per_room = std::collections::HashMap::new();
    for room_id in room_ids {
        guests_per_room.insert(
            (*room_id).to_string(),
            vec![CreateGuestRequest {
                guest_type: Some("domestic".to_string()),
                full_name: format!("Group Guest {}", room_id),
                doc_number: format!("DOC-{}", room_id),
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
        group_name: "Idempotent Group".to_string(),
        organizer_name: "Organizer".to_string(),
        organizer_phone: Some("0903333333".to_string()),
        check_in_date: None,
        room_ids: room_ids
            .iter()
            .map(|room_id| (*room_id).to_string())
            .collect(),
        master_room_id: master_room_id.to_string(),
        guests_per_room,
        nights: 2,
        source: Some("walk-in".to_string()),
        notes: Some("group checkin idempotent".to_string()),
        paid_amount,
    }
}

#[tokio::test]
async fn calendar_insert_conflict_returns_room_unavailable_without_overwrite() {
    let pool = test_pool().await;
    seed_room(&pool, "CAL-1").await.unwrap();
    seed_booked_reservation(&pool, "existing-booking", "CAL-1")
        .await
        .unwrap();
    seed_active_booking(&pool, "new-booking", "CAL-1")
        .await
        .unwrap();

    let mut tx = pool.begin().await.unwrap();
    let error = crate::services::booking::support::insert_room_calendar_rows(
        &mut tx,
        "CAL-1",
        "new-booking",
        NaiveDate::from_ymd_opt(2026, 4, 20).unwrap(),
        NaiveDate::from_ymd_opt(2026, 4, 21).unwrap(),
        crate::models::status::calendar::BOOKED,
    )
    .await
    .unwrap_err();

    assert!(error
        .to_string()
        .contains(crate::app_error::codes::CONFLICT_ROOM_UNAVAILABLE));
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
        (Local::now().date_naive() + Duration::days(1))
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
async fn group_checkin_lock_keys_are_stable_for_room_order() {
    let left = crate::aggregate_locks::canonicalize_lock_keys(vec![
        crate::aggregate_locks::room_key("R2").unwrap(),
        crate::aggregate_locks::room_key("R1").unwrap(),
    ])
    .unwrap();
    let right = crate::aggregate_locks::canonicalize_lock_keys(vec![
        crate::aggregate_locks::room_key("R1").unwrap(),
        crate::aggregate_locks::room_key("R2").unwrap(),
    ])
    .unwrap();

    assert_eq!(left, right);
}

#[tokio::test]
async fn group_checkin_idempotent_normalizes_room_order_and_assigns_payment_ordinals() {
    let pool = test_pool().await;
    seed_room(&pool, "GI601").await.unwrap();
    seed_room(&pool, "GI602").await.unwrap();
    seed_pricing_rule(&pool, "standard", 250_000.0)
        .await
        .unwrap();
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-group-idem-1",
        "idem-group-checkin-1",
        "group_checkin",
    );

    let first = group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &ctx,
        rich_group_checkin_request(&["GI602", "GI601"], "GI602", Some(100_000.0)),
    )
    .await
    .expect("first group checkin succeeds");
    let second = group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &ctx,
        rich_group_checkin_request(&["GI601", "GI602"], "GI602", Some(100_000.0)),
    )
    .await
    .expect("same payload with different room order replays");

    assert!(!first.replayed);
    assert!(second.replayed);
    assert_eq!(first.response["id"], second.response["id"]);

    let rows = sqlx::query(
        "SELECT b.room_id, t.origin_transaction_ordinal
         FROM transactions t
         JOIN bookings b ON b.id = t.booking_id
         WHERE t.origin_idempotency_key = ? AND t.type = 'payment'
         ORDER BY t.origin_transaction_ordinal ASC",
    )
    .bind("idem-group-checkin-1")
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<String, _>("room_id"), "GI601");
    assert_eq!(rows[0].get::<i64, _>("origin_transaction_ordinal"), 0);
    assert_eq!(rows[1].get::<String, _>("room_id"), "GI602");
    assert_eq!(rows[1].get::<i64, _>("origin_transaction_ordinal"), 1);

    let lock_keys_json: String = sqlx::query_scalar(
        "SELECT lock_keys_json
         FROM command_idempotency
         WHERE idempotency_key = ?",
    )
    .bind("idem-group-checkin-1")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(lock_keys_json, r#"["room:GI601","room:GI602"]"#);
}

#[tokio::test]
async fn group_checkin_duplicate_in_flight_does_not_wait_for_room_lock() {
    let pool = test_pool().await;
    seed_room(&pool, "GI650").await.unwrap();
    seed_room(&pool, "GI651").await.unwrap();
    seed_pricing_rule(&pool, "standard", 250_000.0)
        .await
        .unwrap();
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-group-idem-inflight",
        "idem-group-checkin-inflight",
        "group_checkin",
    );
    let held_room_lock = crate::aggregate_locks::global_manager()
        .acquire([crate::aggregate_locks::room_key("GI650").unwrap()])
        .await
        .unwrap();

    let first_pool = pool.clone();
    let first_ctx = ctx.clone();
    let first = tokio::spawn(async move {
        group_lifecycle::group_checkin_idempotent(
            &first_pool,
            Some("seed-user".to_string()),
            &first_ctx,
            rich_group_checkin_request(&["GI650", "GI651"], "GI650", Some(100_000.0)),
        )
        .await
    });

    let mut claim_seen = false;
    for _ in 0..50 {
        let in_progress_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*)
             FROM command_idempotency
             WHERE idempotency_key = ? AND status = 'in_progress'",
        )
        .bind("idem-group-checkin-inflight")
        .fetch_one(&pool)
        .await
        .unwrap();
        if in_progress_count == 1 {
            claim_seen = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        claim_seen,
        "first command should claim before waiting for room lock"
    );

    let duplicate = tokio::time::timeout(
        std::time::Duration::from_millis(200),
        group_lifecycle::group_checkin_idempotent(
            &pool,
            Some("seed-user".to_string()),
            &ctx,
            rich_group_checkin_request(&["GI650", "GI651"], "GI650", Some(100_000.0)),
        ),
    )
    .await
    .expect("duplicate should return without waiting for room lock")
    .expect_err("duplicate in-flight command should conflict");

    assert_eq!(
        duplicate.code,
        crate::app_error::codes::CONFLICT_DUPLICATE_IN_FLIGHT
    );

    drop(held_room_lock);
    first.await.unwrap().expect("first command completes");
}

#[tokio::test]
async fn group_checkin_idempotent_non_positive_paid_amount_writes_no_payment_origin_rows() {
    let pool = test_pool().await;
    seed_room(&pool, "GI610").await.unwrap();
    seed_room(&pool, "GI611").await.unwrap();
    seed_room(&pool, "GI612").await.unwrap();
    seed_room(&pool, "GI613").await.unwrap();
    seed_room(&pool, "GI614").await.unwrap();
    seed_room(&pool, "GI615").await.unwrap();
    seed_pricing_rule(&pool, "standard", 250_000.0)
        .await
        .unwrap();

    let zero_ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-group-idem-2-zero",
        "idem-group-checkin-2-zero",
        "group_checkin",
    );
    let zero_paid = group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &zero_ctx,
        rich_group_checkin_request(&["GI610", "GI611"], "GI610", Some(0.0)),
    )
    .await
    .expect("zero paid amount still creates group");
    assert_eq!(zero_paid.response["status"].as_str(), Some("active"));

    let negative_ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-group-idem-2-neg",
        "idem-group-checkin-2-neg",
        "group_checkin",
    );
    let negative_paid = group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &negative_ctx,
        rich_group_checkin_request(&["GI612", "GI613"], "GI612", Some(-40_000.0)),
    )
    .await
    .expect("negative paid amount still creates group");
    assert_eq!(negative_paid.response["status"].as_str(), Some("active"));

    let negative_reservation_ctx =
        crate::command_idempotency::WriteCommandContext::for_internal_test(
            "req-group-idem-2-neg-reservation",
            "idem-group-checkin-2-neg-reservation",
            "group_checkin",
        );
    let mut negative_reservation_req =
        rich_group_checkin_request(&["GI614", "GI615"], "GI614", Some(-40_000.0));
    negative_reservation_req.check_in_date = Some("2026-05-10".to_string());
    let negative_reservation = group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &negative_reservation_ctx,
        negative_reservation_req,
    )
    .await
    .expect("negative reservation paid amount still creates group");
    assert_eq!(
        negative_reservation.response["status"].as_str(),
        Some("booked")
    );

    let zero_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE origin_idempotency_key = ?")
            .bind("idem-group-checkin-2-zero")
            .fetch_one(&pool)
            .await
            .unwrap();
    let negative_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE origin_idempotency_key = ?")
            .bind("idem-group-checkin-2-neg")
            .fetch_one(&pool)
            .await
            .unwrap();
    let negative_reservation_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE origin_idempotency_key = ?")
            .bind("idem-group-checkin-2-neg-reservation")
            .fetch_one(&pool)
            .await
            .unwrap();
    let negative_reservation_group_id = negative_reservation.response["id"].as_str().unwrap();
    let negative_reservation_deposit_sum: f64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(deposit_amount), 0)
         FROM bookings
         WHERE group_id = ?",
    )
    .bind(negative_reservation_group_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(zero_count, 0);
    assert_eq!(negative_count, 0);
    assert_eq!(negative_reservation_count, 0);
    assert_eq!(negative_reservation_deposit_sum, 0.0);
}

#[tokio::test]
async fn group_checkin_idempotent_blank_key_rejected_before_writes() {
    let pool = test_pool().await;
    seed_room(&pool, "GI620").await.unwrap();
    seed_room(&pool, "GI621").await.unwrap();
    seed_pricing_rule(&pool, "standard", 250_000.0)
        .await
        .unwrap();

    let error = crate::command_idempotency::WriteCommandContext::for_scoped_command(
        "req-group-idem-blank",
        " ",
        "group_checkin",
    )
    .expect_err("blank idempotency key rejected");
    assert_eq!(
        error.code,
        crate::app_error::codes::IDEMPOTENCY_KEY_REQUIRED
    );

    let group_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM booking_groups")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(group_count, 0);

    let claim_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM command_idempotency WHERE request_id = 'req-group-idem-blank'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(claim_count, 0);
}

#[tokio::test]
async fn group_checkin_idempotent_replay_returns_stored_snapshot_after_db_mutation() {
    let pool = test_pool().await;
    seed_room(&pool, "GI630").await.unwrap();
    seed_room(&pool, "GI631").await.unwrap();
    seed_pricing_rule(&pool, "standard", 250_000.0)
        .await
        .unwrap();
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-group-idem-3",
        "idem-group-checkin-3",
        "group_checkin",
    );

    let first = group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &ctx,
        rich_group_checkin_request(&["GI630", "GI631"], "GI630", Some(100_000.0)),
    )
    .await
    .expect("first group checkin succeeds");
    let group_id = first.response["id"].as_str().unwrap().to_string();
    let first_status = first.response["status"].as_str().unwrap().to_string();

    sqlx::query("UPDATE booking_groups SET status = 'completed' WHERE id = ?")
        .bind(&group_id)
        .execute(&pool)
        .await
        .unwrap();

    let replay = group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &ctx,
        rich_group_checkin_request(&["GI630", "GI631"], "GI630", Some(100_000.0)),
    )
    .await
    .expect("replay succeeds");

    assert!(replay.replayed);
    assert_eq!(
        replay.response["status"].as_str(),
        Some(first_status.as_str())
    );
    assert_ne!(replay.response["status"].as_str(), Some("completed"));
}

#[tokio::test]
async fn group_checkin_idempotent_same_key_different_payload_conflicts() {
    let pool = test_pool().await;
    seed_room(&pool, "GI640").await.unwrap();
    seed_room(&pool, "GI641").await.unwrap();
    seed_pricing_rule(&pool, "standard", 250_000.0)
        .await
        .unwrap();
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-group-idem-4",
        "idem-group-checkin-4",
        "group_checkin",
    );

    group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &ctx,
        rich_group_checkin_request(&["GI640", "GI641"], "GI640", Some(100_000.0)),
    )
    .await
    .expect("first group checkin succeeds");

    let mut changed = rich_group_checkin_request(&["GI640", "GI641"], "GI640", Some(100_000.0));
    changed.nights = 3;
    let error = group_lifecycle::group_checkin_idempotent(
        &pool,
        Some("seed-user".to_string()),
        &ctx,
        changed,
    )
    .await
    .expect_err("same key with different payload conflicts");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
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
async fn group_checkout_rejects_stale_selected_booking() {
    let pool = test_pool().await;
    seed_room(&pool, "G501").await.unwrap();
    seed_room(&pool, "G502").await.unwrap();
    seed_pricing_rule(&pool, "standard", 250_000.0)
        .await
        .unwrap();

    let group = group_lifecycle::group_checkin(
        &pool,
        Some("seed-user".to_string()),
        minimal_group_checkin_request(&["G501", "G502"]),
    )
    .await
    .unwrap();

    let booking_ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM bookings WHERE group_id = ? ORDER BY room_id")
            .bind(&group.id)
            .fetch_all(&pool)
            .await
            .unwrap();
    sqlx::query("UPDATE bookings SET status = 'checked_out' WHERE id = ?")
        .bind(&booking_ids[0])
        .execute(&pool)
        .await
        .unwrap();

    let error = group_lifecycle::group_checkout(
        &pool,
        GroupCheckoutRequest {
            group_id: group.id.clone(),
            booking_ids,
            final_paid: None,
        },
    )
    .await
    .unwrap_err();

    assert!(error
        .to_string()
        .contains(crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION));
}

#[test]
fn group_checkout_locked_room_map_rejects_changed_room_mapping() {
    let locked = std::collections::HashMap::from([
        ("booking-1".to_string(), "room-old".to_string()),
        ("booking-2".to_string(), "room-stable".to_string()),
    ]);
    let current = std::collections::HashMap::from([
        ("booking-1".to_string(), "room-new".to_string()),
        ("booking-2".to_string(), "room-stable".to_string()),
    ]);

    let error = group_lifecycle::ensure_group_checkout_room_map_still_locked(
        "group-1",
        &["booking-1".to_string(), "booking-2".to_string()],
        &locked,
        &current,
    )
    .unwrap_err();

    assert!(error
        .to_string()
        .contains(crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION));
    assert!(error
        .to_string()
        .contains("one or more bookings in group group-1 changed rooms before checkout"));
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
async fn record_deposit_with_origin_writes_origin_key_and_ordinal() {
    let pool = test_pool().await;
    seed_room(&pool, "R501").await.unwrap();
    let booking_id = seed_booking_for_origin_tests(&pool, "R501").await.unwrap();
    let origin = OriginSideEffect::new("idem-deposit-1", 0).unwrap();

    let mut tx = pool.begin().await.unwrap();
    record_deposit_with_origin_tx(&mut tx, &booking_id, 25_000.0, "origin deposit", &origin)
        .await
        .unwrap();
    tx.commit().await.unwrap();

    let row = sqlx::query(
        "SELECT origin_idempotency_key, origin_transaction_ordinal
         FROM transactions
         WHERE booking_id = ? AND note = ?",
    )
    .bind(&booking_id)
    .bind("origin deposit")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(
        row.get::<String, _>("origin_idempotency_key"),
        "idem-deposit-1"
    );
    assert_eq!(row.get::<i64, _>("origin_transaction_ordinal"), 0);
}

#[tokio::test]
async fn record_deposit_with_origin_rejects_blank_key_before_write() {
    let pool = test_pool().await;
    seed_room(&pool, "R502").await.unwrap();
    let booking_id = seed_booking_for_origin_tests(&pool, "R502").await.unwrap();

    let err = OriginSideEffect::new(" ", 0).expect_err("blank key should be rejected");
    assert!(err
        .to_string()
        .contains("Origin idempotency key is required"));

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE booking_id = ?")
        .bind(&booking_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
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
async fn create_reservation_idempotent_retry_does_not_duplicate_deposit() {
    let pool = test_pool().await;
    seed_room(&pool, "R601").await.expect("seeds room");
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .expect("seeds pricing");
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-reservation-1",
        "idem-reservation-1",
        "create_reservation",
    );

    let first = reservation_lifecycle::create_reservation_idempotent(
        &pool,
        &ctx,
        CreateReservationRequest {
            room_id: "R601".to_string(),
            guest_name: "Retry Guest".to_string(),
            guest_doc_number: Some("DOC601".to_string()),
            guest_phone: None,
            check_in_date: "2026-05-01".to_string(),
            check_out_date: "2026-05-02".to_string(),
            nights: 1,
            source: Some("phone".to_string()),
            notes: None,
            deposit_amount: Some(50_000.0),
        },
    )
    .await
    .expect("first reservation succeeds");
    let second = reservation_lifecycle::create_reservation_idempotent(
        &pool,
        &ctx,
        CreateReservationRequest {
            room_id: "R601".to_string(),
            guest_name: "Retry Guest".to_string(),
            guest_doc_number: Some("DOC601".to_string()),
            guest_phone: None,
            check_in_date: "2026-05-01".to_string(),
            check_out_date: "2026-05-02".to_string(),
            nights: 1,
            source: Some("phone".to_string()),
            notes: None,
            deposit_amount: Some(50_000.0),
        },
    )
    .await
    .expect("retry replays");

    assert_eq!(first.response["id"], second.response["id"]);
    assert!(!first.replayed);
    assert!(second.replayed);

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE origin_idempotency_key = ?")
            .bind("create_reservation:idem-reservation-1")
            .fetch_one(&pool)
            .await
            .expect("counts deposit rows");

    assert_eq!(count, 1);
}

#[tokio::test]
async fn create_reservation_idempotent_replay_returns_stored_booking_snapshot() {
    let pool = test_pool().await;
    seed_room(&pool, "R604").await.expect("seeds room");
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .expect("seeds pricing");
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-reservation-snapshot",
        "idem-reservation-snapshot",
        "create_reservation",
    );
    let first = reservation_lifecycle::create_reservation_idempotent(
        &pool,
        &ctx,
        CreateReservationRequest {
            room_id: "R604".to_string(),
            guest_name: "Snapshot Guest".to_string(),
            guest_doc_number: Some("DOC604".to_string()),
            guest_phone: None,
            check_in_date: "2026-05-01".to_string(),
            check_out_date: "2026-05-02".to_string(),
            nights: 1,
            source: Some("phone".to_string()),
            notes: None,
            deposit_amount: Some(50_000.0),
        },
    )
    .await
    .expect("first reservation succeeds");
    let booking_id = first.response["id"]
        .as_str()
        .expect("id in first response")
        .to_string();
    let first_status = first.response["status"]
        .as_str()
        .expect("status in first response")
        .to_string();
    assert_eq!(first_status, "booked");

    sqlx::query("UPDATE bookings SET status = 'cancelled' WHERE id = ?")
        .bind(&booking_id)
        .execute(&pool)
        .await
        .expect("mutates booking status");

    let replay = reservation_lifecycle::create_reservation_idempotent(
        &pool,
        &ctx,
        CreateReservationRequest {
            room_id: "R604".to_string(),
            guest_name: "Snapshot Guest".to_string(),
            guest_doc_number: Some("DOC604".to_string()),
            guest_phone: None,
            check_in_date: "2026-05-01".to_string(),
            check_out_date: "2026-05-02".to_string(),
            nights: 1,
            source: Some("phone".to_string()),
            notes: None,
            deposit_amount: Some(50_000.0),
        },
    )
    .await
    .expect("replay succeeds");

    assert!(replay.replayed);
    assert_eq!(
        replay.response["status"].as_str(),
        Some(first_status.as_str())
    );
    assert_ne!(replay.response["status"].as_str(), Some("cancelled"));
}

#[tokio::test]
async fn create_reservation_same_key_different_payload_conflicts() {
    let pool = test_pool().await;
    seed_room(&pool, "R602").await.expect("seeds room");
    seed_room(&pool, "R603").await.expect("seeds room");
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .expect("seeds pricing");
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-reservation-conflict",
        "idem-reservation-conflict",
        "create_reservation",
    );

    reservation_lifecycle::create_reservation_idempotent(
        &pool,
        &ctx,
        CreateReservationRequest {
            room_id: "R602".to_string(),
            guest_name: "Conflict Guest".to_string(),
            guest_doc_number: Some("DOC602".to_string()),
            guest_phone: None,
            check_in_date: "2026-05-01".to_string(),
            check_out_date: "2026-05-02".to_string(),
            nights: 1,
            source: Some("phone".to_string()),
            notes: None,
            deposit_amount: Some(50_000.0),
        },
    )
    .await
    .expect("first reservation succeeds");

    let error = reservation_lifecycle::create_reservation_idempotent(
        &pool,
        &ctx,
        CreateReservationRequest {
            room_id: "R603".to_string(),
            guest_name: "Conflict Guest".to_string(),
            guest_doc_number: Some("DOC602".to_string()),
            guest_phone: None,
            check_in_date: "2026-05-01".to_string(),
            check_out_date: "2026-05-02".to_string(),
            nights: 1,
            source: Some("phone".to_string()),
            notes: None,
            deposit_amount: Some(50_000.0),
        },
    )
    .await
    .expect_err("same key with different payload conflicts");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
    );
}

#[tokio::test]
async fn reservation_command_idempotency_create_hashes_deposit_as_integer_vnd_units() {
    let pool = test_pool().await;
    seed_room(&pool, "R690").await.expect("seeds room");
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .expect("seeds pricing");
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-reservation-deposit-vnd",
        "idem-reservation-deposit-vnd",
        "create_reservation",
    );
    let request = CreateReservationRequest {
        room_id: "R690".to_string(),
        guest_name: "Deposit Units Guest".to_string(),
        guest_doc_number: Some("DOC690".to_string()),
        guest_phone: None,
        check_in_date: "2026-05-01".to_string(),
        check_out_date: "2026-05-02".to_string(),
        nights: 1,
        source: Some("phone".to_string()),
        notes: None,
        deposit_amount: Some(500_000.0),
    };

    reservation_lifecycle::create_reservation_idempotent(&pool, &ctx, request)
        .await
        .expect("reservation succeeds");

    let row = sqlx::query(
        "SELECT request_hash, intent_json FROM command_idempotency
         WHERE command_name = ? AND idempotency_key = ?",
    )
    .bind(&ctx.command_name)
    .bind(&ctx.idempotency_key)
    .fetch_one(&pool)
    .await
    .expect("reads command row");

    let expected_payload = serde_json::json!({
        "schema": "reservation.create.v1",
        "room_id": "R690",
        "guest_name": "Deposit Units Guest",
        "guest_doc_number": "DOC690",
        "guest_phone": null,
        "check_in_date": "2026-05-01",
        "check_out_date": "2026-05-02",
        "nights": 1,
        "source": "phone",
        "notes": null,
        "deposit_vnd_units": 500000,
    });
    let mut cents_payload = expected_payload.clone();
    cents_payload["deposit_vnd_units"] = serde_json::json!(50000000);
    let mut float_payload = expected_payload.clone();
    float_payload["deposit_vnd_units"] = serde_json::json!(500000.0);
    let mut string_payload = expected_payload.clone();
    string_payload["deposit_vnd_units"] = serde_json::json!("500000");

    assert_eq!(
        row.get::<String, _>("request_hash"),
        crate::command_idempotency::stable_request_hash(&expected_payload)
            .expect("expected payload hashes")
    );
    assert_ne!(
        row.get::<String, _>("request_hash"),
        crate::command_idempotency::stable_request_hash(&cents_payload)
            .expect("cents payload hashes")
    );
    assert_ne!(
        row.get::<String, _>("request_hash"),
        crate::command_idempotency::stable_request_hash(&float_payload)
            .expect("float payload hashes")
    );
    assert_ne!(
        row.get::<String, _>("request_hash"),
        crate::command_idempotency::stable_request_hash(&string_payload)
            .expect("string payload hashes")
    );

    let intent_json = row.get::<String, _>("intent_json");
    assert!(intent_json.contains("\"deposit_present\":true"));
    assert!(intent_json.contains("\"deposit_vnd_units\":500000"));
}

fn reservation_modify_request(
    booking_id: &str,
    check_in: &str,
    check_out: &str,
    nights: i32,
) -> crate::models::ModifyReservationRequest {
    crate::models::ModifyReservationRequest {
        booking_id: booking_id.to_string(),
        new_check_in_date: check_in.to_string(),
        new_check_out_date: check_out.to_string(),
        new_nights: nights,
    }
}

#[tokio::test]
async fn reservation_command_idempotency_create_replay_does_not_duplicate_booking_or_calendar() {
    let pool = test_pool().await;
    seed_room(&pool, "R691").await.expect("seeds room");
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .expect("seeds pricing");
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-create-replay-no-dup",
        "idem-create-replay-no-dup",
        "reservation.create",
    );

    let first = reservation_lifecycle::create_reservation_idempotent(
        &pool,
        &ctx,
        minimal_reservation_request("R691"),
    )
    .await
    .expect("first create succeeds");
    let replay = reservation_lifecycle::create_reservation_idempotent(
        &pool,
        &ctx,
        minimal_reservation_request("R691"),
    )
    .await
    .expect("create replays");

    assert!(!first.replayed);
    assert!(replay.replayed);
    assert_eq!(first.response, replay.response);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM bookings WHERE room_id = ?")
            .bind("R691")
            .fetch_one(&pool)
            .await
            .expect("counts bookings"),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM room_calendar WHERE room_id = ?")
            .bind("R691")
            .fetch_one(&pool)
            .await
            .expect("counts calendar rows"),
        2
    );
}

#[tokio::test]
async fn reservation_command_idempotency_modify_replay_returns_stored_snapshot() {
    let pool = test_pool().await;
    seed_room(&pool, "R692").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();
    seed_booked_reservation(&pool, "B692", "R692")
        .await
        .unwrap();
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-modify-snapshot",
        "idem-modify-snapshot",
        "reservation.modify",
    );

    let first = reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &ctx,
        reservation_modify_request("B692", "2026-04-23", "2026-04-26", 3),
    )
    .await
    .expect("first modify succeeds");
    sqlx::query("UPDATE bookings SET total_price = 999 WHERE id = ?")
        .bind("B692")
        .execute(&pool)
        .await
        .expect("mutates booking after first response");
    let replay = reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &ctx,
        reservation_modify_request("B692", "2026-04-23", "2026-04-26", 3),
    )
    .await
    .expect("modify replays");

    assert!(replay.replayed);
    assert_eq!(first.response, replay.response);
    assert_eq!(
        replay.response["total_price"],
        serde_json::json!(1_800_000.0)
    );
}

#[tokio::test]
async fn reservation_command_idempotency_modify_replay_does_not_duplicate_calendar() {
    let pool = test_pool().await;
    seed_room(&pool, "R693").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();
    seed_booked_reservation(&pool, "B693", "R693")
        .await
        .unwrap();
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-modify-calendar",
        "idem-modify-calendar",
        "reservation.modify",
    );
    reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &ctx,
        reservation_modify_request("B693", "2026-04-23", "2026-04-26", 3),
    )
    .await
    .expect("first modify succeeds");
    reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &ctx,
        reservation_modify_request("B693", "2026-04-23", "2026-04-26", 3),
    )
    .await
    .expect("modify replays");

    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM room_calendar WHERE booking_id = ?")
            .bind("B693")
            .fetch_one(&pool)
            .await
            .expect("counts calendar rows"),
        3
    );
}

#[tokio::test]
async fn reservation_command_idempotency_cancel_replay_does_not_duplicate_cancellation_fee() {
    let pool = test_pool().await;
    seed_room(&pool, "R694").await.unwrap();
    seed_booked_reservation(&pool, "B694", "R694")
        .await
        .unwrap();
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-cancel-fee",
        "idem-cancel-fee",
        "reservation.cancel",
    );

    let first = reservation_lifecycle::cancel_reservation_idempotent(&pool, &ctx, "B694")
        .await
        .expect("first cancel succeeds");
    let replay = reservation_lifecycle::cancel_reservation_idempotent(&pool, &ctx, "B694")
        .await
        .expect("cancel replays");

    assert_eq!(first.response, replay.response);
    assert!(replay.replayed);
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND type = 'cancellation_fee'",
        )
        .bind("B694")
        .fetch_one(&pool)
        .await
        .expect("counts cancellation fees"),
        1
    );
}

#[tokio::test]
async fn reservation_command_idempotency_confirm_replay_does_not_duplicate_room_charge() {
    let pool = test_pool().await;
    seed_room(&pool, "R695").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();
    seed_booked_reservation(&pool, "B695", "R695")
        .await
        .unwrap();
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-confirm-charge",
        "idem-confirm-charge",
        "reservation.confirm",
    );

    reservation_lifecycle::confirm_reservation_idempotent(&pool, &ctx, "B695")
        .await
        .expect("first confirm succeeds");
    reservation_lifecycle::confirm_reservation_idempotent(&pool, &ctx, "B695")
        .await
        .expect("confirm replays");

    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND type = 'charge'",
        )
        .bind("B695")
        .fetch_one(&pool)
        .await
        .expect("counts room charges"),
        1
    );
}

#[tokio::test]
async fn reservation_command_idempotency_confirm_replay_does_not_requery_or_reprice_later_retry() {
    let pool = test_pool().await;
    seed_room(&pool, "R696").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();
    seed_booked_reservation(&pool, "B696", "R696")
        .await
        .unwrap();
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-confirm-no-reprice",
        "idem-confirm-no-reprice",
        "reservation.confirm",
    );

    let first = reservation_lifecycle::confirm_reservation_idempotent(&pool, &ctx, "B696")
        .await
        .expect("first confirm succeeds");
    sqlx::query("UPDATE pricing_rules SET daily_rate = 9999999 WHERE room_type = 'standard'")
        .execute(&pool)
        .await
        .expect("mutates pricing");
    sqlx::query("UPDATE bookings SET total_price = 123 WHERE id = ?")
        .bind("B696")
        .execute(&pool)
        .await
        .expect("mutates booking");
    let replay = reservation_lifecycle::confirm_reservation_idempotent(&pool, &ctx, "B696")
        .await
        .expect("confirm replays");

    assert!(replay.replayed);
    assert_eq!(first.response, replay.response);
    assert_ne!(replay.response["total_price"], serde_json::json!(123.0));
}

#[tokio::test]
async fn reservation_command_idempotency_modify_cancel_confirm_same_key_different_payload_conflicts(
) {
    let pool = test_pool().await;
    seed_room(&pool, "R697A").await.unwrap();
    seed_room(&pool, "R697B").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();
    seed_booked_reservation(&pool, "B697A", "R697A")
        .await
        .unwrap();
    seed_booked_reservation(&pool, "B697B", "R697B")
        .await
        .unwrap();

    let modify_ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-modify-hash-conflict",
        "idem-modify-hash-conflict",
        "reservation.modify",
    );
    reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &modify_ctx,
        crate::models::ModifyReservationRequest {
            booking_id: "B697A".to_string(),
            new_check_in_date: "2026-04-23".to_string(),
            new_check_out_date: "2026-04-25".to_string(),
            new_nights: 2,
        },
    )
    .await
    .expect("first modify succeeds");
    let modify_error = reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &modify_ctx,
        crate::models::ModifyReservationRequest {
            booking_id: "B697B".to_string(),
            new_check_in_date: "2026-04-23".to_string(),
            new_check_out_date: "2026-04-25".to_string(),
            new_nights: 2,
        },
    )
    .await
    .expect_err("different modify payload conflicts");
    assert_eq!(
        modify_error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
    );

    let cancel_ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-cancel-hash-conflict",
        "idem-cancel-hash-conflict",
        "reservation.cancel",
    );
    reservation_lifecycle::cancel_reservation_idempotent(&pool, &cancel_ctx, "B697A")
        .await
        .expect("first cancel succeeds");
    let cancel_error =
        reservation_lifecycle::cancel_reservation_idempotent(&pool, &cancel_ctx, "B697B")
            .await
            .expect_err("different cancel payload conflicts");
    assert_eq!(
        cancel_error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
    );

    let confirm_ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-confirm-hash-conflict",
        "idem-confirm-hash-conflict",
        "reservation.confirm",
    );
    reservation_lifecycle::confirm_reservation_idempotent(&pool, &confirm_ctx, "B697B")
        .await
        .expect("first confirm succeeds");
    let confirm_error =
        reservation_lifecycle::confirm_reservation_idempotent(&pool, &confirm_ctx, "B697A")
            .await
            .expect_err("different confirm payload conflicts");
    assert_eq!(
        confirm_error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
    );
}

#[tokio::test]
async fn reservation_command_idempotency_modify_conflict_replays_terminal_room_unavailable() {
    let pool = test_pool().await;
    seed_room(&pool, "R698").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();
    seed_booked_reservation(&pool, "B698", "R698")
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO room_calendar (room_id, date, booking_id, status)
         VALUES (?, '2026-04-23', NULL, 'booked')",
    )
    .bind("R698")
    .execute(&pool)
    .await
    .expect("seeds conflicting calendar");
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-modify-room-conflict",
        "idem-modify-room-conflict",
        "reservation.modify",
    );
    let first = reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &ctx,
        reservation_modify_request("B698", "2026-04-23", "2026-04-25", 2),
    )
    .await
    .expect_err("first conflict fails");
    sqlx::query("DELETE FROM room_calendar WHERE room_id = ? AND booking_id IS NULL")
        .bind("R698")
        .execute(&pool)
        .await
        .expect("removes conflict");
    let replay = reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &ctx,
        reservation_modify_request("B698", "2026-04-23", "2026-04-25", 2),
    )
    .await
    .expect_err("terminal conflict replays");

    assert_eq!(
        first.code,
        crate::app_error::codes::CONFLICT_ROOM_UNAVAILABLE
    );
    assert_eq!(replay.code, first.code);
}

#[tokio::test]
async fn reservation_command_idempotency_cancel_confirm_invalid_state_replays_terminal() {
    let pool = test_pool().await;
    seed_room(&pool, "R699A").await.unwrap();
    seed_room(&pool, "R699B").await.unwrap();
    seed_booked_reservation(&pool, "B699A", "R699A")
        .await
        .unwrap();
    seed_booked_reservation(&pool, "B699B", "R699B")
        .await
        .unwrap();
    sqlx::query("UPDATE bookings SET status = 'active' WHERE id = ?")
        .bind("B699A")
        .execute(&pool)
        .await
        .expect("makes cancel invalid");
    sqlx::query("UPDATE bookings SET status = 'cancelled' WHERE id = ?")
        .bind("B699B")
        .execute(&pool)
        .await
        .expect("makes confirm invalid");

    let cancel_ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-cancel-invalid-replay",
        "idem-cancel-invalid-replay",
        "reservation.cancel",
    );
    let cancel_first =
        reservation_lifecycle::cancel_reservation_idempotent(&pool, &cancel_ctx, "B699A")
            .await
            .expect_err("cancel invalid state fails");
    sqlx::query("UPDATE bookings SET status = 'booked' WHERE id = ?")
        .bind("B699A")
        .execute(&pool)
        .await
        .expect("would make cancel valid");
    let cancel_replay =
        reservation_lifecycle::cancel_reservation_idempotent(&pool, &cancel_ctx, "B699A")
            .await
            .expect_err("cancel invalid state replays");

    let confirm_ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-confirm-invalid-replay",
        "idem-confirm-invalid-replay",
        "reservation.confirm",
    );
    let confirm_first =
        reservation_lifecycle::confirm_reservation_idempotent(&pool, &confirm_ctx, "B699B")
            .await
            .expect_err("confirm invalid state fails");
    sqlx::query("UPDATE bookings SET status = 'booked' WHERE id = ?")
        .bind("B699B")
        .execute(&pool)
        .await
        .expect("would make confirm valid");
    let confirm_replay =
        reservation_lifecycle::confirm_reservation_idempotent(&pool, &confirm_ctx, "B699B")
            .await
            .expect_err("confirm invalid state replays");

    assert_eq!(
        cancel_first.code,
        crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION
    );
    assert_eq!(cancel_replay.code, cancel_first.code);
    assert_eq!(
        confirm_first.code,
        crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION
    );
    assert_eq!(confirm_replay.code, confirm_first.code);
}

#[tokio::test]
async fn reservation_command_idempotency_missing_booking_for_cancel_modify_confirm_replays_terminal(
) {
    let pool = test_pool().await;
    seed_room(&pool, "R700A").await.unwrap();
    seed_room(&pool, "R700B").await.unwrap();
    seed_room(&pool, "R700C").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();

    let cancel_ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-cancel-missing",
        "idem-cancel-missing",
        "reservation.cancel",
    );
    let cancel_first =
        reservation_lifecycle::cancel_reservation_idempotent(&pool, &cancel_ctx, "B700A")
            .await
            .expect_err("missing cancel booking fails");
    seed_booked_reservation(&pool, "B700A", "R700A")
        .await
        .unwrap();
    let cancel_replay =
        reservation_lifecycle::cancel_reservation_idempotent(&pool, &cancel_ctx, "B700A")
            .await
            .expect_err("missing cancel booking replays");

    let modify_ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-modify-missing",
        "idem-modify-missing",
        "reservation.modify",
    );
    let modify_first = reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &modify_ctx,
        reservation_modify_request("B700B", "2026-04-23", "2026-04-25", 2),
    )
    .await
    .expect_err("missing modify booking fails");
    seed_booked_reservation(&pool, "B700B", "R700B")
        .await
        .unwrap();
    let modify_replay = reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &modify_ctx,
        reservation_modify_request("B700B", "2026-04-23", "2026-04-25", 2),
    )
    .await
    .expect_err("missing modify booking replays");

    let confirm_ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-confirm-missing",
        "idem-confirm-missing",
        "reservation.confirm",
    );
    let confirm_first =
        reservation_lifecycle::confirm_reservation_idempotent(&pool, &confirm_ctx, "B700C")
            .await
            .expect_err("missing confirm booking fails");
    seed_booked_reservation(&pool, "B700C", "R700C")
        .await
        .unwrap();
    let confirm_replay =
        reservation_lifecycle::confirm_reservation_idempotent(&pool, &confirm_ctx, "B700C")
            .await
            .expect_err("missing confirm booking replays");

    for error in [
        cancel_first,
        cancel_replay,
        modify_first,
        modify_replay,
        confirm_first,
        confirm_replay,
    ] {
        assert_eq!(error.code, crate::app_error::codes::BOOKING_NOT_FOUND);
    }
}

#[tokio::test]
async fn reservation_command_idempotency_duplicate_in_flight_returns_conflict() {
    let pool = test_pool().await;
    seed_room(&pool, "R701").await.unwrap();
    seed_booked_reservation(&pool, "B701", "R701")
        .await
        .unwrap();
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-cancel-in-flight",
        "idem-cancel-in-flight",
        "reservation.cancel",
    );
    let payload = serde_json::json!({
        "schema": "reservation.cancel.v1",
        "booking_id": "B701",
    });
    let now = chrono::Utc::now().to_rfc3339();
    let lease_expires_at = (chrono::Utc::now() + chrono::Duration::seconds(30)).to_rfc3339();
    sqlx::query(
        "INSERT INTO command_idempotency (
            idempotency_key, command_name, request_hash, intent_json, lock_keys_json,
            status, claim_token, retryable, lease_expires_at, created_at, updated_at, last_attempt_at
        ) VALUES (?, ?, ?, '{}', '[]', 'in_progress', 'other-claim', 0, ?, ?, ?, ?)",
    )
    .bind(&ctx.idempotency_key)
    .bind(&ctx.command_name)
    .bind(crate::command_idempotency::stable_request_hash(&payload).expect("payload hashes"))
    .bind(&lease_expires_at)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("seeds in-flight row");

    let error = reservation_lifecycle::cancel_reservation_idempotent(&pool, &ctx, "B701")
        .await
        .expect_err("duplicate in-flight conflicts");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_DUPLICATE_IN_FLIGHT
    );
}

#[tokio::test]
async fn reservation_command_idempotency_retryable_reclaimable_failure_can_be_reclaimed() {
    let pool = test_pool().await;
    seed_room(&pool, "R702").await.unwrap();
    seed_booked_reservation(&pool, "B702", "R702")
        .await
        .unwrap();
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-cancel-reclaim",
        "idem-cancel-reclaim",
        "reservation.cancel",
    );
    let payload = serde_json::json!({
        "schema": "reservation.cancel.v1",
        "booking_id": "B702",
    });
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO command_idempotency (
            idempotency_key, command_name, request_hash, intent_json, lock_keys_json,
            status, claim_token, error_code, error_json, retryable, created_at, updated_at,
            last_attempt_at
        ) VALUES (?, ?, ?, '{}', '[]', 'failed_retryable', 'failed-claim',
            'DB_LOCKED_RETRYABLE', '{}', 1, ?, ?, ?)",
    )
    .bind(&ctx.idempotency_key)
    .bind(&ctx.command_name)
    .bind(crate::command_idempotency::stable_request_hash(&payload).expect("payload hashes"))
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(&pool)
    .await
    .expect("seeds retryable row");

    let result = reservation_lifecycle::cancel_reservation_idempotent(&pool, &ctx, "B702")
        .await
        .expect("retryable row is reclaimed");

    assert!(!result.replayed);
    assert_eq!(result.response["ok"], true);
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT status FROM command_idempotency WHERE command_name = ? AND idempotency_key = ?",
        )
        .bind(&ctx.command_name)
        .bind(&ctx.idempotency_key)
        .fetch_one(&pool)
        .await
        .expect("reads status"),
        "completed"
    );
}

#[tokio::test]
async fn reservation_command_idempotency_invalid_modify_nights_replays_terminal_error() {
    let pool = test_pool().await;
    seed_room(&pool, "R703").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();
    seed_booked_reservation(&pool, "B703", "R703")
        .await
        .unwrap();
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-modify-invalid-nights",
        "idem-modify-invalid-nights",
        "reservation.modify",
    );

    let first = reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &ctx,
        reservation_modify_request("B703", "2026-04-23", "2026-04-26", 2),
    )
    .await
    .expect_err("invalid nights should fail inside command boundary");

    let status: String = sqlx::query_scalar(
        "SELECT status FROM command_idempotency
         WHERE command_name = ? AND idempotency_key = ?",
    )
    .bind(&ctx.command_name)
    .bind(&ctx.idempotency_key)
    .fetch_one(&pool)
    .await
    .expect("invalid command row is stored");
    assert_eq!(status, "failed_terminal");

    let replay = reservation_lifecycle::modify_reservation_idempotent(
        &pool,
        &ctx,
        reservation_modify_request("B703", "2026-04-23", "2026-04-26", 2),
    )
    .await
    .expect_err("invalid nights should replay stored terminal error");

    assert_eq!(first.code, replay.code);
    assert_eq!(first.message, replay.message);
}

#[tokio::test]
async fn reservation_command_idempotency_same_plain_key_across_commands_scopes_origin_rows() {
    let pool = test_pool().await;
    seed_room(&pool, "R704").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();
    let plain_key = "idem-shared-reservation-origin";
    let create_ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-create-shared-origin",
        plain_key,
        "reservation.create",
    );
    let cancel_ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-cancel-shared-origin",
        plain_key,
        "reservation.cancel",
    );

    let created = reservation_lifecycle::create_reservation_idempotent(
        &pool,
        &create_ctx,
        minimal_reservation_request("R704"),
    )
    .await
    .expect("create with deposit succeeds");
    let booking_id = created.response["id"]
        .as_str()
        .expect("booking id in create response")
        .to_string();

    reservation_lifecycle::cancel_reservation_idempotent(&pool, &cancel_ctx, &booking_id)
        .await
        .expect("cancel with same plain key but different command succeeds");

    let origins = sqlx::query_scalar::<_, String>(
        "SELECT origin_idempotency_key
         FROM transactions
         WHERE booking_id = ? AND type IN ('deposit', 'cancellation_fee')
         ORDER BY type ASC",
    )
    .bind(&booking_id)
    .fetch_all(&pool)
    .await
    .expect("reads transaction origins");

    assert_eq!(origins.len(), 2);
    assert!(origins.contains(&format!("{}:{}", create_ctx.command_name, plain_key)));
    assert!(origins.contains(&format!("{}:{}", cancel_ctx.command_name, plain_key)));
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
async fn cancel_reservation_returns_invalid_state_when_booking_is_not_booked() {
    let pool = test_pool().await;
    seed_room(&pool, "R-CAS-CANCEL").await.unwrap();
    seed_booked_reservation(&pool, "B-CAS-CANCEL", "R-CAS-CANCEL")
        .await
        .unwrap();

    sqlx::query("UPDATE bookings SET status = 'active' WHERE id = ?")
        .bind("B-CAS-CANCEL")
        .execute(&pool)
        .await
        .unwrap();

    let error = reservation_lifecycle::cancel_reservation(&pool, "B-CAS-CANCEL")
        .await
        .expect_err("stale reservation should fail");

    assert!(error
        .to_string()
        .contains(crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION));
}

#[tokio::test]
async fn do_create_reservation_returns_service_booking_and_leaves_room_vacant() {
    let pool = test_pool().await;
    seed_room(&pool, "R162").await.unwrap();
    seed_pricing_rule(&pool, "standard", 600_000.0)
        .await
        .unwrap();

    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-do-create-reservation",
        "idem-do-create-reservation",
        "create_reservation",
    );
    let booking =
        reservations::do_create_reservation(&pool, None, &ctx, minimal_reservation_request("R162"))
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

    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-do-cancel-reservation",
        "idem-do-cancel-reservation",
        "cancel_reservation",
    );
    let response = reservations::do_cancel_reservation(&pool, None, &ctx, "B163")
        .await
        .unwrap();
    assert!(response.ok);
    assert_eq!(response.booking_id, "B163");

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
async fn confirm_reservation_returns_invalid_state_when_booking_is_not_booked() {
    let pool = test_pool().await;
    seed_room(&pool, "R-CAS-CONFIRM").await.unwrap();
    seed_booked_reservation(&pool, "B-CAS-CONFIRM", "R-CAS-CONFIRM")
        .await
        .unwrap();

    sqlx::query("UPDATE bookings SET status = 'cancelled' WHERE id = ?")
        .bind("B-CAS-CONFIRM")
        .execute(&pool)
        .await
        .unwrap();

    let error = reservation_lifecycle::confirm_reservation(&pool, "B-CAS-CONFIRM")
        .await
        .expect_err("stale reservation should fail");

    assert!(error
        .to_string()
        .contains(crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION));
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
async fn modify_reservation_returns_invalid_state_when_booking_is_not_booked() {
    let pool = test_pool().await;
    seed_room(&pool, "R-CAS-MOD").await.unwrap();
    seed_booked_reservation(&pool, "B-CAS-MOD", "R-CAS-MOD")
        .await
        .unwrap();

    sqlx::query("UPDATE bookings SET status = 'cancelled' WHERE id = ?")
        .bind("B-CAS-MOD")
        .execute(&pool)
        .await
        .unwrap();

    let error = reservation_lifecycle::modify_reservation(
        &pool,
        crate::models::ModifyReservationRequest {
            booking_id: "B-CAS-MOD".to_string(),
            new_check_in_date: "2026-04-24".to_string(),
            new_check_out_date: "2026-04-26".to_string(),
            new_nights: 2,
        },
    )
    .await
    .expect_err("stale reservation should fail");

    assert!(error
        .to_string()
        .contains(crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION));
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

    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-do-modify-reservation",
        "idem-do-modify-reservation",
        "modify_reservation",
    );
    let booking = reservations::do_modify_reservation(
        &pool,
        None,
        &ctx,
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
async fn check_in_rolls_back_when_room_status_changes_before_guarded_room_update() {
    let pool = test_pool().await;
    seed_room(&pool, "R-CAS-CHECKIN").await.unwrap();
    seed_pricing_rule(&pool, "standard", 100_000.0)
        .await
        .unwrap();

    sqlx::query(
        "CREATE TRIGGER occupy_room_after_booking_insert
         AFTER INSERT ON bookings
         WHEN NEW.room_id = 'R-CAS-CHECKIN'
         BEGIN
           UPDATE rooms SET status = 'occupied' WHERE id = NEW.room_id;
         END",
    )
    .execute(&pool)
    .await
    .unwrap();

    let error = stay_lifecycle::check_in(&pool, minimal_checkin_request("R-CAS-CHECKIN"), None)
        .await
        .expect_err("guarded room update should catch stale state");

    assert!(error
        .to_string()
        .contains(crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION));

    let booking_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bookings WHERE room_id = ?")
        .bind("R-CAS-CHECKIN")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(booking_count, 0);
}

#[tokio::test]
async fn checkout_fails_when_second_pool_checked_out_booking_first() {
    let (pool_a, pool_b, db_path) = shared_file_test_pools("second-pool-checkout").await;
    seed_room(&pool_a, "R-2POOL").await.unwrap();
    seed_active_booking(&pool_a, "B-2POOL", "R-2POOL")
        .await
        .unwrap();

    sqlx::query("UPDATE bookings SET status = 'checked_out' WHERE id = ?")
        .bind("B-2POOL")
        .execute(&pool_b)
        .await
        .unwrap();

    let error = stay_lifecycle::check_out(
        &pool_a,
        CheckOutRequest {
            booking_id: "B-2POOL".to_string(),
            settlement_mode: CheckoutSettlementMode::BookedNights,
            final_total: 100_000.0,
        },
    )
    .await
    .expect_err("checkout should reject stale booking state");

    assert!(error
        .to_string()
        .contains(crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION));

    pool_a.close().await;
    pool_b.close().await;
    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn check_out_settles_same_day_actual_nights_to_minimum_one_night() {
    let pool = test_pool().await;
    seed_room(&pool, "R410").await.unwrap();
    seed_pricing_rule(&pool, "standard", 500_000.0)
        .await
        .unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B410",
        "R410",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000.0,
        Some(0.0),
    )
    .await
    .unwrap();

    let preview = stay_lifecycle::preview_checkout_settlement_at(
        &pool,
        CheckoutSettlementPreviewRequest {
            booking_id: "B410".to_string(),
            settlement_mode: CheckoutSettlementMode::ActualNights,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 20, 18, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(preview.settled_nights, 1);
    assert_eq!(preview.recommended_total, 500_000.0);

    stay_lifecycle::check_out_at(
        &pool,
        CheckOutRequest {
            booking_id: "B410".to_string(),
            settlement_mode: CheckoutSettlementMode::ActualNights,
            final_total: preview.recommended_total,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 20, 18, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap();

    let booking = sqlx::query(
        "SELECT nights, total_price, paid_amount, pricing_snapshot
         FROM bookings WHERE id = ?",
    )
    .bind("B410")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(booking.get::<i64, _>("nights"), 1);
    assert_eq!(booking.get::<f64, _>("total_price"), 500_000.0);
    assert_eq!(booking.get::<f64, _>("paid_amount"), 500_000.0);
    assert!(booking
        .get::<Option<String>, _>("pricing_snapshot")
        .unwrap()
        .contains("\"reporting_checkout\""));
}

#[tokio::test]
async fn check_out_keeps_active_booking_values_for_booked_nights_mode() {
    let pool = test_pool().await;
    seed_room(&pool, "R411").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B411",
        "R411",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000.0,
        Some(0.0),
    )
    .await
    .unwrap();
    record_payment(&pool, "B411", 1_000_000.0, "prior payment")
        .await
        .unwrap();

    let preview = stay_lifecycle::preview_checkout_settlement_at(
        &pool,
        CheckoutSettlementPreviewRequest {
            booking_id: "B411".to_string(),
            settlement_mode: CheckoutSettlementMode::BookedNights,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 22, 9, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(preview.settled_nights, 5);
    assert_eq!(preview.recommended_total, 2_500_000.0);

    stay_lifecycle::check_out_at(
        &pool,
        CheckOutRequest {
            booking_id: "B411".to_string(),
            settlement_mode: CheckoutSettlementMode::BookedNights,
            final_total: preview.recommended_total,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 22, 9, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap();

    let booking = sqlx::query("SELECT nights, total_price, paid_amount FROM bookings WHERE id = ?")
        .bind("B411")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(booking.get::<i64, _>("nights"), 5);
    assert_eq!(booking.get::<f64, _>("total_price"), 2_500_000.0);
    assert_eq!(booking.get::<f64, _>("paid_amount"), 2_500_000.0);
}

#[tokio::test]
async fn check_out_booked_nights_enforces_minimum_one_night_for_corrupted_booking() {
    let pool = test_pool().await;
    seed_room(&pool, "R413").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B413",
        "R413",
        "2026-04-20T08:00:00+07:00",
        "2026-04-20T12:00:00+07:00",
        0,
        0.0,
        Some(0.0),
    )
    .await
    .unwrap();

    let preview = stay_lifecycle::preview_checkout_settlement_at(
        &pool,
        CheckoutSettlementPreviewRequest {
            booking_id: "B413".to_string(),
            settlement_mode: CheckoutSettlementMode::BookedNights,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 20, 12, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(preview.settled_nights, 1);

    stay_lifecycle::check_out_at(
        &pool,
        CheckOutRequest {
            booking_id: "B413".to_string(),
            settlement_mode: CheckoutSettlementMode::BookedNights,
            final_total: 0.0,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 20, 12, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap();

    let booking = sqlx::query("SELECT nights FROM bookings WHERE id = ?")
        .bind("B413")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(booking.get::<i64, _>("nights"), 1);
}

#[tokio::test]
async fn check_out_actual_nights_uses_early_checkout_nights() {
    let pool = test_pool().await;
    seed_room(&pool, "R414").await.unwrap();
    seed_pricing_rule(&pool, "standard", 500_000.0)
        .await
        .unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B414",
        "R414",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000.0,
        Some(0.0),
    )
    .await
    .unwrap();

    let preview = stay_lifecycle::preview_checkout_settlement_at(
        &pool,
        CheckoutSettlementPreviewRequest {
            booking_id: "B414".to_string(),
            settlement_mode: CheckoutSettlementMode::ActualNights,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 22, 9, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(preview.settled_nights, 2);
    assert_eq!(preview.recommended_total, 1_000_000.0);

    stay_lifecycle::check_out_at(
        &pool,
        CheckOutRequest {
            booking_id: "B414".to_string(),
            settlement_mode: CheckoutSettlementMode::ActualNights,
            final_total: preview.recommended_total,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 22, 9, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap();

    let booking = sqlx::query("SELECT nights, total_price FROM bookings WHERE id = ?")
        .bind("B414")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(booking.get::<i64, _>("nights"), 2);
    assert_eq!(booking.get::<f64, _>("total_price"), 1_000_000.0);
}

#[tokio::test]
async fn check_out_hourly_persists_manual_settlement() {
    let pool = test_pool().await;
    seed_room(&pool, "R415").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B415",
        "R415",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000.0,
        Some(0.0),
    )
    .await
    .unwrap();

    stay_lifecycle::check_out_at(
        &pool,
        CheckOutRequest {
            booking_id: "B415".to_string(),
            settlement_mode: CheckoutSettlementMode::Hourly,
            final_total: 500_000.0,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 20, 10, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap();

    let booking = sqlx::query(
        "SELECT nights, total_price, paid_amount, pricing_snapshot
         FROM bookings WHERE id = ?",
    )
    .bind("B415")
    .fetch_one(&pool)
    .await
    .unwrap();

    let pricing_snapshot = booking
        .get::<Option<String>, _>("pricing_snapshot")
        .unwrap();
    let pricing_snapshot: serde_json::Value = serde_json::from_str(&pricing_snapshot).unwrap();

    assert_eq!(booking.get::<i64, _>("nights"), 1);
    assert_eq!(booking.get::<f64, _>("total_price"), 500_000.0);
    assert_eq!(booking.get::<f64, _>("paid_amount"), 500_000.0);
    assert_eq!(
        pricing_snapshot["checkout_settlement"]["mode"],
        serde_json::json!("hourly")
    );
    assert_eq!(
        pricing_snapshot["checkout_settlement"]["settled_total"],
        serde_json::json!(500_000.0)
    );
}

#[tokio::test]
async fn check_out_hourly_multi_day_stay_still_persists_one_night() {
    let pool = test_pool().await;
    seed_room(&pool, "R419").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B419",
        "R419",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000.0,
        Some(0.0),
    )
    .await
    .unwrap();

    stay_lifecycle::check_out_at(
        &pool,
        CheckOutRequest {
            booking_id: "B419".to_string(),
            settlement_mode: CheckoutSettlementMode::Hourly,
            final_total: 500_000.0,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 22, 9, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap();

    let booking = sqlx::query("SELECT nights, total_price FROM bookings WHERE id = ?")
        .bind("B419")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(booking.get::<i64, _>("nights"), 1);
    assert_eq!(booking.get::<f64, _>("total_price"), 500_000.0);
}

#[tokio::test]
async fn check_out_persists_manual_override_when_final_total_differs_from_recommendation() {
    let pool = test_pool().await;
    seed_room(&pool, "R416").await.unwrap();
    seed_pricing_rule(&pool, "standard", 500_000.0)
        .await
        .unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B416",
        "R416",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000.0,
        Some(0.0),
    )
    .await
    .unwrap();
    record_payment(&pool, "B416", 300_000.0, "prior payment")
        .await
        .unwrap();

    let preview = stay_lifecycle::preview_checkout_settlement_at(
        &pool,
        CheckoutSettlementPreviewRequest {
            booking_id: "B416".to_string(),
            settlement_mode: CheckoutSettlementMode::ActualNights,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 22, 9, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap();

    assert_eq!(preview.recommended_total, 1_000_000.0);

    stay_lifecycle::check_out_at(
        &pool,
        CheckOutRequest {
            booking_id: "B416".to_string(),
            settlement_mode: CheckoutSettlementMode::ActualNights,
            final_total: 800_000.0,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 22, 9, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap();

    let booking = sqlx::query(
        "SELECT total_price, paid_amount, pricing_snapshot
         FROM bookings WHERE id = ?",
    )
    .bind("B416")
    .fetch_one(&pool)
    .await
    .unwrap();

    let pricing_snapshot = booking
        .get::<Option<String>, _>("pricing_snapshot")
        .unwrap();
    let pricing_snapshot: serde_json::Value = serde_json::from_str(&pricing_snapshot).unwrap();

    assert_eq!(booking.get::<f64, _>("total_price"), 800_000.0);
    assert_eq!(booking.get::<f64, _>("paid_amount"), 800_000.0);
    assert_eq!(
        pricing_snapshot["checkout_settlement"]["manual_override"],
        serde_json::json!(true)
    );
    assert_eq!(
        pricing_snapshot["checkout_settlement"]["settled_total"],
        serde_json::json!(800_000.0)
    );
}

#[tokio::test]
async fn check_out_writes_charge_adjustment_ledger_when_settled_total_drops() {
    let pool = test_pool().await;
    seed_room(&pool, "R417").await.unwrap();
    seed_pricing_rule(&pool, "standard", 500_000.0)
        .await
        .unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B417",
        "R417",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000.0,
        Some(0.0),
    )
    .await
    .unwrap();
    record_payment(&pool, "B417", 800_000.0, "prior payment")
        .await
        .unwrap();

    stay_lifecycle::check_out_at(
        &pool,
        CheckOutRequest {
            booking_id: "B417".to_string(),
            settlement_mode: CheckoutSettlementMode::ActualNights,
            final_total: 800_000.0,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 22, 9, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap();

    let adjustment = sqlx::query(
        "SELECT amount, note FROM transactions
         WHERE booking_id = ? AND type = 'charge' AND note LIKE 'Điều chỉnh %'
         LIMIT 1",
    )
    .bind("B417")
    .fetch_one(&pool)
    .await
    .unwrap();

    let payment_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM transactions
         WHERE booking_id = ? AND type = 'payment' AND note = 'Thanh toán khi check-out'",
    )
    .bind("B417")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(adjustment.get::<f64, _>("amount"), -1_700_000.0);
    assert_eq!(
        adjustment.get::<String, _>("note"),
        "Điều chỉnh giảm tiền phòng khi quyết toán check-out"
    );
    assert_eq!(payment_count.0, 0);
}

#[tokio::test]
async fn check_out_writes_payment_delta_ledger_when_collecting_extra_payment() {
    let pool = test_pool().await;
    seed_room(&pool, "R418").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B418",
        "R418",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000.0,
        Some(0.0),
    )
    .await
    .unwrap();
    record_payment(&pool, "B418", 1_000_000.0, "prior payment")
        .await
        .unwrap();

    stay_lifecycle::check_out_at(
        &pool,
        CheckOutRequest {
            booking_id: "B418".to_string(),
            settlement_mode: CheckoutSettlementMode::BookedNights,
            final_total: 2_500_000.0,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 22, 9, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap();

    let payment = sqlx::query(
        "SELECT amount, note FROM transactions
         WHERE booking_id = ? AND type = 'payment' AND note = 'Thanh toán khi check-out'
         LIMIT 1",
    )
    .bind("B418")
    .fetch_one(&pool)
    .await
    .unwrap();

    let booking = sqlx::query("SELECT paid_amount FROM bookings WHERE id = ?")
        .bind("B418")
        .fetch_one(&pool)
        .await
        .unwrap();

    let charge_adjustment_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM transactions
         WHERE booking_id = ? AND type = 'charge' AND note LIKE 'Điều chỉnh %'",
    )
    .bind("B418")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(payment.get::<f64, _>("amount"), 1_500_000.0);
    assert_eq!(payment.get::<String, _>("note"), "Thanh toán khi check-out");
    assert_eq!(booking.get::<f64, _>("paid_amount"), 2_500_000.0);
    assert_eq!(charge_adjustment_count.0, 0);
}

#[tokio::test]
async fn checkout_paid_amount_is_ledger_projection_not_direct_overwrite() {
    let pool = test_pool().await;
    seed_room(&pool, "R420").await.unwrap();
    seed_pricing_rule(&pool, "standard", 75_000.0)
        .await
        .unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B420",
        "R420",
        "2026-04-20T08:00:00+07:00",
        "2026-04-21T12:00:00+07:00",
        1,
        75_000.0,
        Some(0.0),
    )
    .await
    .unwrap();
    record_payment(&pool, "B420", 75_000.0, "prior payment")
        .await
        .unwrap();
    sqlx::query(
        "CREATE TRIGGER forbid_paid_amount_direct_update_in_checkout_test
         BEFORE UPDATE OF paid_amount ON bookings
         BEGIN
             SELECT RAISE(ABORT, 'paid_amount direct update forbidden in checkout test');
         END",
    )
    .execute(&pool)
    .await
    .unwrap();

    stay_lifecycle::check_out_at(
        &pool,
        CheckOutRequest {
            booking_id: "B420".to_string(),
            settlement_mode: CheckoutSettlementMode::ActualNights,
            final_total: 75_000.0,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 21, 9, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap();

    let booking = sqlx::query("SELECT paid_amount FROM bookings WHERE id = ?")
        .bind("B420")
        .fetch_one(&pool)
        .await
        .unwrap();
    let ledger_total: f64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(amount), 0)
         FROM transactions
         WHERE booking_id = ? AND type IN ('payment', 'deposit')",
    )
    .bind("B420")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(ledger_total, 75_000.0);
    assert_eq!(booking.get::<f64, _>("paid_amount"), ledger_total);
}

#[tokio::test]
async fn check_out_rejects_overpaid_booking_until_refund_flow_exists() {
    let pool = test_pool().await;
    seed_room(&pool, "R412").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B412",
        "R412",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000.0,
        Some(0.0),
    )
    .await
    .unwrap();
    record_payment(&pool, "B412", 700_000.0, "prior payment")
        .await
        .unwrap();

    let error = stay_lifecycle::check_out_at(
        &pool,
        CheckOutRequest {
            booking_id: "B412".to_string(),
            settlement_mode: CheckoutSettlementMode::Hourly,
            final_total: 500_000.0,
        },
        chrono::Local
            .with_ymd_and_hms(2026, 4, 20, 12, 0, 0)
            .single()
            .unwrap(),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("refund"));
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
async fn same_day_checkout_settlement_counts_one_room_sold_and_full_revenue() {
    let pool = test_pool().await;
    seed_room(&pool, "R420").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B420",
        "R420",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000.0,
        Some(0.0),
    )
    .await
    .unwrap();

    sqlx::query(
        "UPDATE bookings
         SET status = 'checked_out',
             actual_checkout = '2026-04-20T18:00:00+07:00',
             nights = 1,
             total_price = 500000,
             paid_amount = 500000,
             pricing_snapshot = ?
         WHERE id = ?",
    )
    .bind(r#"{"checkout_settlement":{"mode":"actual_nights","reporting_checkout":"2026-04-21","settled_nights":1,"settled_total":500000}}"#)
    .bind("B420")
    .execute(&pool)
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B420",
        250_000.0,
        "charge",
        "Room charge",
        "2026-04-20T08:00:00+07:00",
    )
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B420",
        -1_750_000.0,
        "charge",
        "Điều chỉnh checkout settlement",
        "2026-04-20T18:00:00+07:00",
    )
    .await
    .unwrap();

    let stats = revenue_queries::load_revenue_stats(&pool, "2026-04-20", "2026-04-20")
        .await
        .unwrap();
    let audit = audit_queries::load_night_audit_snapshot(&pool, "2026-04-20")
        .await
        .unwrap();

    assert_eq!(stats.total_revenue, 500_000.0);
    assert_eq!(stats.rooms_sold, 1);
    assert_eq!(audit.room_revenue, 500_000.0);
    assert_eq!(audit.rooms_sold, 1);
}

#[tokio::test]
async fn booked_nights_settlement_uses_reporting_checkout_for_financial_revenue() {
    let pool = test_pool().await;
    seed_room(&pool, "R421").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B421",
        "R421",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000.0,
        Some(2_500_000.0),
    )
    .await
    .unwrap();

    sqlx::query(
        "UPDATE bookings
         SET status = 'checked_out',
             actual_checkout = '2026-04-22T09:00:00+07:00',
             pricing_snapshot = ?
         WHERE id = ?",
    )
    .bind(r#"{"checkout_settlement":{"mode":"booked_nights","reporting_checkout":"2026-04-25","settled_nights":5,"settled_total":2500000}}"#)
    .bind("B421")
    .execute(&pool)
    .await
    .unwrap();

    let revenue = revenue_queries::load_room_revenue(&pool, "2026-04-20", "2026-04-24")
        .await
        .unwrap();

    assert_eq!(revenue, 2_500_000.0);
}

#[tokio::test]
async fn checkout_settlement_updates_booking_export_rows() {
    let pool = test_pool().await;
    seed_room(&pool, "R422").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B422",
        "R422",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000.0,
        Some(0.0),
    )
    .await
    .unwrap();

    sqlx::query(
        "UPDATE bookings
         SET status = 'checked_out',
             actual_checkout = '2026-04-20T18:00:00+07:00',
             nights = 1,
             total_price = 500000,
             paid_amount = 500000,
             pricing_snapshot = ?
         WHERE id = ?",
    )
    .bind(r#"{"checkout_settlement":{"mode":"actual_nights","reporting_checkout":"2026-04-21","settled_nights":1,"settled_total":500000}}"#)
    .bind("B422")
    .execute(&pool)
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B422",
        2_500_000.0,
        "charge",
        "Room charge",
        "2026-04-20T08:00:00+07:00",
    )
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B422",
        -2_000_000.0,
        "charge",
        "Điều chỉnh checkout settlement",
        "2026-04-20T18:00:00+07:00",
    )
    .await
    .unwrap();

    let export_rows = audit_queries::load_booking_export_rows(&pool, "2026-04-01", "2026-04-30")
        .await
        .unwrap();
    let row = export_rows.iter().find(|row| row.id == "B422").unwrap();

    assert_eq!(row.room_price, 500_000.0);
    assert_eq!(row.charge_total, 500_000.0);
    assert_eq!(row.recognized_revenue, 500_000.0);
}

#[tokio::test]
async fn checkout_settlement_export_rows_follow_reporting_checkout_boundary() {
    let pool = test_pool().await;
    seed_room(&pool, "R423").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B423",
        "R423",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000.0,
        Some(0.0),
    )
    .await
    .unwrap();

    sqlx::query(
        "UPDATE bookings
         SET status = 'checked_out',
             actual_checkout = '2026-04-20T18:00:00+07:00',
             nights = 1,
             total_price = 500000,
             paid_amount = 500000,
             pricing_snapshot = ?
         WHERE id = ?",
    )
    .bind(r#"{"checkout_settlement":{"mode":"actual_nights","reporting_checkout":"2026-04-21","settled_nights":1,"settled_total":500000}}"#)
    .bind("B423")
    .execute(&pool)
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B423",
        2_500_000.0,
        "charge",
        "Room charge",
        "2026-04-20T08:00:00+07:00",
    )
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B423",
        -2_000_000.0,
        "charge",
        "Điều chỉnh checkout settlement",
        "2026-04-20T18:00:00+07:00",
    )
    .await
    .unwrap();

    let export_rows = audit_queries::load_booking_export_rows(&pool, "2026-04-21", "2026-04-21")
        .await
        .unwrap();
    let row = export_rows.iter().find(|row| row.id == "B423").unwrap();

    assert_eq!(row.expected_checkout, "2026-04-21");
    assert_eq!(row.actual_checkout, "2026-04-20T18:00:00+07:00");
}

#[tokio::test]
async fn checkout_settlement_export_rows_exclude_original_checkin_window_after_shift() {
    let pool = test_pool().await;
    seed_room(&pool, "R424").await.unwrap();
    seed_active_booking_with_terms(
        &pool,
        "B424",
        "R424",
        "2026-04-20T08:00:00+07:00",
        "2026-04-25T12:00:00+07:00",
        5,
        2_500_000.0,
        Some(0.0),
    )
    .await
    .unwrap();

    sqlx::query(
        "UPDATE bookings
         SET status = 'checked_out',
             actual_checkout = '2026-04-20T18:00:00+07:00',
             nights = 1,
             total_price = 500000,
             paid_amount = 500000,
             pricing_snapshot = ?
         WHERE id = ?",
    )
    .bind(r#"{"checkout_settlement":{"mode":"actual_nights","reporting_checkout":"2026-04-21","settled_nights":1,"settled_total":500000}}"#)
    .bind("B424")
    .execute(&pool)
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B424",
        2_500_000.0,
        "charge",
        "Room charge",
        "2026-04-20T08:00:00+07:00",
    )
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B424",
        -2_000_000.0,
        "charge",
        "Điều chỉnh checkout settlement",
        "2026-04-20T18:00:00+07:00",
    )
    .await
    .unwrap();

    let export_rows = audit_queries::load_booking_export_rows(&pool, "2026-04-20", "2026-04-20")
        .await
        .unwrap();
    let row = export_rows.iter().find(|row| row.id == "B424");

    assert!(row.is_none());
}

#[tokio::test]
async fn cancellation_fee_export_uses_transaction_period_when_checkin_is_future() {
    let pool = test_pool().await;
    seed_room(&pool, "R425").await.unwrap();
    seed_booked_reservation(&pool, "B425", "R425")
        .await
        .unwrap();

    sqlx::query(
        "UPDATE bookings
         SET check_in_at = '2026-05-20',
             expected_checkout = '2026-05-22',
             scheduled_checkin = '2026-05-20',
             scheduled_checkout = '2026-05-22',
             status = 'cancelled'
         WHERE id = ?",
    )
    .bind("B425")
    .execute(&pool)
    .await
    .unwrap();
    seed_transaction(
        &pool,
        "B425",
        50_000.0,
        "cancellation_fee",
        "Retained deposit",
        "2026-04-15T14:00:00+07:00",
    )
    .await
    .unwrap();

    let export_rows = audit_queries::load_booking_export_rows(&pool, "2026-04-15", "2026-04-15")
        .await
        .unwrap();
    let row = export_rows.iter().find(|row| row.id == "B425").unwrap();

    assert_eq!(row.cancellation_fee_total, 50_000.0);
    assert_eq!(row.recognized_revenue, 50_000.0);
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

#[tokio::test]
async fn add_folio_line_idempotent_retry_replays_and_does_not_duplicate_row() {
    let pool = test_pool().await;
    seed_room(&pool, "FOLIO-IDEM-1").await.unwrap();
    seed_active_booking(&pool, "B-FOLIO-IDEM-1", "FOLIO-IDEM-1")
        .await
        .unwrap();
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-folio-idem-1",
        "idem-folio-line-1",
        "add_folio_line",
    );

    let first = add_folio_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-1",
        "laundry",
        "Laundry bundle",
        25_000.0,
        Some("staff-1"),
    )
    .await
    .expect("first folio line succeeds");
    let second = add_folio_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-1",
        "laundry",
        "Laundry bundle",
        25_000.0,
        Some("staff-1"),
    )
    .await
    .expect("retry replays");

    assert!(!first.replayed);
    assert!(second.replayed);
    assert_eq!(first.response["id"], second.response["id"]);

    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM folio_lines WHERE origin_idempotency_key = ?")
            .bind("add_folio_line:idem-folio-line-1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn add_folio_line_idempotent_accepts_uuid_booking_id_in_safe_ledger_metadata() {
    let pool = test_pool().await;
    seed_room(&pool, "FOLIO-IDEM-UUID").await.unwrap();
    let booking_id = uuid::Uuid::new_v4().to_string();
    seed_active_booking(&pool, &booking_id, "FOLIO-IDEM-UUID")
        .await
        .unwrap();
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-folio-idem-uuid",
        "idem-folio-line-uuid",
        "add_folio_line",
    );

    let result = add_folio_line_idempotent(
        &pool,
        &ctx,
        &booking_id,
        "laundry",
        "Laundry bundle",
        25_000.0,
        Some("staff-1"),
    )
    .await
    .expect("uuid booking id should not be rejected by safe ledger metadata");

    assert!(!result.replayed);
    assert_eq!(
        result.response["booking_id"].as_str(),
        Some(booking_id.as_str())
    );

    let primary_aggregate_key: Option<String> = sqlx::query_scalar(
        "SELECT primary_aggregate_key FROM command_idempotency
         WHERE command_name = 'add_folio_line' AND idempotency_key = ?",
    )
    .bind("idem-folio-line-uuid")
    .fetch_one(&pool)
    .await
    .unwrap();
    let expected_primary_aggregate_key = format!("booking:{booking_id}");
    assert_eq!(
        primary_aggregate_key.as_deref(),
        Some(expected_primary_aggregate_key.as_str())
    );
}

#[tokio::test]
async fn add_folio_line_idempotent_same_key_different_payload_conflicts() {
    let pool = test_pool().await;
    seed_room(&pool, "FOLIO-IDEM-2").await.unwrap();
    seed_active_booking(&pool, "B-FOLIO-IDEM-2", "FOLIO-IDEM-2")
        .await
        .unwrap();
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-folio-idem-2",
        "idem-folio-line-2",
        "add_folio_line",
    );

    add_folio_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-2",
        "laundry",
        "Laundry bundle",
        25_000.0,
        Some("staff-1"),
    )
    .await
    .expect("first folio line succeeds");

    let error = add_folio_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-2",
        "laundry",
        "Different description",
        25_000.0,
        Some("staff-1"),
    )
    .await
    .expect_err("same key with different payload conflicts");

    assert_eq!(
        error.code,
        crate::app_error::codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH
    );
}

#[tokio::test]
async fn add_folio_line_idempotent_replay_returns_stored_snapshot() {
    let pool = test_pool().await;
    seed_room(&pool, "FOLIO-IDEM-3").await.unwrap();
    seed_active_booking(&pool, "B-FOLIO-IDEM-3", "FOLIO-IDEM-3")
        .await
        .unwrap();
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-folio-idem-3",
        "idem-folio-line-3",
        "add_folio_line",
    );

    let first = add_folio_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-3",
        "laundry",
        "Snapshot line",
        25_000.0,
        Some("staff-1"),
    )
    .await
    .expect("first folio line succeeds");
    let line_id = first.response["id"].as_str().unwrap().to_string();
    let first_amount = first.response["amount"].as_f64().unwrap();

    sqlx::query("UPDATE folio_lines SET amount = 99_999.0 WHERE id = ?")
        .bind(&line_id)
        .execute(&pool)
        .await
        .unwrap();

    let replay = add_folio_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-3",
        "laundry",
        "Snapshot line",
        25_000.0,
        Some("staff-1"),
    )
    .await
    .expect("replay succeeds");

    assert!(replay.replayed);
    assert_eq!(replay.response["amount"].as_f64(), Some(first_amount));
    assert_ne!(replay.response["amount"].as_f64(), Some(99_999.0));
}

#[tokio::test]
async fn add_folio_line_idempotent_rejects_blank_key_before_any_write() {
    let pool = test_pool().await;
    seed_room(&pool, "FOLIO-IDEM-4").await.unwrap();
    seed_active_booking(&pool, "B-FOLIO-IDEM-4", "FOLIO-IDEM-4")
        .await
        .unwrap();

    let error = crate::command_idempotency::WriteCommandContext::for_scoped_command(
        "req-folio-idem-4",
        "   ",
        "add_folio_line",
    )
    .expect_err("blank idempotency key rejected");
    assert_eq!(
        error.code,
        crate::app_error::codes::IDEMPOTENCY_KEY_REQUIRED
    );

    let folio_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM folio_lines WHERE booking_id = ?")
            .bind("B-FOLIO-IDEM-4")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(folio_count, 0);

    let claim_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM command_idempotency
         WHERE command_name = 'add_folio_line' AND request_id = 'req-folio-idem-4'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(claim_count, 0);
}

#[tokio::test]
async fn add_folio_line_idempotent_invalid_amount_does_not_consume_claim_or_ordinal() {
    let pool = test_pool().await;
    seed_room(&pool, "FOLIO-IDEM-5").await.unwrap();
    seed_active_booking(&pool, "B-FOLIO-IDEM-5", "FOLIO-IDEM-5")
        .await
        .unwrap();
    let ctx = crate::command_idempotency::WriteCommandContext::for_internal_test(
        "req-folio-idem-5",
        "idem-folio-line-5",
        "add_folio_line",
    );

    let error = add_folio_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-5",
        "laundry",
        "Invalid amount",
        0.0,
        Some("staff-1"),
    )
    .await
    .expect_err("invalid amount rejected");
    assert_eq!(error.code, crate::app_error::codes::BOOKING_INVALID_STATE);

    let folio_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM folio_lines WHERE booking_id = ?")
            .bind("B-FOLIO-IDEM-5")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(folio_count, 0);

    let claim_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM command_idempotency
         WHERE command_name = 'add_folio_line' AND idempotency_key = ?",
    )
    .bind("idem-folio-line-5")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(claim_count, 0);

    let success = add_folio_line_idempotent(
        &pool,
        &ctx,
        "B-FOLIO-IDEM-5",
        "laundry",
        "Valid amount",
        15_000.0,
        Some("staff-1"),
    )
    .await
    .expect("valid amount succeeds");
    assert!(!success.replayed);

    let row = sqlx::query(
        "SELECT origin_idempotency_key, origin_line_ordinal
         FROM folio_lines
         WHERE booking_id = ?",
    )
    .bind("B-FOLIO-IDEM-5")
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        row.get::<String, _>("origin_idempotency_key"),
        "add_folio_line:idem-folio-line-5"
    );
    assert_eq!(row.get::<i64, _>("origin_line_ordinal"), 0);
}

#[tokio::test]
async fn folio_line_insert_rolls_back_with_parent_transaction() {
    let pool = test_pool().await;
    seed_room(&pool, "FOLIO-1").await.unwrap();
    seed_active_booking(&pool, "B-FOLIO-1", "FOLIO-1")
        .await
        .unwrap();

    let mut tx = crate::services::booking::support::begin_tx(&pool)
        .await
        .unwrap();
    crate::repositories::booking::folio_repository::insert_folio_line_tx(
        &mut tx,
        "B-FOLIO-1",
        "laundry",
        "Rollback laundry",
        25_000.0,
        Some("staff-1"),
        "2026-04-15T12:00:00+07:00",
    )
    .await
    .unwrap();
    tx.rollback().await.unwrap();

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM folio_lines WHERE booking_id = ?")
        .bind("B-FOLIO-1")
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(count, 0);
}

#[tokio::test]
async fn insert_folio_line_with_origin_writes_origin_key_and_ordinal() {
    let pool = test_pool().await;
    seed_room(&pool, "R503").await.unwrap();
    let booking_id = seed_booking_for_origin_tests(&pool, "R503").await.unwrap();
    let origin = OriginSideEffect::new("idem-folio-1", 0).unwrap();

    let mut tx = crate::services::booking::support::begin_tx(&pool)
        .await
        .unwrap();
    crate::repositories::booking::folio_repository::insert_folio_line_with_origin_tx(
        &mut tx,
        &booking_id,
        "laundry",
        "Laundry with origin",
        25_000.0,
        Some("staff-1"),
        "2026-04-27T08:00:00+07:00",
        &origin,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let row = sqlx::query(
        "SELECT origin_idempotency_key, origin_line_ordinal
         FROM folio_lines
         WHERE booking_id = ? AND description = ?",
    )
    .bind(&booking_id)
    .bind("Laundry with origin")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(
        row.get::<String, _>("origin_idempotency_key"),
        "idem-folio-1"
    );
    assert_eq!(row.get::<i64, _>("origin_line_ordinal"), 0);
}
