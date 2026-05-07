use crate::{
    app_error::{codes, CommandError, CommandResult},
    command_idempotency::system_error,
};
use chrono::Utc;
use serde_json::Value;
use sqlx::{Pool, Row, Sqlite, Transaction};

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

fn digest_payload_conflict_error() -> CommandError {
    CommandError::system(
        codes::SYSTEM_INTERNAL_ERROR,
        "Digest run id already exists with different payload.",
    )
}

fn invalid_delivery_chat_id_error() -> CommandError {
    CommandError::system(
        codes::SYSTEM_INTERNAL_ERROR,
        "Digest run persisted delivery chat id is invalid.",
    )
}

fn missing_retry_time_error() -> CommandError {
    CommandError::system(
        codes::SYSTEM_INTERNAL_ERROR,
        "Digest retry requires next_retry_at before max attempts.",
    )
}

fn telegram_send_started(summary_json: &str) -> bool {
    serde_json::from_str::<Value>(summary_json)
        .ok()
        .and_then(|value| {
            value
                .get("telegram_send_started")
                .and_then(serde_json::Value::as_bool)
        })
        .unwrap_or(false)
}

fn retry_suppressed_after_send_started_summary(mut summary: Value) -> Value {
    match &mut summary {
        Value::Object(map) => {
            map.insert("retry_suppressed".to_string(), Value::Bool(true));
            map.insert(
                "retry_suppressed_reason".to_string(),
                Value::String("telegram_send_started".to_string()),
            );
            summary
        }
        _ => serde_json::json!({
            "retry_suppressed": true,
            "retry_suppressed_reason": "telegram_send_started",
        }),
    }
}

fn parse_delivery_chat_id(value: Option<String>) -> CommandResult<Option<i64>> {
    value
        .map(|value| {
            value
                .parse::<i64>()
                .map_err(|_| invalid_delivery_chat_id_error())
        })
        .transpose()
}

async fn update_selected_digest_run_claim_tx(
    tx: &mut Transaction<'_, Sqlite>,
    id: &str,
    now: &str,
    claim_token: &str,
) -> CommandResult<u64> {
    let result = sqlx::query(
        "UPDATE agent_digest_runs
         SET status = 'in_progress',
             attempt_count = attempt_count + 1,
             claimed_at = ?,
             claim_token = ?,
             next_retry_at = NULL,
             updated_at = ?
         WHERE id = ?
           AND due_at <= ?
           AND (
             status = 'pending'
             OR (status = 'retry_waiting' AND next_retry_at IS NOT NULL AND next_retry_at <= ?)
           )",
    )
    .bind(now)
    .bind(claim_token)
    .bind(now)
    .bind(id)
    .bind(now)
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(system_error)?;

    Ok(result.rows_affected())
}

fn target_matches_sql() -> &'static str {
    "AND ((? IS NULL AND channel_actor_id IS NULL) OR channel_actor_id = ?)
     AND ((? IS NULL AND delivery_chat_id IS NULL) OR delivery_chat_id = ?)"
}

