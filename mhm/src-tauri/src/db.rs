use log::warn;
use sqlx::{
    sqlite::{
        SqliteConnectOptions, SqliteConnection, SqliteJournalMode, SqlitePoolOptions,
        SqliteSynchronous,
    },
    Pool, Row, Sqlite, Transaction,
};
use std::{str::FromStr, time::Duration};

use crate::app_identity;

const SQLITE_BUSY_TIMEOUT_MS: u64 = 5000;

pub async fn init_db() -> Result<Pool<Sqlite>, sqlx::Error> {
    let db_dir = app_identity::runtime_root();
    std::fs::create_dir_all(&db_dir).expect("Cannot create runtime directory");
    std::fs::create_dir_all(app_identity::diagnostics_dir())
        .expect("Cannot create diagnostics directory");

    let db_path = app_identity::database_path();
    let db_url = format!("sqlite:{}?mode=rwc", db_path.display());

    let pool = connect_configured_sqlite_pool(&db_url).await?;

    run_migrations(&pool).await?;
    ensure_setting_default(&pool, "setup_completed", "false").await?;
    ensure_setting_default(&pool, "send_crash_reports", "false").await?;

    Ok(pool)
}

pub(crate) async fn connect_configured_sqlite_pool(
    db_url: &str,
) -> Result<Pool<Sqlite>, sqlx::Error> {
    let options = SqliteConnectOptions::from_str(db_url)?
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_millis(SQLITE_BUSY_TIMEOUT_MS))
        .synchronous(SqliteSynchronous::Normal);

    SqlitePoolOptions::new()
        .after_connect(|connection, _metadata| {
            Box::pin(async move {
                configure_sqlite_connection(connection).await?;
                verify_sqlite_connection_pragmas(connection).await
            })
        })
        .connect_with(options)
        .await
}

async fn configure_sqlite_connection(connection: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    sqlx::query("PRAGMA foreign_keys=ON;")
        .execute(&mut *connection)
        .await?;
    sqlx::query("PRAGMA busy_timeout=5000;")
        .execute(&mut *connection)
        .await?;
    sqlx::query("PRAGMA synchronous=NORMAL;")
        .execute(&mut *connection)
        .await?;

    Ok(())
}

async fn verify_sqlite_connection_pragmas(
    connection: &mut SqliteConnection,
) -> Result<(), sqlx::Error> {
    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys;")
        .fetch_one(&mut *connection)
        .await?;
    if foreign_keys != 1 {
        return Err(sqlite_pragma_mismatch("foreign_keys", "1", foreign_keys));
    }

    let busy_timeout: i64 = sqlx::query_scalar("PRAGMA busy_timeout;")
        .fetch_one(&mut *connection)
        .await?;
    if busy_timeout != SQLITE_BUSY_TIMEOUT_MS as i64 {
        return Err(sqlite_pragma_mismatch("busy_timeout", "5000", busy_timeout));
    }

    let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous;")
        .fetch_one(&mut *connection)
        .await?;
    if synchronous != 1 {
        return Err(sqlite_pragma_mismatch("synchronous", "1", synchronous));
    }

    Ok(())
}

