use crate::{
    app_error::{codes, CommandError, CommandResult},
    command_idempotency::system_error,
};
use chrono::Utc;
use serde_json::Value;
use sqlx::{Pool, Row, Sqlite};

pub const DIGEST_STATUS_PENDING: &str = "pending";
pub const DIGEST_STATUS_IN_PROGRESS: &str = "in_progress";
pub const DIGEST_STATUS_RETRY_WAITING: &str = "retry_waiting";
pub const DIGEST_STATUS_DELIVERED: &str = "delivered";
pub const DIGEST_STATUS_FAILED: &str = "failed";

pub const CEO_DIGEST_MAX_ATTEMPTS: i64 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDigestRun {
    pub id: String,
    pub channel_actor_id: Option<String>,
    pub delivery_chat_id: Option<i64>,
    pub due_at: String,
    pub max_attempts: i64,
}

#[derive(Debug, Clone)]
pub struct ClaimedDigestRun {
    pub id: String,
    pub channel_actor_id: Option<String>,
    pub delivery_chat_id: Option<i64>,
    pub due_at: String,
    pub attempt_count: i64,
    pub max_attempts: i64,
    pub claim_token: String,
}

fn stable_json(value: &Value) -> CommandResult<String> {
    serde_json::to_string(value).map_err(system_error)
}

pub async fn create_digest_run_if_absent(
    pool: &Pool<Sqlite>,
    input: NewDigestRun,
) -> CommandResult<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT OR IGNORE INTO agent_digest_runs (
            id, role, channel, channel_actor_id, delivery_chat_id, due_at, status,
            attempt_count, max_attempts, last_error_summary_json, delivery_summary_json,
            created_at, updated_at
         ) VALUES (?, 'ceo_secretary', 'telegram', ?, ?, ?, 'pending', 0, ?, '{}', '{}', ?, ?)",
    )
    .bind(&input.id)
    .bind(&input.channel_actor_id)
    .bind(input.delivery_chat_id.map(|value| value.to_string()))
    .bind(&input.due_at)
    .bind(input.max_attempts)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(system_error)?;
    Ok(())
}

pub async fn claim_due_digest_run(
    pool: &Pool<Sqlite>,
    now: &str,
    claim_token: &str,
) -> CommandResult<Option<ClaimedDigestRun>> {
    let mut tx = pool.begin().await.map_err(system_error)?;
    let row = sqlx::query(
        "SELECT id, channel_actor_id, delivery_chat_id, due_at, attempt_count, max_attempts
         FROM agent_digest_runs
         WHERE (
             status = 'pending'
             OR (status = 'retry_waiting' AND next_retry_at IS NOT NULL AND next_retry_at <= ?)
         )
           AND due_at <= ?
         ORDER BY due_at ASC
         LIMIT 1",
    )
    .bind(now)
    .bind(now)
    .fetch_optional(&mut *tx)
    .await
    .map_err(system_error)?;

    let Some(row) = row else {
        tx.commit().await.map_err(system_error)?;
        return Ok(None);
    };

    let id: String = row.get("id");
    let result = sqlx::query(
        "UPDATE agent_digest_runs
         SET status = 'in_progress',
             attempt_count = attempt_count + 1,
             claimed_at = ?,
             claim_token = ?,
             next_retry_at = NULL,
             updated_at = ?
         WHERE id = ?
           AND (
             status = 'pending'
             OR (status = 'retry_waiting' AND next_retry_at IS NOT NULL AND next_retry_at <= ?)
           )",
    )
    .bind(now)
    .bind(claim_token)
    .bind(now)
    .bind(&id)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(system_error)?;

    if result.rows_affected() == 0 {
        tx.commit().await.map_err(system_error)?;
        return Ok(None);
    }

    let claimed = ClaimedDigestRun {
        id,
        channel_actor_id: row.get("channel_actor_id"),
        delivery_chat_id: row
            .get::<Option<String>, _>("delivery_chat_id")
            .and_then(|value| value.parse::<i64>().ok()),
        due_at: row.get("due_at"),
        attempt_count: row.get::<i64, _>("attempt_count") + 1,
        max_attempts: row.get("max_attempts"),
        claim_token: claim_token.to_string(),
    };
    tx.commit().await.map_err(system_error)?;
    Ok(Some(claimed))
}

pub async fn mark_digest_delivered(
    pool: &Pool<Sqlite>,
    id: &str,
    claim_token: &str,
    summary: Value,
) -> CommandResult<()> {
    let now = Utc::now().to_rfc3339();
    let result = sqlx::query(
        "UPDATE agent_digest_runs
         SET status = 'delivered',
             delivered_at = ?,
             delivery_summary_json = ?,
             last_error_code = NULL,
             last_error_summary_json = '{}',
             updated_at = ?
         WHERE id = ? AND status = 'in_progress' AND claim_token = ?",
    )
    .bind(&now)
    .bind(stable_json(&summary)?)
    .bind(&now)
    .bind(id)
    .bind(claim_token)
    .execute(pool)
    .await
    .map_err(system_error)?;
    if result.rows_affected() == 0 {
        return Err(CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            "Digest claim is no longer current.",
        ));
    }
    Ok(())
}