pub async fn create_digest_run_if_absent(
    pool: &Pool<Sqlite>,
    input: NewDigestRun,
) -> CommandResult<()> {
    let now = Utc::now().to_rfc3339();
    let result = sqlx::query(
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

    if result.rows_affected() > 0 {
        return Ok(());
    }

    let row = sqlx::query(
        "SELECT channel_actor_id, delivery_chat_id, due_at, max_attempts
         FROM agent_digest_runs
         WHERE id = ?",
    )
    .bind(&input.id)
    .fetch_one(pool)
    .await
    .map_err(system_error)?;

    let persisted_channel_actor_id: Option<String> = row.get("channel_actor_id");
    let persisted_delivery_chat_id =
        parse_delivery_chat_id(row.get::<Option<String>, _>("delivery_chat_id"))?;
    let persisted_due_at: String = row.get("due_at");
    let persisted_max_attempts: i64 = row.get("max_attempts");

    if persisted_channel_actor_id == input.channel_actor_id
        && persisted_delivery_chat_id == input.delivery_chat_id
        && persisted_due_at == input.due_at
        && persisted_max_attempts == input.max_attempts
    {
        return Ok(());
    }

    Err(digest_payload_conflict_error())
}

pub async fn claim_due_digest_run(
    pool: &Pool<Sqlite>,
    now: &str,
    claim_token: &str,
) -> CommandResult<Option<ClaimedDigestRun>> {
    claim_due_digest_run_internal(pool, now, claim_token, None, None, false).await
}

pub async fn claim_due_digest_run_for_target(
    pool: &Pool<Sqlite>,
    now: &str,
    claim_token: &str,
    channel_actor_id: Option<&str>,
    delivery_chat_id: Option<i64>,
) -> CommandResult<Option<ClaimedDigestRun>> {
    claim_due_digest_run_internal(
        pool,
        now,
        claim_token,
        channel_actor_id,
        delivery_chat_id,
        true,
    )
    .await
}

async fn claim_due_digest_run_internal(
    pool: &Pool<Sqlite>,
    now: &str,
    claim_token: &str,
    channel_actor_id: Option<&str>,
    delivery_chat_id: Option<i64>,
    filter_target: bool,
) -> CommandResult<Option<ClaimedDigestRun>> {
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(system_error)?;
    let delivery_chat_id = delivery_chat_id.map(|value| value.to_string());
    let query = if filter_target {
        format!(
            "SELECT id, channel_actor_id, delivery_chat_id, due_at, attempt_count, max_attempts
         FROM agent_digest_runs
         WHERE (
             status = 'pending'
             OR (status = 'retry_waiting' AND next_retry_at IS NOT NULL AND next_retry_at <= ?)
         )
           AND due_at <= ?
           {}
         ORDER BY due_at ASC
         LIMIT 1",
            target_matches_sql()
        )
    } else {
        "SELECT id, channel_actor_id, delivery_chat_id, due_at, attempt_count, max_attempts
         FROM agent_digest_runs
         WHERE (
             status = 'pending'
             OR (status = 'retry_waiting' AND next_retry_at IS NOT NULL AND next_retry_at <= ?)
         )
           AND due_at <= ?
         ORDER BY due_at ASC
         LIMIT 1"
            .to_string()
    };

    let mut query = sqlx::query(&query).bind(now).bind(now);
    if filter_target {
        query = query
            .bind(channel_actor_id)
            .bind(channel_actor_id)
            .bind(&delivery_chat_id)
            .bind(&delivery_chat_id);
    }
    let row = query.fetch_optional(&mut *tx).await.map_err(system_error)?;

    let Some(row) = row else {
        tx.commit().await.map_err(system_error)?;
        return Ok(None);
    };

    let id: String = row.get("id");
    let delivery_chat_id = parse_delivery_chat_id(row.get("delivery_chat_id"))?;
    let rows_affected = update_selected_digest_run_claim_tx(&mut tx, &id, now, claim_token).await?;

    if rows_affected == 0 {
        tx.commit().await.map_err(system_error)?;
        return Ok(None);
    }

    let claimed = ClaimedDigestRun {
        id,
        channel_actor_id: row.get("channel_actor_id"),
        delivery_chat_id,
        due_at: row.get("due_at"),
        attempt_count: row.get::<i64, _>("attempt_count") + 1,
        max_attempts: row.get("max_attempts"),
        claim_token: claim_token.to_string(),
    };
    tx.commit().await.map_err(system_error)?;
    Ok(Some(claimed))
}

pub async fn mark_digest_telegram_send_started(
    pool: &Pool<Sqlite>,
    id: &str,
    claim_token: &str,
    summary: Value,
) -> CommandResult<()> {
    let now = Utc::now().to_rfc3339();
    let result = sqlx::query(
        "UPDATE agent_digest_runs
         SET delivery_summary_json = ?,
             updated_at = ?
         WHERE id = ? AND status = 'in_progress' AND claim_token = ?",
    )
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
    let row = sqlx::query(
        "SELECT attempt_count, max_attempts, delivery_summary_json
         FROM agent_digest_runs
         WHERE id = ?",
    )
    .bind(id)
    .fetch_one(pool)
    .await
    .map_err(system_error)?;
    let attempt_count: i64 = row.get("attempt_count");
    let max_attempts: i64 = row.get("max_attempts");
    let send_started = telegram_send_started(&row.get::<String, _>("delivery_summary_json"));
    let status = if send_started || attempt_count >= max_attempts {
        DIGEST_STATUS_FAILED
    } else {
        DIGEST_STATUS_RETRY_WAITING
    };
    let retry_at = if status == DIGEST_STATUS_FAILED {
        None
    } else {
        Some(next_retry_at.ok_or_else(missing_retry_time_error)?)
    };
    let error_summary = if send_started {
        retry_suppressed_after_send_started_summary(error_summary)
    } else {
        error_summary
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

    async fn test_pool_with_max_connections(max_connections: u32) -> Pool<Sqlite> {
        let database_url = format!(
            "sqlite://file:{}?mode=memory&cache=shared",
            uuid::Uuid::new_v4()
        );
        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .connect(&database_url)
            .await
            .expect("connect sqlite");
        crate::db::run_migrations(&pool).await.expect("migrations");
        pool
    }

    async fn test_pool() -> Pool<Sqlite> {
        test_pool_with_max_connections(1).await
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

    fn run_for_target(
        id: &str,
        due_at: &str,
        channel_actor_id: &str,
        delivery_chat_id: i64,
    ) -> NewDigestRun {
        NewDigestRun {
            id: id.to_string(),
            channel_actor_id: Some(channel_actor_id.to_string()),
            delivery_chat_id: Some(delivery_chat_id),
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
    async fn create_digest_run_rejects_same_id_with_different_payload() {
        let pool = test_pool().await;
        create_digest_run_if_absent(&pool, run("digest-1", "2026-05-07T01:00:00Z"))
            .await
            .expect("first insert");

        let mut drifted = run("digest-1", "2026-05-07T02:00:00Z");
        drifted.delivery_chat_id = Some(66);
        create_digest_run_if_absent(&pool, drifted)
            .await
            .expect_err("payload drift rejected");

        let row = sqlx::query(
            "SELECT due_at, channel_actor_id, delivery_chat_id, max_attempts
             FROM agent_digest_runs
             WHERE id = 'digest-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(row.get::<String, _>("due_at"), "2026-05-07T01:00:00Z");
        assert_eq!(
            row.get::<Option<String>, _>("channel_actor_id"),
            Some("123".to_string())
        );
        assert_eq!(
            row.get::<Option<String>, _>("delivery_chat_id"),
            Some("55".to_string())
        );
        assert_eq!(row.get::<i64, _>("max_attempts"), CEO_DIGEST_MAX_ATTEMPTS);
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
    async fn claim_due_digest_run_for_target_skips_stale_delivery_target() {
        let pool = test_pool().await;
        create_digest_run_if_absent(
            &pool,
            run_for_target("digest-old-chat", "2026-05-07T01:00:00Z", "123", 55),
        )
        .await
        .expect("insert old chat run");
        create_digest_run_if_absent(
            &pool,
            run_for_target("digest-current-chat", "2026-05-07T01:00:01Z", "123", 66),
        )
        .await
        .expect("insert current chat run");

        let claim = claim_due_digest_run_for_target(
            &pool,
            "2026-05-07T01:00:02Z",
            "claim-current",
            Some("123"),
            Some(66),
        )
        .await
        .expect("targeted claim")
        .expect("current target claimed");

        assert_eq!(claim.id, "digest-current-chat");
        assert_eq!(claim.delivery_chat_id, Some(66));

        let stale = sqlx::query(
            "SELECT status, attempt_count, claim_token
             FROM agent_digest_runs
             WHERE id = 'digest-old-chat'",
        )
        .fetch_one(&pool)
        .await
        .expect("stale target row");
        assert_eq!(stale.get::<String, _>("status"), DIGEST_STATUS_PENDING);
        assert_eq!(stale.get::<i64, _>("attempt_count"), 0);
        assert_eq!(stale.get::<Option<String>, _>("claim_token"), None);
    }

    #[tokio::test]
    async fn concurrent_claims_return_one_claim_and_one_none() {
        let pool = test_pool_with_max_connections(2).await;
        create_digest_run_if_absent(&pool, run("digest-1", "2026-05-07T01:00:00Z"))
            .await
            .expect("insert");

        let pool_a = pool.clone();
        let pool_b = pool.clone();
        let claim_a = tokio::spawn(async move {
            claim_due_digest_run(&pool_a, "2026-05-07T01:00:01Z", "claim-a").await
        });
        let claim_b = tokio::spawn(async move {
            claim_due_digest_run(&pool_b, "2026-05-07T01:00:01Z", "claim-b").await
        });
        let (claim_a, claim_b) = tokio::join!(claim_a, claim_b);

        let results = vec![
            claim_a.expect("claim task a").expect("claim a"),
            claim_b.expect("claim task b").expect("claim b"),
        ];
        assert_eq!(results.iter().filter(|result| result.is_some()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_none()).count(), 1);

        let row = sqlx::query(
            "SELECT status, attempt_count, claim_token FROM agent_digest_runs WHERE id = 'digest-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(row.get::<String, _>("status"), DIGEST_STATUS_IN_PROGRESS);
        assert_eq!(row.get::<i64, _>("attempt_count"), 1);
        assert!(["claim-a", "claim-b"].contains(
            &row.get::<Option<String>, _>("claim_token")
                .expect("claim token")
                .as_str()
        ));
    }

    #[tokio::test]
    async fn claim_due_digest_run_fails_closed_on_invalid_delivery_chat_id() {
        let pool = test_pool().await;
        create_digest_run_if_absent(&pool, run("digest-1", "2026-05-07T01:00:00Z"))
            .await
            .expect("insert");
        sqlx::query("UPDATE agent_digest_runs SET delivery_chat_id = ? WHERE id = ?")
            .bind("not-an-integer")
            .bind("digest-1")
            .execute(&pool)
            .await
            .expect("corrupt delivery chat id");

        claim_due_digest_run(&pool, "2026-05-07T01:00:01Z", "claim-a")
            .await
            .expect_err("invalid delivery chat id rejected");
    }

    #[tokio::test]
    async fn claim_update_rechecks_due_time_after_selection() {
        let pool = test_pool().await;
        create_digest_run_if_absent(&pool, run("digest-1", "2026-05-07T01:00:00Z"))
            .await
            .expect("insert");

        let mut tx = pool.begin().await.expect("begin tx");
        sqlx::query("UPDATE agent_digest_runs SET due_at = ? WHERE id = ?")
            .bind("2026-05-07T02:00:00Z")
            .bind("digest-1")
            .execute(&mut *tx)
            .await
            .expect("move due_at forward");

        let rows_affected = update_selected_digest_run_claim_tx(
            &mut tx,
            "digest-1",
            "2026-05-07T01:00:01Z",
            "claim-a",
        )
        .await
        .expect("claim update");
        tx.commit().await.expect("commit");

        assert_eq!(rows_affected, 0);

        let row = sqlx::query(
            "SELECT status, attempt_count, claim_token FROM agent_digest_runs WHERE id = ?",
        )
        .bind("digest-1")
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(row.get::<String, _>("status"), DIGEST_STATUS_PENDING);
        assert_eq!(row.get::<i64, _>("attempt_count"), 0);
        assert_eq!(row.get::<Option<String>, _>("claim_token"), None);
    }

    #[tokio::test]
    async fn retry_requires_next_retry_time_before_max_attempts() {
        let pool = test_pool().await;
        create_digest_run_if_absent(&pool, run("digest-1", "2026-05-07T01:00:00Z"))
            .await
            .expect("insert");
        let claim = claim_due_digest_run(&pool, "2026-05-07T01:00:01Z", "claim-a")
            .await
            .expect("claim")
            .expect("claimed");

        mark_digest_retry_or_failed(
            &pool,
            &claim.id,
            &claim.claim_token,
            "AGENT_PROVIDER_REQUEST_FAILED",
            serde_json::json!({"message": "network unavailable"}),
            None,
        )
        .await
        .expect_err("retry without next_retry_at rejected");

        let row = sqlx::query(
            "SELECT status, attempt_count, claim_token, next_retry_at
             FROM agent_digest_runs
             WHERE id = 'digest-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("row");
        assert_eq!(row.get::<String, _>("status"), DIGEST_STATUS_IN_PROGRESS);
        assert_eq!(row.get::<i64, _>("attempt_count"), 1);
        assert_eq!(
            row.get::<Option<String>, _>("claim_token"),
            Some("claim-a".to_string())
        );
        assert_eq!(row.get::<Option<String>, _>("next_retry_at"), None);
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
