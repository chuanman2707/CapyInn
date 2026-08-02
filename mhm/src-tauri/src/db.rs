use log::warn;
use sqlx::{
    sqlite::{
        SqliteConnectOptions, SqliteConnection, SqliteJournalMode, SqlitePoolOptions,
        SqliteSynchronous,
    },
    Pool, Row, Sqlite, Transaction,
};
use std::{str::FromStr, time::Duration};

mod agent;
mod command_safety;
mod core_extensions;
pub mod declaration;
pub mod local_day;
mod migrations;
mod money;
mod outbox;
pub(crate) mod row;

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
    ensure_setting_default(&pool, "ceo_cloud_data_opt_in", "false").await?;

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

async fn restore_foreign_keys_after_v14_migration(
    connection: &mut SqliteConnection,
    migration_result: Result<(), sqlx::Error>,
) -> Result<(), sqlx::Error> {
    let restore_result = sqlx::query("PRAGMA foreign_keys=ON")
        .execute(&mut *connection)
        .await;

    match (migration_result, restore_result) {
        (Ok(()), Ok(_)) => Ok(()),
        (Err(error), Ok(_)) => Err(error),
        (Ok(()), Err(error)) => Err(error),
        (Err(migration_error), Err(restore_error)) => Err(sqlx::Error::Protocol(format!(
            "v14 migration failed ({migration_error}) and restoring foreign_keys failed ({restore_error})"
        ))),
    }
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

/// Schema version a database reaches after `run_migrations` finishes on this
/// build. Bump this together with every new migration block below; the
/// `migrations_run_to_latest_schema_version` test fails otherwise.
///
/// 23 and 24 are deliberately skipped, not free: 23 belongs to
/// `design/kbtt-ux-simplify` (`migrate_v23_stay_snapshot`) and has already
/// shipped to the hotel's machine, 24 belongs to
/// `feat/room-drawer-stay-edits` (`migrate_v24_booking_rate_override`, see its
/// commit 4c5c1c3, which renumbered away from this same collision). Reusing a
/// number another branch already shipped is silent and fatal: the gate
/// `current < N` is false on a database already at N, so the migration never
/// runs and every query touching the new column dies at runtime. Before
/// claiming the next number, survey every ref — `git for-each-ref` +
/// `git show <ref>:mhm/src-tauri/src/db.rs | grep LATEST_SCHEMA_VERSION` — and
/// read the live database, not the brief. Gaps are harmless; collisions are not.
///
/// **The same trap bites backwards, and that is the easier half to miss.** A
/// branch written earlier and merged later is just as broken: once this build
/// ships and the hotel's database reads 25, `feat/room-drawer-stay-edits` —
/// still sitting at 24 — has a gate `current < 24` that can never fire again,
/// so its `bookings.rate_overridden_at` would never be created. Picking a
/// number higher than every *branch* is not enough; it must also be higher than
/// whatever the live database has already reached by the time you merge. So the
/// survey is not a one-off at the start of the work: re-run it immediately
/// before merging, and if the shipped version has moved past yours, renumber
/// above it. Nothing warns you — the app simply starts up and fails on the
/// first query that touches the missing column.
pub(crate) const LATEST_SCHEMA_VERSION: i32 = 25;

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
        migrations::migrate_v1_base_schema(pool).await?;
    }

    // ── V2: Phase 1 — Foundation + RBAC ──
    if current < 2 {
        migrations::migrate_v2_foundation_rbac(pool).await?;
    }

    // ── V3: Phase 2 — Pricing Engine ──
    if current < 3 {
        migrations::migrate_v3_pricing_engine(pool).await?;
    }

    // ── V4: Phase 3+4 — Folio/Billing + Night Audit ──
    if current < 4 {
        migrations::migrate_v4_folio_billing_night_audit(pool).await?;
    }

    // ── V5: Dynamic Room Config — room_types table + per-person pricing ──
    if current < 5 {
        migrations::migrate_v5_dynamic_room_config(pool).await?;
    }

    // ── V6: Reservation Calendar Block System ──
    if current < 6 {
        migrations::migrate_v6_reservation_calendar(pool).await?;
    }

    // -- V7: MCP Gateway - API Key Storage --
    if current < 7 {
        core_extensions::migrate_v7_gateway_api_keys(pool).await?;
    }

    // -- V8: Invoice PDF System --
    if current < 8 {
        core_extensions::migrate_v8_invoice_pdf_system(pool).await?;
    }

    // -- V9: Group Booking System --
    if current < 9 {
        core_extensions::migrate_v9_group_booking_system(pool).await?;
    }

    // ── V10: Command Idempotency ──
    if current < 10 {
        command_safety::migrate_v10_command_idempotency(pool).await?;
    }

    // ── V11: Command terminal error replay payload ──
    if current < 11 {
        command_safety::migrate_v11_command_terminal_error_replay(pool).await?;
    }

    // ── V12: Operator-ready command ledger metadata ──
    if current < 12 {
        command_safety::migrate_v12_command_ledger_metadata(pool).await?;
    }

    // ── V13: Origin idempotency on ledger and folio rows ──
    if current < 13 {
        command_safety::migrate_v13_origin_idempotency(pool).await?;
    }

    // ── V14: Integer VND money foundation ──
    if current < 14 {
        money::migrate_v14_integer_vnd_money(pool).await?;
    }

    // ── V15: Command recovery queue and audit actions ──
    if current < 15 {
        command_safety::migrate_v15_command_recovery(pool).await?;
    }

    // ── V16: Durable outbox events ──
    if current < 16 {
        outbox::migrate_v16_durable_outbox_events(pool).await?;
    }

    // -- V17: Outbox per-aggregate open-row FIFO support --
    if current < 17 {
        outbox::migrate_v17_outbox_fifo_support(pool).await?;
    }

    // -- V18: Agent safety session, audit, and memory schema --
    if current < 18 {
        agent::migrate_v18_agent_safety_tables(pool).await?;
    }

    // -- V19: CEO hourly digest run state --
    if current < 19 {
        agent::migrate_v19_agent_digest_runs(pool).await?;
    }

    // -- V20: Khai báo tạm trú — bốn bảng mới, thuần CREATE TABLE --
    if current < 20 {
        declaration::migrate_v20_declaration_tables(pool).await?;
    }

    // -- V21: khai báo được phép chưa gắn phòng --
    if current < 21 {
        declaration::migrate_v21_optional_stay(pool).await?;
    }

    // -- V22: số khách trên booking, để tính phụ thu thêm người --
    if current < 22 {
        core_extensions::migrate_v22_booking_guest_count(pool).await?;
    }

    // -- V25: lời quyết toán in được cho khách, tách khỏi ghi chú nội bộ --
    // (23 đã bị nhánh kbtt chiếm và đã chạy trên máy thật, 24 là của nhánh
    //  room-drawer — xem chú thích ở LATEST_SCHEMA_VERSION)
    if current < 25 {
        core_extensions::migrate_v25_invoice_settlement_note(pool).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        connect_configured_sqlite_pool, execute_compat_alter, get_schema_version,
        restore_foreign_keys_after_v14_migration, run_migrations,
    };
    use sqlx::{Row, SqlitePool};

    const PMS_CORE_TABLES: &[&str] = &[
        "rooms",
        "guests",
        "bookings",
        "booking_guests",
        "transactions",
        "expenses",
        "housekeeping",
        "settings",
        "users",
        "audit_logs",
        "pricing_rules",
        "special_dates",
        "folio_lines",
        "night_audit_logs",
        "room_types",
        "room_calendar",
        "invoices",
        "booking_groups",
        "group_services",
    ];

    const COMMAND_SAFETY_TABLES: &[&str] = &[
        "command_idempotency",
        "command_recovery_actions",
        "outbox_events",
    ];

    const EXPERIMENTAL_GATEWAY_TABLES: &[&str] = &["gateway_api_keys"];

    const EXPERIMENTAL_AGENT_TABLES: &[&str] = &[
        "agent_sessions",
        "agent_audit_events",
        "agent_memory_items",
        "agent_digest_runs",
    ];

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
    async fn v14_early_failure_path_restores_foreign_keys() {
        let pool = connect_configured_sqlite_pool("sqlite::memory:")
            .await
            .expect("connects configured in-memory sqlite pool");
        let mut conn = pool.acquire().await.expect("acquires connection");
        sqlx::query("PRAGMA foreign_keys=OFF")
            .execute(&mut *conn)
            .await
            .expect("disables foreign keys");

        let error = restore_foreign_keys_after_v14_migration(
            &mut conn,
            Err(sqlx::Error::Protocol("begin failed".to_string())),
        )
        .await
        .expect_err("returns original migration failure");

        assert!(error.to_string().contains("begin failed"));
        let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys;")
            .fetch_one(&mut *conn)
            .await
            .expect("reads foreign_keys pragma");
        assert_eq!(foreign_keys, 1);
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

    async fn sqlite_table_count(pool: &SqlitePool, name: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT COUNT(*)
             FROM sqlite_master
             WHERE type = 'table' AND name = ?",
        )
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("checks sqlite table")
    }

    async fn table_exists(pool: &SqlitePool, table: &str) -> bool {
        sqlite_table_count(pool, table).await == 1
    }

    async fn assert_table_group_exists(pool: &SqlitePool, group: &str, tables: &[&str]) {
        for table in tables {
            assert!(
                table_exists(pool, table).await,
                "missing {group} table {table}"
            );
        }
    }

    async fn test_pool() -> SqlitePool {
        SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite")
    }

    async fn column_exists(pool: &SqlitePool, table: &str, column: &str) -> bool {
        let sql = match table {
            "agent_sessions" => {
                "SELECT COUNT(*) FROM pragma_table_info('agent_sessions') WHERE name = ?"
            }
            "agent_audit_events" => {
                "SELECT COUNT(*) FROM pragma_table_info('agent_audit_events') WHERE name = ?"
            }
            "agent_memory_items" => {
                "SELECT COUNT(*) FROM pragma_table_info('agent_memory_items') WHERE name = ?"
            }
            "agent_digest_runs" => {
                "SELECT COUNT(*) FROM pragma_table_info('agent_digest_runs') WHERE name = ?"
            }
            _ => panic!("unsupported table {table}"),
        };

        let count: i64 = sqlx::query_scalar(sql)
            .bind(column)
            .fetch_one(pool)
            .await
            .expect("reads table info");
        count == 1
    }

    async fn assert_agent_safety_shape(pool: &SqlitePool) {
        for table in ["agent_sessions", "agent_audit_events", "agent_memory_items"] {
            assert!(table_exists(pool, table).await, "{table} table exists");
        }

        for column in [
            "id",
            "role",
            "channel",
            "channel_actor_id",
            "status",
            "uses_memory",
            "retention_policy",
            "metadata_json",
            "started_at",
            "last_seen_at",
            "ended_at",
        ] {
            assert!(
                column_exists(pool, "agent_sessions", column).await,
                "agent_sessions.{column} exists"
            );
        }

        for column in [
            "id",
            "session_id",
            "event_type",
            "actor_id",
            "role",
            "channel",
            "tool_name",
            "provider",
            "policy_outcome",
            "mutation_risk",
            "data_sensitivity",
            "summary_json",
            "created_at",
        ] {
            assert!(
                column_exists(pool, "agent_audit_events", column).await,
                "agent_audit_events.{column} exists"
            );
        }

        for column in [
            "id",
            "role",
            "scope",
            "key",
            "value_json",
            "created_at",
            "updated_at",
        ] {
            assert!(
                column_exists(pool, "agent_memory_items", column).await,
                "agent_memory_items.{column} exists"
            );
        }
    }

    async fn assert_agent_digest_runs_shape(pool: &SqlitePool) {
        assert!(
            table_exists(pool, "agent_digest_runs").await,
            "agent_digest_runs table exists"
        );

        for column in [
            "id",
            "role",
            "channel",
            "channel_actor_id",
            "delivery_chat_id",
            "due_at",
            "status",
            "attempt_count",
            "max_attempts",
            "next_retry_at",
            "claimed_at",
            "claim_token",
            "delivered_at",
            "last_error_code",
            "last_error_summary_json",
            "delivery_summary_json",
            "created_at",
            "updated_at",
        ] {
            assert!(
                column_exists(pool, "agent_digest_runs", column).await,
                "agent_digest_runs.{column} exists"
            );
        }

        for index in [
            "agent_digest_runs_status_due_idx",
            "agent_digest_runs_retry_idx",
            "agent_digest_runs_delivered_idx",
            "agent_digest_runs_actor_due_idx",
        ] {
            assert_eq!(sqlite_index_count(pool, index).await, 1, "{index} exists");
        }
    }

    async fn outbox_column_count(pool: &SqlitePool, name: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT COUNT(*)
             FROM pragma_table_info('outbox_events')
             WHERE name = ?",
        )
        .bind(name)
        .fetch_one(pool)
        .await
        .expect("checks outbox_events column")
    }

    async fn assert_outbox_shape(pool: &SqlitePool) {
        assert_eq!(sqlite_table_count(pool, "outbox_events").await, 1);

        for column in [
            "id",
            "event_type",
            "aggregate_key",
            "payload_json",
            "origin_request_id",
            "origin_idempotency_key",
            "origin_command_name",
            "origin_request_hash",
            "status",
            "worker_token",
            "attempts",
            "next_attempt_at",
            "processing_started_at",
            "processing_expires_at",
            "last_error",
            "created_at",
            "dispatched_at",
        ] {
            assert_eq!(
                outbox_column_count(pool, column).await,
                1,
                "missing outbox_events column {column}"
            );
        }

        assert_eq!(
            sqlite_index_count(pool, "outbox_events_pending_idx").await,
            1
        );
        assert_eq!(
            sqlite_index_count(pool, "outbox_events_processing_idx").await,
            1
        );
        assert_eq!(
            sqlite_index_count(pool, "outbox_events_origin_command_uq").await,
            1
        );
        assert_eq!(
            sqlite_index_count(pool, "outbox_events_aggregate_open_idx").await,
            1
        );
    }

    async fn column_type(pool: &SqlitePool, table: &str, column: &str) -> String {
        let sql = format!(
            "SELECT type FROM pragma_table_info('{}') WHERE name = ?",
            table
        );
        sqlx::query_scalar::<_, String>(&sql)
            .bind(column)
            .fetch_one(pool)
            .await
            .expect("reads column type")
            .to_uppercase()
    }

    async fn assert_money_columns_are_integer(pool: &SqlitePool) {
        for (table, columns) in [
            ("rooms", vec!["base_price", "extra_person_fee"]),
            (
                "pricing_rules",
                vec!["hourly_rate", "overnight_rate", "daily_rate"],
            ),
            (
                "bookings",
                vec!["total_price", "paid_amount", "deposit_amount"],
            ),
            ("transactions", vec!["amount"]),
            ("expenses", vec!["amount"]),
            ("folio_lines", vec!["amount"]),
            (
                "night_audit_logs",
                vec![
                    "total_revenue",
                    "room_revenue",
                    "folio_revenue",
                    "total_expenses",
                ],
            ),
            (
                "invoices",
                vec!["subtotal", "deposit_amount", "total", "balance_due"],
            ),
            ("group_services", vec!["unit_price", "total_price"]),
        ] {
            for column in columns {
                assert_eq!(
                    column_type(pool, table, column).await,
                    "INTEGER",
                    "{table}.{column}"
                );
            }
        }
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
        // Minimal shape: v22 ALTERs `bookings` directly (like v9's group columns
        // before it), so a partial-upgrade fixture starting past v9 needs the
        // table to exist even though this fixture never runs the real v1 schema.
        // No money columns here on purpose — money_migration's table/column
        // guards already skip tables that don't carry the columns it converts.
        sqlx::query("CREATE TABLE bookings (id TEXT PRIMARY KEY)")
            .execute(pool)
            .await
            .expect("creates legacy bookings table");

        // Same reason, for v23's ALTER: a real database sitting at v10 or v11
        // went through v8, so it already has `invoices`. This fixture jumps
        // straight past v8, so it has to stand the table up itself.
        sqlx::query("CREATE TABLE invoices (id TEXT PRIMARY KEY)")
            .execute(pool)
            .await
            .expect("creates legacy invoices table");

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

        assert_eq!(version, super::LATEST_SCHEMA_VERSION);
    }

    #[tokio::test]
    async fn fresh_database_migration_creates_required_table_groups() {
        let pool = test_pool().await;

        run_migrations(&pool).await.expect("runs migrations");

        let version: i32 = sqlx::query_scalar("SELECT version FROM schema_version LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("reads final schema version");

        assert_eq!(version, 25);

        assert_table_group_exists(&pool, "PMS core", PMS_CORE_TABLES).await;
        assert_table_group_exists(&pool, "command safety", COMMAND_SAFETY_TABLES).await;
        assert_table_group_exists(&pool, "experimental gateway", EXPERIMENTAL_GATEWAY_TABLES).await;
        assert_table_group_exists(&pool, "experimental agent", EXPERIMENTAL_AGENT_TABLES).await;
    }

    #[tokio::test]
    async fn migration_v14_converts_money_columns_to_integer_on_fresh_database() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects");
        run_migrations(&pool).await.expect("runs migrations");

        let version: i32 = sqlx::query("SELECT version FROM schema_version LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("reads version")
            .get("version");

        assert_eq!(version, 25);
        assert_money_columns_are_integer(&pool).await;
    }

    #[tokio::test]
    async fn migration_v14_converts_whole_vnd_legacy_real_values() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects");
        run_migrations(&pool).await.expect("runs migrations");
        sqlx::query(
            "INSERT INTO rooms (
                id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status
             ) VALUES ('R1', 'Room R1', 'standard', 1, 0, 100000.0, 2, 0.0, 'vacant')",
        )
        .execute(&pool)
        .await
        .expect("seeds whole-VND room");
        sqlx::query("UPDATE schema_version SET version = 13")
            .execute(&pool)
            .await
            .expect("rewinds schema version for test");

        run_migrations(&pool)
            .await
            .expect("whole-VND money migrates");

        let row = sqlx::query("SELECT base_price, extra_person_fee FROM rooms WHERE id = 'R1'")
            .fetch_one(&pool)
            .await
            .expect("reads migrated room");
        assert_eq!(row.get::<i64, _>("base_price"), 100000);
        assert_eq!(row.get::<i64, _>("extra_person_fee"), 0);
    }

    #[tokio::test]
    async fn migration_v14_rejects_fractional_legacy_money_and_rolls_back() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects");
        run_migrations(&pool).await.expect("runs migrations");
        sqlx::query(
            "INSERT INTO rooms (
                id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status
             ) VALUES ('R1', 'Room R1', 'standard', 1, 0, 100000, 2, 0, 'vacant')",
        )
        .execute(&pool)
        .await
        .expect("seeds room");
        sqlx::query("UPDATE schema_version SET version = 13")
            .execute(&pool)
            .await
            .expect("rewinds schema version for test");
        sqlx::query(
            "UPDATE rooms SET base_price = 100000.5 WHERE id = (SELECT id FROM rooms LIMIT 1)",
        )
        .execute(&pool)
        .await
        .expect("writes fractional legacy money");

        let error = run_migrations(&pool)
            .await
            .expect_err("fractional money blocks migration");
        assert!(error.to_string().contains("rooms.base_price"));

        let version: i32 = sqlx::query_scalar("SELECT version FROM schema_version LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("reads version");
        assert_eq!(version, 13);
    }

    #[tokio::test]
    async fn migration_v14_converts_persisted_money_json_to_integer_numbers() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects");
        run_migrations(&pool).await.expect("runs migrations");

        sqlx::query(
            "INSERT INTO rooms (
                id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status
             ) VALUES ('R1', 'Room R1', 'standard', 1, 0, 100000, 2, 0, 'vacant')",
        )
        .execute(&pool)
        .await
        .expect("seeds room");
        sqlx::query(
            "INSERT INTO guests (id, guest_type, full_name, doc_number, created_at)
             VALUES ('G1', 'domestic', 'Legacy Guest', 'DOC1', '2026-04-30T08:00:00+07:00')",
        )
        .execute(&pool)
        .await
        .expect("seeds guest");

        let pricing_snapshot = serde_json::json!({
            "base_amount": 500000.0,
            "total": 500000.0,
            "breakdown": [
                { "label": "Room", "amount": 500000.0 }
            ],
            "checkout_settlement": {
                "original_total": 500000.0,
                "settled_total": 500000.0
            }
        })
        .to_string();
        sqlx::query(
            "INSERT INTO bookings (
                id, room_id, primary_guest_id, check_in_at, expected_checkout,
                nights, total_price, status, created_at, pricing_snapshot
             ) VALUES ('B1', 'R1', 'G1', '2026-04-30', '2026-05-01', 1, 500000, 'active', '2026-04-30T08:00:00+07:00', ?)",
        )
        .bind(pricing_snapshot)
        .execute(&pool)
        .await
        .expect("seeds booking");
        sqlx::query("UPDATE schema_version SET version = 13")
            .execute(&pool)
            .await
            .expect("rewinds schema version for test");

        run_migrations(&pool).await.expect("runs v14 migration");

        let raw_snapshot: String =
            sqlx::query_scalar("SELECT pricing_snapshot FROM bookings WHERE id = 'B1'")
                .fetch_one(&pool)
                .await
                .expect("reads pricing snapshot");
        let snapshot: serde_json::Value =
            serde_json::from_str(&raw_snapshot).expect("pricing snapshot remains JSON");
        assert_eq!(snapshot["base_amount"], serde_json::json!(500000));
        assert_eq!(snapshot["total"], serde_json::json!(500000));
        assert_eq!(
            snapshot["breakdown"][0]["amount"],
            serde_json::json!(500000)
        );
        assert_eq!(
            snapshot["checkout_settlement"]["settled_total"],
            serde_json::json!(500000)
        );
    }

    #[tokio::test]
    async fn migration_v14_preserves_command_legacy_hash_and_converts_replay_json() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects");
        run_migrations(&pool).await.expect("runs migrations");

        sqlx::query(
            "INSERT INTO command_idempotency (
                idempotency_key, command_name, request_hash, intent_json,
                lock_keys_json, status, claim_token, response_json, retryable,
                created_at, updated_at
             ) VALUES (
                'idem-1', 'test.command', 'legacy-hash', ?,
                '[]', 'completed', 'claim-1', ?, 0,
                '2026-04-30T08:00:00+07:00', '2026-04-30T08:00:00+07:00'
             )",
        )
        .bind(serde_json::json!({ "amount": 100000.0 }).to_string())
        .bind(serde_json::json!({ "amount": 100000.0 }).to_string())
        .execute(&pool)
        .await
        .expect("seeds command row");
        sqlx::query("UPDATE schema_version SET version = 13")
            .execute(&pool)
            .await
            .expect("rewinds schema version for test");

        run_migrations(&pool).await.expect("runs v14 migration");

        let row = sqlx::query(
            "SELECT request_hash, legacy_request_hash, intent_json, response_json
             FROM command_idempotency
             WHERE idempotency_key = 'idem-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("reads migrated command row");
        assert_eq!(row.get::<String, _>("request_hash"), "legacy-hash");
        assert_eq!(
            row.get::<Option<String>, _>("legacy_request_hash"),
            Some("legacy-hash".to_string())
        );

        let intent: serde_json::Value =
            serde_json::from_str(&row.get::<String, _>("intent_json")).expect("intent JSON");
        let response: serde_json::Value = serde_json::from_str(
            &row.get::<Option<String>, _>("response_json")
                .expect("response JSON"),
        )
        .expect("response JSON");
        assert_eq!(intent["amount"], serde_json::json!(100000));
        assert_eq!(response["amount"], serde_json::json!(100000));
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

        assert_eq!(version, 25);
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

        assert_eq!(version, 25);
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
    async fn migration_v15_adds_command_recovery_schema() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");
        run_migrations(&pool).await.expect("migrations run");

        assert_eq!(
            command_idempotency_column_count(&pool, "recovery_dismissed_at").await,
            1
        );
        assert_eq!(
            command_idempotency_column_count(&pool, "recovery_dismissed_by").await,
            1
        );

        let table_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master
             WHERE type = 'table' AND name = 'command_recovery_actions'",
        )
        .fetch_one(&pool)
        .await
        .expect("reads recovery action table count");
        assert_eq!(table_count, 1);

        assert_eq!(
            sqlite_index_count(&pool, "command_recovery_actions_command_idx").await,
            1
        );
        assert_eq!(
            sqlite_index_count(&pool, "command_idempotency_recovery_queue_idx").await,
            1
        );

        let version = get_schema_version(&pool).await.expect("schema version");
        assert_eq!(version, 25);
    }

    #[tokio::test]
    async fn migration_v15_upgrades_existing_v14_database() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");
        run_migrations(&pool).await.expect("initial migrations run");
        sqlx::query("UPDATE schema_version SET version = 14")
            .execute(&pool)
            .await
            .expect("rewinds schema version");

        run_migrations(&pool).await.expect("v15 migration reruns");

        assert_eq!(
            command_idempotency_column_count(&pool, "recovery_dismissed_at").await,
            1
        );
        assert_eq!(
            command_idempotency_column_count(&pool, "recovery_dismissed_by").await,
            1
        );
        assert_eq!(
            sqlite_index_count(&pool, "command_idempotency_recovery_queue_idx").await,
            1
        );
    }

    #[tokio::test]
    async fn migration_v16_adds_outbox_events_schema() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");

        run_migrations(&pool).await.expect("runs migrations");

        assert_outbox_shape(&pool).await;
        let version = get_schema_version(&pool).await.expect("schema version");
        assert_eq!(version, 25);
    }

    #[tokio::test]
    async fn migration_v16_upgrades_existing_v15_database() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");
        run_migrations(&pool).await.expect("initial migrations run");
        sqlx::query("UPDATE schema_version SET version = 15")
            .execute(&pool)
            .await
            .expect("rewinds schema version");

        run_migrations(&pool).await.expect("v16 migration reruns");

        assert_outbox_shape(&pool).await;
        let version = get_schema_version(&pool).await.expect("schema version");
        assert_eq!(version, 25);
    }

    #[tokio::test]
    async fn migration_v17_adds_outbox_fifo_support_index() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");
        run_migrations(&pool).await.expect("initial migrations run");
        sqlx::query("UPDATE schema_version SET version = 16")
            .execute(&pool)
            .await
            .expect("rewinds schema version");
        sqlx::query("DROP INDEX outbox_events_aggregate_open_idx")
            .execute(&pool)
            .await
            .expect("removes v17 index");

        assert_eq!(
            sqlite_index_count(&pool, "outbox_events_aggregate_open_idx").await,
            0
        );

        run_migrations(&pool).await.expect("v17 migration reruns");

        assert_eq!(
            sqlite_index_count(&pool, "outbox_events_aggregate_open_idx").await,
            1
        );
        let version = get_schema_version(&pool).await.expect("schema version");
        assert_eq!(version, 25);
    }

    #[tokio::test]
    async fn migration_v18_adds_agent_safety_tables() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");

        run_migrations(&pool).await.expect("runs migrations");

        assert_agent_safety_shape(&pool).await;
        let version = get_schema_version(&pool).await.expect("schema version");
        assert_eq!(version, 25);
    }

    #[tokio::test]
    async fn v19_creates_agent_digest_runs_schema() {
        let pool = test_pool().await;
        run_migrations(&pool).await.expect("runs migrations");

        assert_agent_digest_runs_shape(&pool).await;
    }

    /// The decisive migration test, modelled on
    /// `migration_v24_alter_actually_runs_on_a_genuinely_pre_v24_database`
    /// (commit 4c5c1c3, which renumbered away from this same collision).
    ///
    /// Rewinding `schema_version` while the column is still present proves
    /// nothing: the replayed ALTER lands in `execute_compat_alter`'s
    /// duplicate-column swallow and the assertion passes on the column the
    /// FIRST run created. It stays green even when the gate never fires —
    /// exactly the production failure this branch shipped, where the hotel's
    /// database was already at 23 (claimed by another branch), `current < 23`
    /// was false, `settlement_note` was never created, and the first
    /// `generate_invoice` died on `no such column`.
    ///
    /// So: DROP the column to build a database that has genuinely never seen
    /// this migration, rewind to the true predecessor, and require the real
    /// ALTER path to put it back.
    #[tokio::test]
    async fn migration_v25_alter_actually_runs_on_a_genuinely_pre_v25_database() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");
        run_migrations(&pool).await.expect("runs migrations to latest");

        sqlx::query("ALTER TABLE invoices DROP COLUMN settlement_note")
            .execute(&pool)
            .await
            .expect("drops settlement_note to simulate a pre-v25 database");
        assert_eq!(
            invoices_settlement_note_column_count(&pool).await,
            0,
            "tiền đề của test: DB phải thật sự chưa có cột settlement_note"
        );

        // 24 is the true predecessor — the highest number claimed by any other
        // branch. A database sitting at 23 or 24 must still be upgraded.
        sqlx::query("UPDATE schema_version SET version = 24")
            .execute(&pool)
            .await
            .expect("rewinds schema version to a genuinely pre-v25 state");

        run_migrations(&pool)
            .await
            .expect("v25 migration adds the column back via a real ALTER TABLE");

        assert_eq!(
            invoices_settlement_note_column_count(&pool).await,
            1,
            "ALTER TABLE thật phải chạy và tạo lại cột settlement_note"
        );

        // Replaying must be a no-op, not a half-applied upgrade that dies on
        // the hotel's database: `execute_compat_alter` swallows the
        // duplicate-column error the second ALTER would raise.
        run_migrations(&pool)
            .await
            .expect("v25 migration is idempotent");
        assert_eq!(invoices_settlement_note_column_count(&pool).await, 1);

        let version = get_schema_version(&pool).await.expect("schema version");
        assert_eq!(version, 25);
    }

    /// A database still sitting at kbtt's 23 — the version the hotel's machine
    /// actually reports — must be carried all the way to 25, not stall because
    /// some intermediate number was skipped.
    #[tokio::test]
    async fn migration_v25_upgrades_a_database_left_at_the_shipped_v23() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");
        run_migrations(&pool).await.expect("runs migrations to latest");

        sqlx::query("ALTER TABLE invoices DROP COLUMN settlement_note")
            .execute(&pool)
            .await
            .expect("drops settlement_note");
        sqlx::query("UPDATE schema_version SET version = 23")
            .execute(&pool)
            .await
            .expect("rewinds to the version the live database reports");

        run_migrations(&pool).await.expect("v25 runs from v23");

        assert_eq!(invoices_settlement_note_column_count(&pool).await, 1);
        let version = get_schema_version(&pool).await.expect("schema version");
        assert_eq!(version, 25);
    }

    async fn invoices_settlement_note_column_count(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info('invoices') WHERE name = 'settlement_note'",
        )
        .fetch_one(pool)
        .await
        .expect("reads invoices columns")
    }

    #[tokio::test]
    async fn migration_v19_upgrades_existing_v18_database_idempotently() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");
        run_migrations(&pool).await.expect("initial migrations run");
        sqlx::query("UPDATE schema_version SET version = 18")
            .execute(&pool)
            .await
            .expect("rewinds schema version");
        sqlx::query("DROP TABLE IF EXISTS agent_digest_runs")
            .execute(&pool)
            .await
            .expect("removes v19 table");

        run_migrations(&pool).await.expect("v19 migration reruns");
        run_migrations(&pool)
            .await
            .expect("v19 migration is idempotent");

        assert_agent_digest_runs_shape(&pool).await;
        let version = get_schema_version(&pool).await.expect("schema version");
        assert_eq!(version, 25);
    }

    #[tokio::test]
    async fn migration_v18_upgrades_existing_v17_database() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");
        run_migrations(&pool).await.expect("initial migrations run");
        sqlx::query("UPDATE schema_version SET version = 17")
            .execute(&pool)
            .await
            .expect("rewinds schema version");
        for table in ["agent_audit_events", "agent_memory_items", "agent_sessions"] {
            sqlx::query(&format!("DROP TABLE IF EXISTS {table}"))
                .execute(&pool)
                .await
                .expect("removes v18 table");
        }

        run_migrations(&pool).await.expect("v18 migration reruns");

        assert_agent_safety_shape(&pool).await;
        let version = get_schema_version(&pool).await.expect("schema version");
        assert_eq!(version, 25);
    }

    #[tokio::test]
    async fn migration_v16_pending_outbox_defaults_are_insertable() {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");
        run_migrations(&pool).await.expect("runs migrations");

        sqlx::query(
            "INSERT INTO outbox_events (
                event_type, aggregate_key, payload_json,
                origin_request_id, origin_idempotency_key,
                origin_command_name, origin_request_hash,
                status, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("booking.checked_out")
        .bind("booking:B1")
        .bind(r#"{"schema_version":1}"#)
        .bind("req-1")
        .bind("idem-1")
        .bind("check_out")
        .bind("hash-1")
        .bind("pending")
        .bind("2026-05-03T09:00:00+07:00")
        .execute(&pool)
        .await
        .expect("inserts pending outbox row");

        let row = sqlx::query(
            "SELECT status, attempts, worker_token, next_attempt_at,
                    processing_started_at, processing_expires_at,
                    last_error, dispatched_at
             FROM outbox_events WHERE origin_command_name = 'check_out'",
        )
        .fetch_one(&pool)
        .await
        .expect("reads outbox row");

        assert_eq!(row.get::<String, _>("status"), "pending");
        assert_eq!(row.get::<i64, _>("attempts"), 0);
        assert_eq!(row.get::<Option<String>, _>("worker_token"), None);
        assert_eq!(row.get::<Option<String>, _>("next_attempt_at"), None);
        assert_eq!(row.get::<Option<String>, _>("processing_started_at"), None);
        assert_eq!(row.get::<Option<String>, _>("processing_expires_at"), None);
        assert_eq!(row.get::<Option<String>, _>("last_error"), None);
        assert_eq!(row.get::<Option<String>, _>("dispatched_at"), None);
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