pub async fn mark_digest_retry_or_failed(
    pool: &Pool<Sqlite>,
    id: &str,
    claim_token: &str,
    error_code: &str,
    error_summary: Value,
    next_retry_at: Option<String>,
) -> CommandResult<()> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query("SELECT attempt_count, max_attempts FROM agent_digest_runs WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(system_error)?;
    let attempt_count: i64 = row.get("attempt_count");
    let max_attempts: i64 = row.get("max_attempts");
    let status = if attempt_count >= max_attempts {
        DIGEST_STATUS_FAILED
    } else {
        DIGEST_STATUS_RETRY_WAITING
    };
    let retry_at = if status == DIGEST_STATUS_FAILED {
        None
    } else {
        next_retry_at
    };

    let result = sqlx::query(
        "UPDATE agent_digest_runs
         SET status = ?,
             next_retry_at = ?,
             last_error_code = ?,
             last_error_summary_json = ?,
             updated_at = ?
         WHERE id = ? AND status = 'in_progress' AND claim_token = ?",
    )
    .bind(status)
    .bind(retry_at)
    .bind(error_code)
    .bind(stable_json(&error_summary)?)
    .bind(&now)
    .bind(id)
    .bind(claim_token)
    .execute(pool)
    .await
    .map_err(system_error)?;
    if result.rows_affected() == 0 {
        return Err(CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            "Digest claim is no longer current.",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{sqlite::SqlitePoolOptions, Row};

    async fn test_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect sqlite");
        crate::db::run_migrations(&pool).await.expect("migrations");
        pool
    }

    fn run(id: &str, due_at: &str) -> NewDigestRun {
        NewDigestRun {
            id: id.to_string(),
            channel_actor_id: Some("123".to_string()),
            delivery_chat_id: Some(55),
            due_at: due_at.to_string(),
            max_attempts: CEO_DIGEST_MAX_ATTEMPTS,
        }
    }

    #[tokio::test]
    async fn create_digest_run_is_idempotent_for_same_id() {
        let pool = test_pool().await;
        create_digest_run_if_absent(&pool, run("digest-1", "2026-05-07T01:00:00Z"))
            .await
            .expect("first insert");
        create_digest_run_if_absent(&pool, run("digest-1", "2026-05-07T01:00:00Z"))
            .await
            .expect("second insert ignored");

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_digest_runs")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn claim_due_digest_run_allows_only_one_claim() {
        let pool = test_pool().await;
        create_digest_run_if_absent(&pool, run("digest-1", "2026-05-07T01:00:00Z"))
            .await
            .expect("insert");

        let first = claim_due_digest_run(&pool, "2026-05-07T01:00:01Z", "claim-a")
            .await
            .expect("claim")
            .expect("claimed");
        assert_eq!(first.id, "digest-1");
        assert_eq!(first.attempt_count, 1);

        let second = claim_due_digest_run(&pool, "2026-05-07T01:00:01Z", "claim-b")
            .await
            .expect("claim");
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn retry_marks_failed_after_max_attempts() {
        let pool = test_pool().await;
        create_digest_run_if_absent(&pool, run("digest-1", "2026-05-07T01:00:00Z"))
            .await
            .expect("insert");

        for attempt in 1..=CEO_DIGEST_MAX_ATTEMPTS {
            // If needed, adjust these timestamps so retry_waiting is due before the next claim.
            let claim_now = format!("2026-05-07T01:00:0{attempt}Z");
            let next_retry_at = format!("2026-05-07T01:00:0{}Z", attempt + 1);
            let claim = claim_due_digest_run(&pool, &claim_now, &format!("claim-{attempt}"))
                .await
                .expect("claim")
                .expect("claimed");
            mark_digest_retry_or_failed(
                &pool,
                &claim.id,
                &claim.claim_token,
                "AGENT_PROVIDER_REQUEST_FAILED",
                serde_json::json!({"message": "network unavailable"}),
                Some(next_retry_at),
            )
            .await
            .expect("mark retry or failed");
        }

        let row = sqlx::query(
            "SELECT status, attempt_count FROM agent_digest_runs WHERE id = 'digest-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(row.get::<String, _>("status"), DIGEST_STATUS_FAILED);
        assert_eq!(row.get::<i64, _>("attempt_count"), CEO_DIGEST_MAX_ATTEMPTS);
    }

    #[tokio::test]
    async fn delivered_requires_current_claim_token() {
        let pool = test_pool().await;
        create_digest_run_if_absent(&pool, run("digest-1", "2026-05-07T01:00:00Z"))
            .await
            .expect("insert");

        let claim = claim_due_digest_run(&pool, "2026-05-07T01:00:01Z", "claim-a")
            .await
            .expect("claim")
            .expect("claimed");

        mark_digest_delivered(
            &pool,
            &claim.id,
            "claim-b",
            serde_json::json!({"message_count": 1}),
        )
        .await
        .expect_err("stale claim rejected");

        mark_digest_delivered(
            &pool,
            &claim.id,
            &claim.claim_token,
            serde_json::json!({"message_count": 1}),
        )
        .await
        .expect("delivered");

        let row = sqlx::query(
            "SELECT status, delivery_summary_json FROM agent_digest_runs WHERE id = 'digest-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(row.get::<String, _>("status"), DIGEST_STATUS_DELIVERED);
        assert_eq!(
            row.get::<String, _>("delivery_summary_json"),
            serde_json::json!({"message_count": 1}).to_string()
        );
    }
}
