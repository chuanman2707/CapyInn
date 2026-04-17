use log::warn;
use sqlx::{Pool, Row, Sqlite, SqlitePool, Transaction};

use crate::app_identity;

pub async fn init_db() -> Result<Pool<Sqlite>, sqlx::Error> {
    let db_dir = app_identity::runtime_root();
    std::fs::create_dir_all(&db_dir).expect("Cannot create runtime directory");

    let db_path = app_identity::database_path();
    let db_url = format!("sqlite:{}?mode=rwc", db_path.display());

    let pool = SqlitePool::connect(&db_url).await?;

    sqlx::query("PRAGMA journal_mode=WAL;")
        .execute(&pool)
        .await?;
    sqlx::query("PRAGMA foreign_keys=ON;")
        .execute(&pool)
        .await?;

    run_migrations(&pool).await?;
    ensure_setting_default(&pool, "setup_completed", "false").await?;

    Ok(pool)
}

async fn ensure_setting_default(
    pool: &Pool<Sqlite>,
    key: &str,
    value: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT OR IGNORE INTO settings (key, value) VALUES (?, ?)")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await?;
    Ok(())
}

// ─── Versioned Inline Migrations ───

async fn get_schema_version(pool: &Pool<Sqlite>) -> Result<i32, sqlx::Error> {
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER NOT NULL DEFAULT 0
        )",
    )
    .execute(pool)
    .await?;

    let row = sqlx::query("SELECT version FROM schema_version LIMIT 1")
        .fetch_optional(pool)
        .await?;

    match row {
        Some(row) => Ok(row.get::<i32, _>("version")),
        None => {
            sqlx::query("INSERT INTO schema_version (version) VALUES (0)")
                .execute(pool)
                .await?;
            Ok(0)
        }
    }
}

async fn set_schema_version(
    executor: &mut Transaction<'_, Sqlite>,
    version: i32,
) -> Result<(), sqlx::Error> {
    let result = sqlx::query("UPDATE schema_version SET version = ?")
        .bind(version)
        .execute(&mut **executor)
        .await?;

    if result.rows_affected() == 0 {
        sqlx::query("INSERT INTO schema_version (version) VALUES (?)")
            .bind(version)
            .execute(&mut **executor)
            .await?;
    }

    Ok(())
}

