use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};

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
            base_price INTEGER NOT NULL DEFAULT 0,
            max_guests INTEGER NOT NULL DEFAULT 2,
            extra_person_fee INTEGER NOT NULL DEFAULT 0,
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
            total_price INTEGER NOT NULL,
            paid_amount INTEGER,
            status TEXT NOT NULL,
            source TEXT,
            notes TEXT,
            created_by TEXT,
            booking_type TEXT DEFAULT 'walk-in',
            pricing_type TEXT DEFAULT 'nightly',
            deposit_amount INTEGER,
            guest_phone TEXT,
            scheduled_checkin TEXT,
            scheduled_checkout TEXT,
            group_id TEXT REFERENCES booking_groups(id),
            is_master_room INTEGER NOT NULL DEFAULT 0,
            is_audited INTEGER NOT NULL DEFAULT 0,
            pricing_snapshot TEXT,
            guests INTEGER,
            rate_overridden_at TEXT,
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
        "CREATE TABLE group_services (
            id TEXT PRIMARY KEY,
            group_id TEXT NOT NULL REFERENCES booking_groups(id),
            booking_id TEXT REFERENCES bookings(id),
            name TEXT NOT NULL,
            quantity INTEGER NOT NULL DEFAULT 1,
            unit_price INTEGER NOT NULL,
            total_price INTEGER NOT NULL,
            note TEXT,
            created_by TEXT,
            created_at TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create group_services table");

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
            amount INTEGER NOT NULL,
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
            hourly_rate INTEGER NOT NULL DEFAULT 0,
            overnight_rate INTEGER NOT NULL DEFAULT 0,
            daily_rate INTEGER NOT NULL DEFAULT 0,
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
            amount INTEGER NOT NULL,
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
            amount INTEGER NOT NULL,
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
            total_revenue INTEGER NOT NULL DEFAULT 0,
            room_revenue INTEGER NOT NULL DEFAULT 0,
            folio_revenue INTEGER NOT NULL DEFAULT 0,
            total_expenses INTEGER NOT NULL DEFAULT 0,
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
            recovery_dismissed_at TEXT,
            recovery_dismissed_by TEXT,
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

    sqlx::query(
        "CREATE TABLE outbox_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            event_type TEXT NOT NULL,
            aggregate_key TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            origin_request_id TEXT NOT NULL,
            origin_idempotency_key TEXT NOT NULL,
            origin_command_name TEXT NOT NULL,
            origin_request_hash TEXT NOT NULL,
            status TEXT NOT NULL,
            worker_token TEXT,
            attempts INTEGER NOT NULL DEFAULT 0,
            next_attempt_at TEXT,
            processing_started_at TEXT,
            processing_expires_at TEXT,
            last_error TEXT,
            created_at TEXT NOT NULL,
            dispatched_at TEXT
        )",
    )
    .execute(&pool)
    .await
    .expect("failed to create outbox_events table");

    sqlx::query(
        "CREATE UNIQUE INDEX outbox_events_origin_command_uq
         ON outbox_events (origin_command_name, origin_idempotency_key)",
    )
    .execute(&pool)
    .await
    .expect("failed to create outbox origin command index");

    pool
}

pub async fn shared_file_test_pools(
    label: &str,
) -> (Pool<Sqlite>, Pool<Sqlite>, std::path::PathBuf) {
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
