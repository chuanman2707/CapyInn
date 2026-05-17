use sqlx::{Pool, Sqlite};

use super::set_schema_version;

pub(super) async fn migrate_v18_agent_safety_tables(
    pool: &Pool<Sqlite>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS agent_sessions (
                id TEXT PRIMARY KEY,
                role TEXT NOT NULL,
                channel TEXT NOT NULL,
                channel_actor_id TEXT,
                status TEXT NOT NULL,
                uses_memory INTEGER NOT NULL DEFAULT 0,
                retention_policy TEXT NOT NULL,
                metadata_json TEXT NOT NULL DEFAULT '{}',
                started_at TEXT NOT NULL,
                last_seen_at TEXT,
                ended_at TEXT
            )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS agent_sessions_actor_idx
             ON agent_sessions(role, channel, channel_actor_id, last_seen_at)",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS agent_sessions_status_idx
             ON agent_sessions(status, started_at)",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS agent_audit_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT REFERENCES agent_sessions(id),
                event_type TEXT NOT NULL,
                actor_id TEXT,
                role TEXT,
                channel TEXT,
                tool_name TEXT,
                provider TEXT,
                policy_outcome TEXT NOT NULL,
                mutation_risk TEXT,
                data_sensitivity TEXT,
                summary_json TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL
            )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS agent_audit_events_type_idx
             ON agent_audit_events(event_type, created_at)",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS agent_audit_events_session_idx
             ON agent_audit_events(session_id, created_at)",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS agent_audit_events_role_channel_idx
             ON agent_audit_events(role, channel, created_at)",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS agent_memory_items (
                id TEXT PRIMARY KEY,
                role TEXT NOT NULL,
                scope TEXT NOT NULL,
                key TEXT NOT NULL,
                value_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                UNIQUE(role, scope, key)
            )",
    )
    .execute(&mut *tx)
    .await?;

    set_schema_version(&mut tx, 18).await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn migrate_v19_agent_digest_runs(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS agent_digest_runs (
                id TEXT PRIMARY KEY,
                role TEXT NOT NULL,
                channel TEXT NOT NULL,
                channel_actor_id TEXT,
                delivery_chat_id TEXT,
                due_at TEXT NOT NULL,
                status TEXT NOT NULL,
                attempt_count INTEGER NOT NULL DEFAULT 0,
                max_attempts INTEGER NOT NULL,
                next_retry_at TEXT,
                claimed_at TEXT,
                claim_token TEXT,
                delivered_at TEXT,
                last_error_code TEXT,
                last_error_summary_json TEXT NOT NULL DEFAULT '{}',
                delivery_summary_json TEXT NOT NULL DEFAULT '{}',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS agent_digest_runs_status_due_idx
             ON agent_digest_runs(status, due_at)",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS agent_digest_runs_retry_idx
             ON agent_digest_runs(status, next_retry_at)
             WHERE status = 'retry_waiting'",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS agent_digest_runs_delivered_idx
             ON agent_digest_runs(delivered_at)",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS agent_digest_runs_actor_due_idx
             ON agent_digest_runs(channel_actor_id, delivery_chat_id, due_at)",
    )
    .execute(&mut *tx)
    .await?;

    set_schema_version(&mut tx, 19).await?;
    tx.commit().await?;
    Ok(())
}