async fn execute_compat_alter(
    tx: &mut Transaction<'_, Sqlite>,
    sql: &str,
) -> Result<(), sqlx::Error> {
    match sqlx::query(sql).execute(&mut **tx).await {
        Ok(_) => Ok(()),
        Err(error) if is_duplicate_column_error(&error) => {
            warn!("Ignoring compatibility migration '{}': {}", sql, error);
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn is_duplicate_column_error(error: &sqlx::Error) -> bool {
    let message = error.to_string().to_lowercase();
    message.contains("duplicate column name") || message.contains("already exists")
}

pub(crate) async fn run_migrations(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let current = get_schema_version(pool).await?;

    // ── V0: Base schema (original tables) ──
    if current < 1 {
        let mut tx = pool.begin().await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS rooms (
                id          TEXT PRIMARY KEY,
                name        TEXT NOT NULL,
                type        TEXT NOT NULL,
                floor       INTEGER NOT NULL,
                has_balcony INTEGER NOT NULL,
                base_price  REAL NOT NULL,
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
                total_price         REAL NOT NULL,
                paid_amount         REAL DEFAULT 0,
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
                amount      REAL NOT NULL,
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
                amount       REAL NOT NULL,
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
    }

    // ── V2: Phase 1 — Foundation + RBAC ──
    if current < 2 {
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
        execute_compat_alter(&mut tx, "ALTER TABLE transactions ADD COLUMN created_by TEXT")
            .await?;

        // Add created_by to bookings
        execute_compat_alter(&mut tx, "ALTER TABLE bookings ADD COLUMN created_by TEXT").await?;

        set_schema_version(&mut tx, 2).await?;
        tx.commit().await?;
    }

    // ── V3: Phase 2 — Pricing Engine ──
    if current < 3 {
        let mut tx = pool.begin().await?;

        // pricing_rules: per room_type configuration
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS pricing_rules (
                id              TEXT PRIMARY KEY,
                room_type       TEXT NOT NULL,
                hourly_rate     REAL NOT NULL DEFAULT 0,
                overnight_rate  REAL NOT NULL DEFAULT 0,
                daily_rate      REAL NOT NULL DEFAULT 0,
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
        execute_compat_alter(&mut tx, "ALTER TABLE bookings ADD COLUMN pricing_snapshot TEXT")
            .await?;

        // Add pricing_type to bookings
        execute_compat_alter(
            &mut tx,
            "ALTER TABLE bookings ADD COLUMN pricing_type TEXT DEFAULT 'nightly'",
        )
        .await?;

        set_schema_version(&mut tx, 3).await?;
        tx.commit().await?;
    }

    // ── V4: Phase 3+4 — Folio/Billing + Night Audit ──
    if current < 4 {
        let mut tx = pool.begin().await?;

        // folio_lines: per-booking itemized charges
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS folio_lines (
                id          TEXT PRIMARY KEY,
                booking_id  TEXT NOT NULL REFERENCES bookings(id),
                category    TEXT NOT NULL,
                description TEXT NOT NULL,
                amount      REAL NOT NULL,
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
                total_revenue   REAL NOT NULL DEFAULT 0,
                room_revenue    REAL NOT NULL DEFAULT 0,
                folio_revenue   REAL NOT NULL DEFAULT 0,
                total_expenses  REAL NOT NULL DEFAULT 0,
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
        execute_compat_alter(&mut tx, "ALTER TABLE bookings ADD COLUMN is_audited INTEGER DEFAULT 0")
            .await?;

        set_schema_version(&mut tx, 4).await?;
        tx.commit().await?;
    }

    // ── V5: Dynamic Room Config — room_types table + per-person pricing ──
    if current < 5 {
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
            "ALTER TABLE rooms ADD COLUMN extra_person_fee REAL NOT NULL DEFAULT 0",
        )
        .await?;

        set_schema_version(&mut tx, 5).await?;
        tx.commit().await?;
    }

    // ── V6: Reservation Calendar Block System ──
    if current < 6 {
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
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_calendar_status ON room_calendar(room_id, status)",
        )
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
            "ALTER TABLE bookings ADD COLUMN deposit_amount REAL DEFAULT 0",
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
    }

    // ── V7: MCP Gateway — API Key Storage ──
    if current < 7 {
        let mut tx = pool.begin().await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS gateway_api_keys (
                id TEXT PRIMARY KEY,
                key_hash TEXT NOT NULL,
                label TEXT DEFAULT 'default',
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                last_used_at TEXT
            )",
        )
        .execute(&mut *tx)
        .await?;

        set_schema_version(&mut tx, 7).await?;
        tx.commit().await?;
    }

    // ── V8: Invoice PDF System ──
    if current < 8 {
        let mut tx = pool.begin().await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS invoices (
                id                TEXT PRIMARY KEY,
                invoice_number    TEXT NOT NULL UNIQUE,
                booking_id        TEXT NOT NULL REFERENCES bookings(id),
                hotel_name        TEXT NOT NULL,
                hotel_address     TEXT NOT NULL,
                hotel_phone       TEXT NOT NULL,
                guest_name        TEXT NOT NULL,
                guest_phone       TEXT,
                room_name         TEXT NOT NULL,
                room_type         TEXT NOT NULL,
                check_in          TEXT NOT NULL,
                check_out         TEXT NOT NULL,
                nights            INTEGER NOT NULL,
                pricing_breakdown TEXT NOT NULL,
                subtotal          REAL NOT NULL,
                deposit_amount    REAL NOT NULL DEFAULT 0,
                total             REAL NOT NULL,
                balance_due       REAL NOT NULL,
                policy_text       TEXT,
                notes             TEXT,
                status            TEXT NOT NULL DEFAULT 'issued',
                created_at        TEXT NOT NULL
            )",
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query("CREATE INDEX IF NOT EXISTS idx_invoices_booking ON invoices(booking_id)")
            .execute(&mut *tx)
            .await?;

        set_schema_version(&mut tx, 8).await?;
        tx.commit().await?;
    }

    // ── V9: Group Booking System ──
    if current < 9 {
        let mut tx = pool.begin().await?;

        // booking_groups: group metadata
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS booking_groups (
                id                TEXT PRIMARY KEY,
                group_name        TEXT NOT NULL,
                master_booking_id TEXT,
                organizer_name    TEXT NOT NULL,
                organizer_phone   TEXT,
                total_rooms       INTEGER NOT NULL,
                status            TEXT NOT NULL DEFAULT 'active',
                notes             TEXT,
                created_by        TEXT,
                created_at        TEXT NOT NULL
            )",
        )
        .execute(&mut *tx)
        .await?;

        // group_services: per-group add-on charges
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS group_services (
                id          TEXT PRIMARY KEY,
                group_id    TEXT NOT NULL REFERENCES booking_groups(id),
                booking_id  TEXT REFERENCES bookings(id),
                name        TEXT NOT NULL,
                quantity    INTEGER NOT NULL DEFAULT 1,
                unit_price  REAL NOT NULL,
                total_price REAL NOT NULL,
                note        TEXT,
                created_by  TEXT,
                created_at  TEXT NOT NULL
            )",
        )
        .execute(&mut *tx)
        .await?;

        // Add group columns to bookings
        execute_compat_alter(
            &mut tx,
            "ALTER TABLE bookings ADD COLUMN group_id TEXT REFERENCES booking_groups(id)",
        )
        .await?;
        execute_compat_alter(
            &mut tx,
            "ALTER TABLE bookings ADD COLUMN is_master_room INTEGER DEFAULT 0",
        )
        .await?;

        // Indexes
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_bookings_group ON bookings(group_id)")
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_group_services_group ON group_services(group_id)",
        )
        .execute(&mut *tx)
        .await?;

        set_schema_version(&mut tx, 9).await?;
        tx.commit().await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{execute_compat_alter, get_schema_version, run_migrations};
    use sqlx::{Row, SqlitePool};

    #[tokio::test]
    async fn migrations_bootstrap_schema_version_row() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");

        let version = get_schema_version(&pool)
            .await
            .expect("bootstraps schema version state");

        assert_eq!(version, 0);

        let count: i64 = sqlx::query("SELECT COUNT(*) AS count FROM schema_version")
            .fetch_one(&pool)
            .await
            .expect("reads schema_version row")
            .get("count");

        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn migrations_ignore_duplicate_columns_in_compat_alters() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");

        sqlx::query("CREATE TABLE sample (existing TEXT)")
            .execute(&pool)
            .await
            .expect("creates sample table");

        let mut tx = pool.begin().await.expect("starts test tx");
        execute_compat_alter(&mut tx, "ALTER TABLE sample ADD COLUMN existing TEXT")
            .await
            .expect("duplicate column compatibility path is ignored");
        tx.commit().await.expect("commits tx");
    }

    #[tokio::test]
    async fn migrations_run_to_latest_schema_version() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");

        run_migrations(&pool).await.expect("runs migrations");

        let version: i32 = sqlx::query("SELECT version FROM schema_version LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("reads final schema version")
            .get("version");

        assert_eq!(version, 9);
    }
}