fn sqlite_pragma_mismatch(name: &str, expected: &str, actual: i64) -> sqlx::Error {
    sqlx::Error::Protocol(format!(
        "SQLite PRAGMA {} expected {}, got {}",
        name, expected, actual
    ))
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
        execute_compat_alter(
            &mut tx,
            "ALTER TABLE transactions ADD COLUMN created_by TEXT",
        )
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
        execute_compat_alter(
            &mut tx,
            "ALTER TABLE bookings ADD COLUMN is_audited INTEGER DEFAULT 0",
        )
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

    // ── V10: Command Idempotency ──
    if current < 10 {
        let mut tx = pool.begin().await?;

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS command_idempotency (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                idempotency_key TEXT NOT NULL,
                command_name TEXT NOT NULL,
                request_hash TEXT NOT NULL,
                intent_json TEXT NOT NULL,
                primary_aggregate_key TEXT,
                lock_keys_json TEXT NOT NULL,
                status TEXT NOT NULL,
                claim_token TEXT NOT NULL,
                response_json TEXT,
                error_code TEXT,
                retryable INTEGER NOT NULL DEFAULT 0,
                lease_expires_at TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                completed_at TEXT,
                last_attempt_at TEXT,
                UNIQUE(command_name, idempotency_key)
            )",
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS command_idempotency_lease_idx
             ON command_idempotency(lease_expires_at)
             WHERE status = 'in_progress'",
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS command_idempotency_completed_idx
             ON command_idempotency(completed_at)
             WHERE status IN ('completed', 'failed_terminal')",
        )
        .execute(&mut *tx)
        .await?;

        set_schema_version(&mut tx, 10).await?;
        tx.commit().await?;
    }

    // ── V11: Command terminal error replay payload ──
    if current < 11 {
        let mut tx = pool.begin().await?;

        execute_compat_alter(
            &mut tx,
            "ALTER TABLE command_idempotency ADD COLUMN error_json TEXT",
        )
        .await?;

        set_schema_version(&mut tx, 11).await?;
        tx.commit().await?;
    }

    // ── V12: Operator-ready command ledger metadata ──
    if current < 12 {
        let mut tx = pool.begin().await?;

        for alter in [
            "ALTER TABLE command_idempotency ADD COLUMN request_id TEXT",
            "ALTER TABLE command_idempotency ADD COLUMN actor_type TEXT NOT NULL DEFAULT 'system'",
            "ALTER TABLE command_idempotency ADD COLUMN actor_id TEXT",
            "ALTER TABLE command_idempotency ADD COLUMN client_id TEXT",
            "ALTER TABLE command_idempotency ADD COLUMN session_id TEXT",
            "ALTER TABLE command_idempotency ADD COLUMN channel_id TEXT",
            "ALTER TABLE command_idempotency ADD COLUMN issued_at TEXT",
            "ALTER TABLE command_idempotency ADD COLUMN summary_json TEXT NOT NULL DEFAULT '{}'",
            "ALTER TABLE command_idempotency ADD COLUMN result_summary_json TEXT",
            "ALTER TABLE command_idempotency ADD COLUMN error_summary_json TEXT",
        ] {
            execute_compat_alter(&mut tx, alter).await?;
        }

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS command_idempotency_attention_status_idx
             ON command_idempotency(status, updated_at)
             WHERE status IN ('failed_retryable', 'failed_terminal')",
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "CREATE INDEX IF NOT EXISTS command_idempotency_primary_aggregate_idx
             ON command_idempotency(primary_aggregate_key, updated_at)
             WHERE primary_aggregate_key IS NOT NULL",
        )
        .execute(&mut *tx)
        .await?;

        set_schema_version(&mut tx, 12).await?;
        tx.commit().await?;
    }

    // ── V13: Origin idempotency on ledger and folio rows ──
    if current < 13 {
        let mut tx = pool.begin().await?;

        for alter in [
            "ALTER TABLE transactions ADD COLUMN origin_idempotency_key TEXT",
            "ALTER TABLE transactions ADD COLUMN origin_transaction_ordinal INTEGER NOT NULL DEFAULT 0",
            "ALTER TABLE folio_lines ADD COLUMN origin_idempotency_key TEXT",
            "ALTER TABLE folio_lines ADD COLUMN origin_line_ordinal INTEGER NOT NULL DEFAULT 0",
        ] {
            execute_compat_alter(&mut tx, alter).await?;
        }

        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS transactions_origin_idem_uq
             ON transactions (booking_id, origin_idempotency_key, origin_transaction_ordinal)
             WHERE origin_idempotency_key IS NOT NULL AND origin_idempotency_key != ''",
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS folio_lines_origin_idem_uq
             ON folio_lines (booking_id, origin_idempotency_key, origin_line_ordinal)
             WHERE origin_idempotency_key IS NOT NULL AND origin_idempotency_key != ''",
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS transactions_origin_command_uq
             ON transactions (origin_idempotency_key, origin_transaction_ordinal)
             WHERE origin_idempotency_key IS NOT NULL AND origin_idempotency_key != ''",
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS folio_lines_origin_command_uq
             ON folio_lines (origin_idempotency_key, origin_line_ordinal)
             WHERE origin_idempotency_key IS NOT NULL AND origin_idempotency_key != ''",
        )
        .execute(&mut *tx)
        .await?;

        set_schema_version(&mut tx, 13).await?;
        tx.commit().await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        connect_configured_sqlite_pool, execute_compat_alter, get_schema_version, run_migrations,
    };
    use sqlx::{Row, SqlitePool};

    #[tokio::test]
    async fn configured_pool_applies_connection_pragmas() {
        let pool = connect_configured_sqlite_pool("sqlite::memory:")
            .await
            .expect("connects configured in-memory sqlite pool");

        let mut first = pool.acquire().await.expect("acquires first connection");
        let mut second = pool.acquire().await.expect("acquires second connection");

        for connection in [&mut first, &mut second] {
            let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys;")
                .fetch_one(&mut **connection)
                .await
                .expect("reads foreign_keys pragma");
            let busy_timeout: i64 = sqlx::query_scalar("PRAGMA busy_timeout;")
                .fetch_one(&mut **connection)
                .await
                .expect("reads busy_timeout pragma");
            let synchronous: i64 = sqlx::query_scalar("PRAGMA synchronous;")
                .fetch_one(&mut **connection)
                .await
                .expect("reads synchronous pragma");

            assert_eq!(foreign_keys, 1);
            assert_eq!(busy_timeout, 5000);
            assert_eq!(synchronous, 1);
        }
    }

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

    async fn command_idempotency_column_count(pool: &SqlitePool, name: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT COUNT(*)
             FROM pragma_table_info('command_idempotency')
             WHERE name = ?",
        )
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("checks command_idempotency column")
    }

    async fn table_column_count(pool: &SqlitePool, table: &str, name: &str) -> i64 {
        let sql = match table {
            "transactions" => {
                "SELECT COUNT(*) FROM pragma_table_info('transactions') WHERE name = ?"
            }
            "folio_lines" => "SELECT COUNT(*) FROM pragma_table_info('folio_lines') WHERE name = ?",
            _ => panic!("unsupported table {table}"),
        };

        sqlx::query_scalar(sql)
            .bind(name)
            .fetch_one(pool)
            .await
            .expect("checks table column")
    }

    async fn sqlite_index_count(pool: &SqlitePool, name: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT COUNT(*)
             FROM sqlite_master
             WHERE type = 'index' AND name = ?",
        )
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("checks sqlite index")
    }

    async fn assert_command_ledger_v12_shape(pool: &SqlitePool) {
        for column in [
            "request_id",
            "actor_type",
            "actor_id",
            "client_id",
            "session_id",
            "channel_id",
            "issued_at",
            "summary_json",
            "result_summary_json",
            "error_summary_json",
        ] {
            assert_eq!(
                command_idempotency_column_count(pool, column).await,
                1,
                "missing command_idempotency column {column}"
            );
        }

        assert_eq!(
            sqlite_index_count(pool, "command_idempotency_attention_status_idx").await,
            1
        );
        assert_eq!(
            sqlite_index_count(pool, "command_idempotency_primary_aggregate_idx").await,
            1
        );
    }

    async fn create_legacy_billing_tables_for_partial_upgrade(pool: &SqlitePool) {
        sqlx::query(
            "CREATE TABLE transactions (
                id          TEXT PRIMARY KEY,
                booking_id  TEXT NOT NULL,
                amount      REAL NOT NULL,
                type        TEXT NOT NULL,
                note        TEXT,
                created_at  TEXT NOT NULL
            )",
        )
        .execute(pool)
        .await
        .expect("creates legacy transactions table");

        sqlx::query(
            "CREATE TABLE folio_lines (
                id          TEXT PRIMARY KEY,
                booking_id  TEXT NOT NULL,
                category    TEXT NOT NULL,
                description TEXT NOT NULL,
                amount      REAL NOT NULL,
                created_by  TEXT,
                created_at  TEXT NOT NULL
            )",
        )
        .execute(pool)
        .await
        .expect("creates legacy folio_lines table");
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

        assert_eq!(version, 13);
    }

    #[tokio::test]
    async fn migration_v10_creates_command_idempotency_table() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");

        run_migrations(&pool).await.expect("runs migrations");

        let table_count: i64 = sqlx::query(
            "SELECT COUNT(*) AS count
             FROM sqlite_master
             WHERE type = 'table' AND name = 'command_idempotency'",
        )
        .fetch_one(&pool)
        .await
        .expect("reads sqlite_master")
        .get("count");

        assert_eq!(table_count, 1);
    }

    #[tokio::test]
    async fn migration_v11_adds_command_error_json_on_fresh_migration() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");

        run_migrations(&pool).await.expect("runs migrations");

        assert_eq!(
            command_idempotency_column_count(&pool, "error_json").await,
            1
        );
    }

    #[tokio::test]
    async fn migration_v11_upgrades_existing_v10_command_idempotency_table() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");

        get_schema_version(&pool)
            .await
            .expect("bootstraps schema version state");

        sqlx::query(
            "CREATE TABLE command_idempotency (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                idempotency_key TEXT NOT NULL,
                command_name TEXT NOT NULL,
                request_hash TEXT NOT NULL,
                intent_json TEXT NOT NULL,
                primary_aggregate_key TEXT,
                lock_keys_json TEXT NOT NULL,
                status TEXT NOT NULL,
                claim_token TEXT NOT NULL,
                response_json TEXT,
                error_code TEXT,
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
        .expect("creates v10 command_idempotency table");
        create_legacy_billing_tables_for_partial_upgrade(&pool).await;

        sqlx::query("UPDATE schema_version SET version = 10")
            .execute(&pool)
            .await
            .expect("sets schema version to v10");

        run_migrations(&pool).await.expect("runs migrations");

        assert_eq!(
            command_idempotency_column_count(&pool, "error_json").await,
            1
        );

        let version: i32 = sqlx::query("SELECT version FROM schema_version LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("reads final schema version")
            .get("version");

        assert_eq!(version, 13);
    }

    #[tokio::test]
    async fn migration_v12_adds_command_ledger_metadata_on_fresh_migration() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");

        run_migrations(&pool).await.expect("runs migrations");

        assert_command_ledger_v12_shape(&pool).await;
    }

    #[tokio::test]
    async fn migration_v12_upgrades_existing_v11_command_idempotency_table() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");

        get_schema_version(&pool)
            .await
            .expect("bootstraps schema version state");

        sqlx::query(
            "CREATE TABLE command_idempotency (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                idempotency_key TEXT NOT NULL,
                command_name TEXT NOT NULL,
                request_hash TEXT NOT NULL,
                intent_json TEXT NOT NULL,
                primary_aggregate_key TEXT,
                lock_keys_json TEXT NOT NULL,
                status TEXT NOT NULL,
                claim_token TEXT NOT NULL,
                response_json TEXT,
                error_code TEXT,
                error_json TEXT,
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
        .expect("creates v11 command_idempotency table");
        create_legacy_billing_tables_for_partial_upgrade(&pool).await;

        sqlx::query("UPDATE schema_version SET version = 11")
            .execute(&pool)
            .await
            .expect("sets schema version to v11");

        run_migrations(&pool).await.expect("runs migrations");

        assert_command_ledger_v12_shape(&pool).await;

        let version: i32 = sqlx::query("SELECT version FROM schema_version LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("reads final schema version")
            .get("version");

        assert_eq!(version, 13);
    }

    #[tokio::test]
    async fn migration_v13_adds_origin_idempotency_columns_and_indexes() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");

        run_migrations(&pool).await.expect("runs migrations");

        assert_eq!(
            table_column_count(&pool, "transactions", "origin_idempotency_key").await,
            1
        );
        assert_eq!(
            table_column_count(&pool, "transactions", "origin_transaction_ordinal").await,
            1
        );
        assert_eq!(
            table_column_count(&pool, "folio_lines", "origin_idempotency_key").await,
            1
        );
        assert_eq!(
            table_column_count(&pool, "folio_lines", "origin_line_ordinal").await,
            1
        );
        assert_eq!(
            sqlite_index_count(&pool, "transactions_origin_idem_uq").await,
            1
        );
        assert_eq!(
            sqlite_index_count(&pool, "folio_lines_origin_idem_uq").await,
            1
        );
        assert_eq!(
            sqlite_index_count(&pool, "transactions_origin_command_uq").await,
            1
        );
        assert_eq!(
            sqlite_index_count(&pool, "folio_lines_origin_command_uq").await,
            1
        );
    }

    #[tokio::test]
    async fn migration_v13_keeps_legacy_null_origin_rows_valid() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");

        run_migrations(&pool).await.expect("runs migrations");

        sqlx::query(
            "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status)
             VALUES ('R1', 'Room R1', 'standard', 1, 0, 250000, 2, 0, 'vacant')",
        )
        .execute(&pool)
        .await
        .expect("seeds room");
        sqlx::query(
            "INSERT INTO guests (id, guest_type, full_name, doc_number, created_at)
             VALUES ('G1', 'domestic', 'Legacy Guest', 'DOC1', '2026-04-27T08:00:00+07:00')",
        )
        .execute(&pool)
        .await
        .expect("seeds guest");
        sqlx::query(
            "INSERT INTO bookings (id, room_id, primary_guest_id, check_in_at, expected_checkout, nights, total_price, status, created_at)
             VALUES ('B1', 'R1', 'G1', '2026-04-27', '2026-04-28', 1, 250000, 'active', '2026-04-27T08:00:00+07:00')",
        )
        .execute(&pool)
        .await
        .expect("seeds booking");

        for id in ["T1", "T2"] {
            sqlx::query(
                "INSERT INTO transactions (id, booking_id, amount, type, note, created_at)
                 VALUES (?, 'B1', 100000, 'payment', 'legacy', '2026-04-27T08:00:00+07:00')",
            )
            .bind(id)
            .execute(&pool)
            .await
            .expect("legacy transaction with NULL origin remains valid");
        }

        for id in ["F1", "F2"] {
            sqlx::query(
                "INSERT INTO folio_lines (id, booking_id, category, description, amount, created_at)
                 VALUES (?, 'B1', 'laundry', 'legacy', 20000, '2026-04-27T08:00:00+07:00')",
            )
            .bind(id)
            .execute(&pool)
            .await
            .expect("legacy folio line with NULL origin remains valid");
        }
    }
}
