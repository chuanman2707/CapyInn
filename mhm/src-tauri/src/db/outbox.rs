use sqlx::{Pool, Sqlite};

use super::set_schema_version;

pub(super) async fn migrate_v16_durable_outbox_events(
    pool: &Pool<Sqlite>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS outbox_events (
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
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS outbox_events_pending_idx
             ON outbox_events(next_attempt_at, aggregate_key, id)
             WHERE status = 'pending'",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS outbox_events_processing_idx
             ON outbox_events(processing_expires_at)
             WHERE status = 'processing'",
    )
    .execute(&mut *tx)
    .await?;

    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS outbox_events_origin_command_uq
             ON outbox_events(origin_command_name, origin_idempotency_key)",
    )
    .execute(&mut *tx)
    .await?;

    set_schema_version(&mut tx, 16).await?;
    tx.commit().await?;
    Ok(())
}

pub(super) async fn migrate_v17_outbox_fifo_support(
    pool: &Pool<Sqlite>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS outbox_events_aggregate_open_idx
             ON outbox_events(aggregate_key, id)
             WHERE status IN ('pending', 'processing')",
    )
    .execute(&mut *tx)
    .await?;

    set_schema_version(&mut tx, 17).await?;
    tx.commit().await?;
    Ok(())
}
