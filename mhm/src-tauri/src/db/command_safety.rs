use sqlx::{Pool, Sqlite};

use super::{execute_compat_alter, set_schema_version};

pub(super) async fn migrate_v10_command_idempotency(
    pool: &Pool<Sqlite>,
) -> Result<(), sqlx::Error> {
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
    Ok(())
}

pub(super) async fn migrate_v11_command_terminal_error_replay(
    pool: &Pool<Sqlite>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    execute_compat_alter(
        &mut tx,
        "ALTER TABLE command_idempotency ADD COLUMN error_json TEXT",
    )
    .await?;

    set_schema_version(&mut tx, 11).await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn migrate_v12_command_ledger_metadata(
    pool: &Pool<Sqlite>,
) -> Result<(), sqlx::Error> {
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
    Ok(())
}

pub(super) async fn migrate_v13_origin_idempotency(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
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
    Ok(())
}

pub(super) async fn migrate_v15_command_recovery(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    for alter in [
        "ALTER TABLE command_idempotency ADD COLUMN recovery_dismissed_at TEXT",
        "ALTER TABLE command_idempotency ADD COLUMN recovery_dismissed_by TEXT",
    ] {
        execute_compat_alter(&mut tx, alter).await?;
    }

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS command_recovery_actions (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                command_idempotency_id INTEGER NOT NULL,
                action TEXT NOT NULL,
                operator_id TEXT,
                operator_role TEXT,
                reason TEXT,
                confirmed INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                FOREIGN KEY(command_idempotency_id) REFERENCES command_idempotency(id)
            )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS command_recovery_actions_command_idx
             ON command_recovery_actions(command_idempotency_id, created_at)",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS command_idempotency_recovery_queue_idx
             ON command_idempotency(status, lease_expires_at, updated_at)
             WHERE status IN ('in_progress', 'failed_retryable')
               AND recovery_dismissed_at IS NULL",
    )
    .execute(&mut *tx)
    .await?;

    set_schema_version(&mut tx, 15).await?;
    tx.commit().await?;
    Ok(())
}
