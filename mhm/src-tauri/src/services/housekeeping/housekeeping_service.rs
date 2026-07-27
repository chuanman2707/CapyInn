//! Housekeeping status rules.
//!
//! Completing a clean is the only interesting transition: it flips the task to
//! `clean` and the room to `vacant`, in one transaction, under the room's
//! aggregate lock. Both updates are guarded on `status = 'cleaning'`, so a room
//! that has been re-occupied in the meantime is never marked vacant by a late
//! housekeeping update — the guard that
//! `housekeeping_clean_does_not_mark_occupied_room_vacant` exercises.
//!
//! This moved out of `commands::rooms`, where it sat inline with its SQL. The
//! `log_system_error` step names are unchanged.

use serde_json::json;
use sqlx::{Pool, Sqlite};

use crate::app_error::{codes, log_system_error, CommandError, CommandResult};
use crate::models::HousekeepingTask;
use crate::queries::housekeeping::housekeeping_queries as housekeeping_reads;
use crate::repositories::housekeeping::housekeeping_repository as housekeeping_writes;

pub async fn list_open_tasks(pool: &Pool<Sqlite>) -> Result<Vec<HousekeepingTask>, String> {
    housekeeping_reads::load_open_tasks(pool)
        .await
        .map_err(|error| error.to_string())
}

pub async fn update_status(
    pool: &Pool<Sqlite>,
    task_id: &str,
    new_status: &str,
    note: Option<&str>,
) -> CommandResult<()> {
    if new_status == "clean" {
        return complete_clean_to_vacant(pool, task_id, note).await;
    }

    housekeeping_writes::update_task_status(pool, task_id, new_status, note)
        .await
        .map_err(|error| {
            log_system_error(
                "update_housekeeping",
                error.to_string(),
                json!({ "task_id": task_id, "step": "update_status" }),
            )
        })
}

