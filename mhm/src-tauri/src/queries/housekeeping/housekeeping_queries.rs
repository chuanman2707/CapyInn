//! Housekeeping reads.

use sqlx::{Pool, Row, Sqlite};

use crate::models::HousekeepingTask;

/// Everything still needing attention. `clean` tasks are done and are excluded
/// rather than filtered client-side.
pub async fn load_open_tasks(pool: &Pool<Sqlite>) -> Result<Vec<HousekeepingTask>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, room_id, status, note, triggered_at, cleaned_at, created_at
         FROM housekeeping
         WHERE status != 'clean'
         ORDER BY triggered_at ASC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| HousekeepingTask {
            id: row.get("id"),
            room_id: row.get("room_id"),
            status: row.get("status"),
            note: row.get("note"),
            triggered_at: row.get("triggered_at"),
            cleaned_at: row.get("cleaned_at"),
            created_at: row.get("created_at"),
        })
        .collect())
}

/// Errors when the task does not exist — the caller has a task id it believes in,
/// so a missing row is a system-level surprise rather than a user outcome.
pub async fn load_task_room_id(pool: &Pool<Sqlite>, task_id: &str) -> Result<String, sqlx::Error> {
    sqlx::query_scalar("SELECT room_id FROM housekeeping WHERE id = ?")
        .bind(task_id)
        .fetch_one(pool)
        .await
}
