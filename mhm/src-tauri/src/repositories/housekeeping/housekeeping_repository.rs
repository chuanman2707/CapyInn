//! Housekeeping writes.
//!
//! The two `_tx` functions exist because finishing a clean has to flip the task
//! and the room together or not at all; they take the caller's transaction and
//! return `rows_affected` so the service can enforce the state transition
//! instead of trusting it.

use sqlx::{Pool, Sqlite, Transaction};

/// Any move that is *not* completing a clean. Clears `cleaned_at`, because a task
/// going back to needing attention is no longer a finished clean.
pub async fn update_task_status(
    pool: &Pool<Sqlite>,
    task_id: &str,
    status: &str,
    note: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE housekeeping SET status = ?, note = COALESCE(?, note), cleaned_at = NULL WHERE id = ?",
    )
    .bind(status)
    .bind(note)
    .bind(task_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Guarded by `status = 'cleaning'`, so a task that already finished or was never
/// started reports zero rows rather than being force-completed.
pub async fn mark_task_clean_tx(
    tx: &mut Transaction<'_, Sqlite>,
    task_id: &str,
    note: Option<&str>,
    cleaned_at: &str,
) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE housekeeping
         SET status = 'clean', note = COALESCE(?, note), cleaned_at = ?
         WHERE id = ? AND status = 'cleaning'",
    )
    .bind(note)
    .bind(cleaned_at)
    .bind(task_id)
    .execute(&mut **tx)
    .await?;

    Ok(result.rows_affected())
}

/// Guarded by `status = 'cleaning'`, which is what stops a re-occupied room from
/// being marked vacant by a late housekeeping update.
pub async fn mark_room_vacant_tx(
    tx: &mut Transaction<'_, Sqlite>,
    room_id: &str,
) -> Result<u64, sqlx::Error> {
    let result =
        sqlx::query("UPDATE rooms SET status = 'vacant' WHERE id = ? AND status = 'cleaning'")
            .bind(room_id)
            .execute(&mut **tx)
            .await?;

    Ok(result.rows_affected())
}