pub(crate) async fn complete_clean_to_vacant(
    pool: &Pool<Sqlite>,
    task_id: &str,
    note: Option<&str>,
) -> CommandResult<()> {
    let room_id = housekeeping_reads::load_task_room_id(pool, task_id)
        .await
        .map_err(|error| {
            log_system_error(
                "update_housekeeping",
                error.to_string(),
                json!({
                    "task_id": task_id,
                    "step": "lookup_room",
                }),
            )
        })?;

    let _lock_guard = crate::aggregate_locks::global_manager()
        .acquire([crate::aggregate_locks::room_key(&room_id)?])
        .await?;

    let mut tx = pool.begin().await.map_err(|error| {
        log_system_error(
            "update_housekeeping",
            error.to_string(),
            json!({
                "task_id": task_id,
                "room_id": room_id,
                "step": "begin",
            }),
        )
    })?;

    let cleaned_at = chrono::Local::now().to_rfc3339();
    let housekeeping_rows =
        housekeeping_writes::mark_task_clean_tx(&mut tx, task_id, note, &cleaned_at)
            .await
            .map_err(|error| {
                log_system_error(
                    "update_housekeeping",
                    error.to_string(),
                    json!({
                        "task_id": task_id,
                        "room_id": room_id,
                        "step": "update_housekeeping",
                    }),
                )
            })?;

    if housekeeping_rows != 1 {
        let _ = tx.rollback().await;
        return Err(CommandError::user(
            codes::CONFLICT_INVALID_STATE_TRANSITION,
            "Housekeeping task is no longer cleaning",
        ));
    }

    let room_rows = housekeeping_writes::mark_room_vacant_tx(&mut tx, &room_id)
        .await
        .map_err(|error| {
            log_system_error(
                "update_housekeeping",
                error.to_string(),
                json!({
                    "task_id": task_id,
                    "room_id": room_id,
                    "step": "update_room",
                }),
            )
        })?;

    if room_rows != 1 {
        let _ = tx.rollback().await;
        return Err(CommandError::user(
            codes::CONFLICT_INVALID_STATE_TRANSITION,
            "Room is no longer waiting for cleaning completion",
        ));
    }

    tx.commit().await.map_err(|error| {
        log_system_error(
            "update_housekeeping",
            error.to_string(),
            json!({
                "task_id": task_id,
                "room_id": room_id,
                "step": "commit",
            }),
        )
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{complete_clean_to_vacant, update_status};
    use crate::app_error::codes;
    use sqlx::{sqlite::SqlitePoolOptions, Row};

    async fn migrated_pool() -> sqlx::Pool<sqlx::Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect in-memory sqlite");
        crate::db::run_migrations(&pool)
            .await
            .expect("run migrations");
        pool
    }

    async fn seed_room(pool: &sqlx::Pool<sqlx::Sqlite>, room_id: &str, status: &str) {
        sqlx::query(
            "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(room_id)
        .bind("Housekeeping Guard")
        .bind("standard")
        .bind(1)
        .bind(0)
        .bind(100000)
        .bind(2)
        .bind(0)
        .bind(status)
        .execute(pool)
        .await
        .expect("insert room");
    }

    async fn seed_task(
        pool: &sqlx::Pool<sqlx::Sqlite>,
        task_id: &str,
        room_id: &str,
        status: &str,
    ) {
        sqlx::query(
            "INSERT INTO housekeeping (id, room_id, status, note, triggered_at, cleaned_at, created_at)
             VALUES (?, ?, ?, ?, datetime('now'), NULL, datetime('now'))",
        )
        .bind(task_id)
        .bind(room_id)
        .bind(status)
        .bind("started")
        .execute(pool)
        .await
        .expect("insert housekeeping task");
    }

    async fn status_of(pool: &sqlx::Pool<sqlx::Sqlite>, table: &str, id: &str) -> String {
        sqlx::query(&format!("SELECT status FROM {table} WHERE id = ?"))
            .bind(id)
            .fetch_one(pool)
            .await
            .expect("status row")
            .get::<String, _>("status")
    }

    #[tokio::test]
    async fn housekeeping_clean_does_not_mark_occupied_room_vacant() {
        let pool = migrated_pool().await;
        seed_room(&pool, "R-HK", "occupied").await;
        seed_task(&pool, "HK1", "R-HK", "cleaning").await;

        let error = complete_clean_to_vacant(&pool, "HK1", None)
            .await
            .expect_err("occupied room should reject clean-to-vacant");

        assert_eq!(error.code, codes::CONFLICT_INVALID_STATE_TRANSITION);
        assert_eq!(status_of(&pool, "rooms", "R-HK").await, "occupied");
        assert_eq!(status_of(&pool, "housekeeping", "HK1").await, "cleaning");
    }

    #[tokio::test]
    async fn housekeeping_clean_frees_a_room_that_is_still_cleaning() {
        let pool = migrated_pool().await;
        seed_room(&pool, "R-HK2", "cleaning").await;
        seed_task(&pool, "HK2", "R-HK2", "cleaning").await;

        update_status(&pool, "HK2", "clean", Some("done"))
            .await
            .expect("clean-to-vacant should succeed");

        assert_eq!(status_of(&pool, "rooms", "R-HK2").await, "vacant");
        assert_eq!(status_of(&pool, "housekeeping", "HK2").await, "clean");

        let cleaned_at: Option<String> =
            sqlx::query_scalar("SELECT cleaned_at FROM housekeeping WHERE id = ?")
                .bind("HK2")
                .fetch_one(&pool)
                .await
                .expect("cleaned_at");
        assert!(cleaned_at.is_some());
    }

    #[tokio::test]
    async fn housekeeping_clean_rejects_a_task_that_was_never_started() {
        let pool = migrated_pool().await;
        seed_room(&pool, "R-HK3", "cleaning").await;
        seed_task(&pool, "HK3", "R-HK3", "needs_cleaning").await;

        let error = update_status(&pool, "HK3", "clean", None)
            .await
            .expect_err("a task that is not cleaning cannot be completed");

        assert_eq!(error.code, codes::CONFLICT_INVALID_STATE_TRANSITION);
        assert_eq!(status_of(&pool, "rooms", "R-HK3").await, "cleaning");
    }

    #[tokio::test]
    async fn a_non_clean_transition_clears_the_cleaned_timestamp() {
        let pool = migrated_pool().await;
        seed_room(&pool, "R-HK4", "cleaning").await;
        seed_task(&pool, "HK4", "R-HK4", "cleaning").await;
        sqlx::query("UPDATE housekeeping SET cleaned_at = ? WHERE id = ?")
            .bind("2026-04-22T00:00:00+07:00")
            .bind("HK4")
            .execute(&pool)
            .await
            .expect("preset cleaned_at");

        update_status(&pool, "HK4", "needs_cleaning", None)
            .await
            .expect("move back to needs_cleaning");

        let cleaned_at: Option<String> =
            sqlx::query_scalar("SELECT cleaned_at FROM housekeeping WHERE id = ?")
                .bind("HK4")
                .fetch_one(&pool)
                .await
                .expect("cleaned_at");
        assert_eq!(cleaned_at, None);
        assert_eq!(
            status_of(&pool, "housekeeping", "HK4").await,
            "needs_cleaning"
        );
    }

    #[tokio::test]
    async fn a_non_clean_transition_keeps_the_existing_note_when_none_is_given() {
        let pool = migrated_pool().await;
        seed_room(&pool, "R-HK5", "cleaning").await;
        seed_task(&pool, "HK5", "R-HK5", "cleaning").await;

        update_status(&pool, "HK5", "needs_cleaning", None)
            .await
            .expect("move back to needs_cleaning");

        let note: Option<String> = sqlx::query_scalar("SELECT note FROM housekeeping WHERE id = ?")
            .bind("HK5")
            .fetch_one(&pool)
            .await
            .expect("note");
        assert_eq!(note.as_deref(), Some("started"));
    }
}
