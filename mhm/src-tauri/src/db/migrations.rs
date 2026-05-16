use sqlx::{Pool, Sqlite};

use super::{execute_compat_alter, set_schema_version};

pub(super) async fn migrate_v1_base_schema(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS rooms (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                type        TEXT NOT NULL,
                floor       INTEGER NOT NULL,
                has_balcony INTEGER NOT NULL,
                base_price  INTEGER NOT NULL,
                status      TEXT NOT NULL DEFAULT 'vacant'
            )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS guests (
                id              TEXT PRIMARY KEY,
                guest_type      TEXT NOT NULL DEFAULT 'domestic',
                full_name       TEXT NOT NULL,
                doc_number      TEXT NOT NULL,
                dob             TEXT,
                gender          TEXT,
                nationality     TEXT DEFAULT 'Việt Nam',
                address         TEXT,
                visa_expiry     TEXT,
                scan_path       TEXT,
                created_at      TEXT NOT NULL
            )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS bookings (
                id                  TEXT PRIMARY KEY,
                room_id             TEXT NOT NULL REFERENCES rooms(id),
                primary_guest_id    TEXT NOT NULL REFERENCES guests(id),
                check_in_at         TEXT NOT NULL,
                expected_checkout   TEXT NOT NULL,
                actual_checkout     TEXT,
                nights              INTEGER NOT NULL,
                total_price         INTEGER NOT NULL,
                paid_amount         INTEGER DEFAULT 0,
                status              TEXT NOT NULL DEFAULT 'active',
                source              TEXT DEFAULT 'walk-in',
                notes               TEXT,
                created_at          TEXT NOT NULL
            )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS booking_guests (
                booking_id  TEXT NOT NULL REFERENCES bookings(id),
                guest_id    TEXT NOT NULL REFERENCES guests(id),
                PRIMARY KEY (booking_id, guest_id)
            )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS transactions (
                id          TEXT PRIMARY KEY,
                booking_id  TEXT NOT NULL REFERENCES bookings(id),
                amount      INTEGER NOT NULL,
                type        TEXT NOT NULL,
                note        TEXT,
                created_at  TEXT NOT NULL
            )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS expenses (
                id           TEXT PRIMARY KEY,
                category     TEXT NOT NULL,
                amount       INTEGER NOT NULL,
                note         TEXT,
                expense_date TEXT NOT NULL,
                created_at   TEXT NOT NULL
            )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS housekeeping (
                id           TEXT PRIMARY KEY,
                room_id      TEXT NOT NULL REFERENCES rooms(id),
                status       TEXT NOT NULL DEFAULT 'needs_cleaning',
                note         TEXT,
                triggered_at TEXT NOT NULL,
                cleaned_at   TEXT,
                created_at   TEXT NOT NULL
            )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS settings (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
    )
    .execute(&mut *tx)
    .await?;

    set_schema_version(&mut tx, 1).await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn migrate_v2_foundation_rbac(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    // Users table
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS users (
                id         TEXT PRIMARY KEY,
                name       TEXT NOT NULL,
                pin_hash   TEXT NOT NULL,
                role       TEXT NOT NULL DEFAULT 'receptionist',
                active     INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL
            )",
    )
    .execute(&mut *tx)
    .await?;

    // Audit logs table
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS audit_logs (
                id          TEXT PRIMARY KEY,
                user_id     TEXT,
                action      TEXT NOT NULL,
                entity_type TEXT NOT NULL,
                entity_id   TEXT,
                details     TEXT,
                created_at  TEXT NOT NULL
            )",
    )
    .execute(&mut *tx)
    .await?;

    // Add phone and notes to guests
    // Using IF NOT EXISTS pattern: try ALTER, ignore if already exists
    execute_compat_alter(&mut tx, "ALTER TABLE guests ADD COLUMN phone TEXT").await?;
    execute_compat_alter(&mut tx, "ALTER TABLE guests ADD COLUMN notes TEXT").await?;

    // Add payment_method and created_by to transactions
    execute_compat_alter(
        &mut tx,
        "ALTER TABLE transactions ADD COLUMN payment_method TEXT DEFAULT 'cash'",
    )
    .await?;
    execute_compat_alter(
        &mut tx,
        "ALTER TABLE transactions ADD COLUMN created_by TEXT",
    )
    .await?;

    // Add created_by to bookings
    execute_compat_alter(&mut tx, "ALTER TABLE bookings ADD COLUMN created_by TEXT").await?;

    set_schema_version(&mut tx, 2).await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn migrate_v3_pricing_engine(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    // pricing_rules: per room_type configuration
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS pricing_rules (
                id              TEXT PRIMARY KEY,
                room_type       TEXT NOT NULL,
                hourly_rate     INTEGER NOT NULL DEFAULT 0,
                overnight_rate  INTEGER NOT NULL DEFAULT 0,
                daily_rate      INTEGER NOT NULL DEFAULT 0,
                overnight_start TEXT NOT NULL DEFAULT '22:00',
                overnight_end   TEXT NOT NULL DEFAULT '11:00',
                daily_checkin   TEXT NOT NULL DEFAULT '14:00',
                daily_checkout  TEXT NOT NULL DEFAULT '12:00',
                early_checkin_surcharge_pct REAL NOT NULL DEFAULT 30,
                late_checkout_surcharge_pct REAL NOT NULL DEFAULT 30,
                weekend_uplift_pct  REAL NOT NULL DEFAULT 0,
                created_at      TEXT NOT NULL,
                updated_at      TEXT NOT NULL,
                UNIQUE(room_type)
            )",
    )
    .execute(&mut *tx)
    .await?;

    // special_dates: holiday/weekend overrides
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS special_dates (
                id          TEXT PRIMARY KEY,
                date        TEXT NOT NULL,
                label       TEXT NOT NULL DEFAULT '',
                uplift_pct  REAL NOT NULL DEFAULT 0,
                created_at  TEXT NOT NULL,
                UNIQUE(date)
            )",
    )
    .execute(&mut *tx)
    .await?;

    // Add pricing_snapshot to bookings (JSON)
    execute_compat_alter(
        &mut tx,
        "ALTER TABLE bookings ADD COLUMN pricing_snapshot TEXT",
    )
    .await?;

    // Add pricing_type to bookings
    execute_compat_alter(
        &mut tx,
        "ALTER TABLE bookings ADD COLUMN pricing_type TEXT DEFAULT 'nightly'",
    )
    .await?;

    set_schema_version(&mut tx, 3).await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn migrate_v4_folio_billing_night_audit(
    pool: &Pool<Sqlite>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    // folio_lines: per-booking itemized charges
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS folio_lines (
                id          TEXT PRIMARY KEY,
                booking_id  TEXT NOT NULL REFERENCES bookings(id),
                category    TEXT NOT NULL,
                description TEXT NOT NULL,
                amount      INTEGER NOT NULL,
                created_by  TEXT,
                created_at  TEXT NOT NULL
            )",
    )
    .execute(&mut *tx)
    .await?;

    // night_audit_logs: daily revenue snapshots
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS night_audit_logs (
                id              TEXT PRIMARY KEY,
                audit_date      TEXT NOT NULL,
                total_revenue   INTEGER NOT NULL DEFAULT 0,
                room_revenue    INTEGER NOT NULL DEFAULT 0,
                folio_revenue   INTEGER NOT NULL DEFAULT 0,
                total_expenses  INTEGER NOT NULL DEFAULT 0,
                occupancy_pct   REAL NOT NULL DEFAULT 0,
                rooms_sold      INTEGER NOT NULL DEFAULT 0,
                total_rooms     INTEGER NOT NULL DEFAULT 0,
                notes           TEXT,
                created_by      TEXT,
                created_at      TEXT NOT NULL,
                UNIQUE(audit_date)
            )",
    )
    .execute(&mut *tx)
    .await?;

    // Add is_audited flag to bookings
    execute_compat_alter(
        &mut tx,
        "ALTER TABLE bookings ADD COLUMN is_audited INTEGER DEFAULT 0",
    )
    .await?;

    set_schema_version(&mut tx, 4).await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn migrate_v5_dynamic_room_config(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    // room_types: admin creates these first, rooms reference them
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS room_types (
                id         TEXT PRIMARY KEY,
                name       TEXT NOT NULL UNIQUE,
                created_at TEXT NOT NULL
            )",
    )
    .execute(&mut *tx)
    .await?;

    // Seed default room types from existing rooms
    sqlx::query(
        "INSERT OR IGNORE INTO room_types (id, name, created_at)
             SELECT DISTINCT lower(type), type, datetime('now') FROM rooms",
    )
    .execute(&mut *tx)
    .await?;

    // Add per-person pricing columns
    execute_compat_alter(
        &mut tx,
        "ALTER TABLE rooms ADD COLUMN max_guests INTEGER NOT NULL DEFAULT 2",
    )
    .await?;
    execute_compat_alter(
        &mut tx,
        "ALTER TABLE rooms ADD COLUMN extra_person_fee INTEGER NOT NULL DEFAULT 0",
    )
    .await?;

    set_schema_version(&mut tx, 5).await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn migrate_v6_reservation_calendar(
    pool: &Pool<Sqlite>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    // room_calendar: each row = 1 day blocked for 1 room
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS room_calendar (
                room_id    TEXT NOT NULL REFERENCES rooms(id) ON DELETE CASCADE,
                date       TEXT NOT NULL,
                booking_id TEXT REFERENCES bookings(id) ON DELETE CASCADE,
                status     TEXT NOT NULL DEFAULT 'booked',
                PRIMARY KEY (room_id, date)
            )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query("CREATE INDEX IF NOT EXISTS idx_calendar_booking ON room_calendar(booking_id)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_calendar_status ON room_calendar(room_id, status)")
        .execute(&mut *tx)
        .await?;

    // Add reservation fields to bookings
    execute_compat_alter(
        &mut tx,
        "ALTER TABLE bookings ADD COLUMN booking_type TEXT DEFAULT 'walk-in'",
    )
    .await?;
    execute_compat_alter(
        &mut tx,
        "ALTER TABLE bookings ADD COLUMN deposit_amount INTEGER DEFAULT 0",
    )
    .await?;
    execute_compat_alter(&mut tx, "ALTER TABLE bookings ADD COLUMN guest_phone TEXT").await?;
    execute_compat_alter(
        &mut tx,
        "ALTER TABLE bookings ADD COLUMN scheduled_checkin TEXT",
    )
    .await?;
    execute_compat_alter(
        &mut tx,
        "ALTER TABLE bookings ADD COLUMN scheduled_checkout TEXT",
    )
    .await?;

    set_schema_version(&mut tx, 6).await?;
    tx.commit().await?;
    Ok(())
}
