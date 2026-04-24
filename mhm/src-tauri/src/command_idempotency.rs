use crate::{
    app_error::{codes, CommandError, CommandResult},
    services::settings_store,
};
use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{sqlite::SqliteRow, Pool, Row, Sqlite};

pub const SET_CRASH_REPORTING_PREFERENCE_COMMAND: &str = "settings.set_crash_reporting_preference";
const CLAIM_LEASE_SECONDS: i64 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorType {
    Human,
    AiAgent,
    System,
    Integration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteCommandContext {
    pub request_id: String,
    pub idempotency_key: String,
    pub command_name: String,
    pub actor_id: Option<String>,
    pub actor_type: ActorType,
    pub client_id: Option<String>,
    pub session_id: Option<String>,
    pub channel_id: Option<String>,
    pub issued_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdempotentCommandResult<T> {
    pub response: T,
    pub replayed: bool,
}

impl WriteCommandContext {
    pub fn new_internal(command_name: &str) -> Self {
        Self {
            request_id: uuid::Uuid::new_v4().to_string(),
            idempotency_key: uuid::Uuid::new_v4().to_string(),
            command_name: command_name.to_string(),
            actor_id: None,
            actor_type: ActorType::System,
            client_id: None,
            session_id: None,
            channel_id: None,
            issued_at: chrono::Local::now().fixed_offset(),
        }
    }

    #[cfg(test)]
    pub fn for_internal_test(request_id: &str, idempotency_key: &str, command_name: &str) -> Self {
        let issued_at = DateTime::parse_from_rfc3339("2026-04-24T10:00:00+07:00")
            .expect("fixed test timestamp parses");

        Self {
            request_id: request_id.to_string(),
            idempotency_key: idempotency_key.to_string(),
            command_name: command_name.to_string(),
            actor_id: Some("test".to_string()),
            actor_type: ActorType::System,
            client_id: None,
            session_id: None,
            channel_id: None,
            issued_at,
        }
    }
}

pub async fn set_crash_reporting_preference_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    enabled: bool,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let intent = serde_json::json!({ "enabled": enabled });
    let intent_json = stable_json_string(&intent)?;
    let request_hash = stable_request_hash(&intent)?;
    let now = Utc::now();
    let now_string = now.to_rfc3339();
    let lease_expires_at = (now + chrono::Duration::seconds(CLAIM_LEASE_SECONDS)).to_rfc3339();
    let response = serde_json::json!({ "ok": true });
    let response_json = stable_json_string(&response)?;
    let claim_token = uuid::Uuid::new_v4().to_string();

    if let Some(row) = fetch_existing_claim(pool, ctx).await? {
        if existing_claim_is_reclaimable(&row, &request_hash, now)? {
            if reclaim_expired_claim(
                pool,
                ctx,
                &request_hash,
                &intent_json,
                &claim_token,
                &now_string,
                &lease_expires_at,
            )
            .await?
            {
                return complete_claimed_crash_reporting_preference(
                    pool,
                    ctx,
                    enabled,
                    &claim_token,
                    response,
                    &response_json,
                )
                .await;
            }

            return resolve_existing_claim(pool, ctx, &request_hash).await;
        }
        return resolve_existing_claim_row(ctx, &request_hash, row);
    }

    let mut claim_tx = pool.begin().await.map_err(system_error)?;
    let claim_result = sqlx::query(
        "INSERT OR IGNORE INTO command_idempotency (
            idempotency_key,
            command_name,
            request_hash,
            intent_json,
            primary_aggregate_key,
            lock_keys_json,
            status,
            claim_token,
            response_json,
            error_code,
            retryable,
            lease_expires_at,
            created_at,
            updated_at,
            completed_at,
            last_attempt_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&ctx.idempotency_key)
    .bind(&ctx.command_name)
    .bind(&request_hash)
    .bind(&intent_json)
    .bind(Option::<String>::None)
    .bind("[]")
    .bind("in_progress")
    .bind(&claim_token)
    .bind(Option::<String>::None)
    .bind(Option::<String>::None)
    .bind(0_i64)
    .bind(&lease_expires_at)
    .bind(&now_string)
    .bind(&now_string)
    .bind(Option::<String>::None)
    .bind(&now_string)
    .execute(&mut *claim_tx)
    .await
    .map_err(system_error)?;

    claim_tx.commit().await.map_err(system_error)?;

    if claim_result.rows_affected() != 1 {
        return resolve_existing_claim(pool, ctx, &request_hash).await;
    }

    complete_claimed_crash_reporting_preference(
        pool,
        ctx,
        enabled,
        &claim_token,
        response,
        &response_json,
    )
    .await
}

async fn complete_claimed_crash_reporting_preference(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    enabled: bool,
    claim_token: &str,
    response: serde_json::Value,
    response_json: &str,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let mut tx = pool.begin().await.map_err(system_error)?;

    settings_store::save_setting_tx(
        &mut tx,
        "send_crash_reports",
        if enabled { "true" } else { "false" },
    )
    .await
    .map_err(system_error)?;

    let completed_at = chrono::Utc::now().to_rfc3339();
    let completion_result = sqlx::query(
        "UPDATE command_idempotency
         SET status = 'completed',
             response_json = ?,
             lease_expires_at = NULL,
             updated_at = ?,
             completed_at = ?
         WHERE command_name = ? AND idempotency_key = ? AND claim_token = ?",
    )
    .bind(response_json)
    .bind(&completed_at)
    .bind(&completed_at)
    .bind(&ctx.command_name)
    .bind(&ctx.idempotency_key)
    .bind(claim_token)
    .execute(&mut *tx)
    .await
    .map_err(system_error)?;

    if completion_result.rows_affected() != 1 {
        return Err(CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            "Failed to complete claimed idempotency row",
        ));
    }

    tx.commit().await.map_err(system_error)?;

    Ok(IdempotentCommandResult {
        response,
        replayed: false,
    })
}

fn stable_json_string(value: &serde_json::Value) -> CommandResult<String> {
    serde_json::to_string(value).map_err(system_error)
}

fn stable_request_hash(intent: &serde_json::Value) -> CommandResult<String> {
    let intent_json = stable_json_string(intent)?;
    let digest = Sha256::digest(intent_json.as_bytes());
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

async fn resolve_existing_claim(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    request_hash: &str,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let row = fetch_existing_claim(pool, ctx)
        .await?
        .ok_or_else(|| system_error("Idempotency claim was not inserted and no row exists"))?;

    resolve_existing_claim_row(ctx, request_hash, row)
}

async fn reclaim_expired_claim(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    request_hash: &str,
    intent_json: &str,
    claim_token: &str,
    now: &str,
    lease_expires_at: &str,
) -> CommandResult<bool> {
    let mut tx = pool.begin().await.map_err(system_error)?;
    let result = sqlx::query(
        "UPDATE command_idempotency
         SET request_hash = ?,
             intent_json = ?,
             status = 'in_progress',
             claim_token = ?,
             response_json = NULL,
             error_code = NULL,
             retryable = 0,
             lease_expires_at = ?,
             updated_at = ?,
             completed_at = NULL,
             last_attempt_at = ?
         WHERE command_name = ?
           AND idempotency_key = ?
           AND request_hash = ?
           AND status = 'in_progress'
           AND (lease_expires_at IS NULL OR lease_expires_at <= ?)",
    )
    .bind(request_hash)
    .bind(intent_json)
    .bind(claim_token)
    .bind(lease_expires_at)
    .bind(now)
    .bind(now)
    .bind(&ctx.command_name)
    .bind(&ctx.idempotency_key)
    .bind(request_hash)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(system_error)?;

    tx.commit().await.map_err(system_error)?;

    Ok(result.rows_affected() == 1)
}

async fn fetch_existing_claim(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
) -> CommandResult<Option<SqliteRow>> {
    sqlx::query(
        "SELECT request_hash, status, response_json, lease_expires_at
         FROM command_idempotency
         WHERE command_name = ? AND idempotency_key = ?",
    )
    .bind(&ctx.command_name)
    .bind(&ctx.idempotency_key)
    .fetch_optional(pool)
    .await
    .map_err(system_error)
}

fn existing_claim_is_reclaimable(
    row: &SqliteRow,
    request_hash: &str,
    now: DateTime<Utc>,
) -> CommandResult<bool> {
    let stored_hash: String = row.get("request_hash");
    if stored_hash != request_hash {
        return Ok(false);
    }

    let status: String = row.get("status");
    if status != "in_progress" {
        return Ok(false);
    }

    let lease_expires_at: Option<String> = row.try_get("lease_expires_at").map_err(system_error)?;
    let Some(lease_expires_at) = lease_expires_at else {
        return Ok(true);
    };

    let lease_expires_at = DateTime::parse_from_rfc3339(&lease_expires_at)
        .map_err(system_error)?
        .with_timezone(&Utc);

    Ok(lease_expires_at <= now)
}

fn resolve_existing_claim_row(
    ctx: &WriteCommandContext,
    request_hash: &str,
    row: SqliteRow,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let stored_hash: String = row.get("request_hash");
    if stored_hash != request_hash {
        return Err(CommandError::user(
            codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH,
            "Idempotency key was reused with a different request payload",
        )
        .with_request_id(ctx.request_id.clone()));
    }

    let status: String = row.get("status");
    if status == "completed" {
        let raw_response = row
            .try_get::<Option<String>, _>("response_json")
            .map_err(system_error)?
            .ok_or_else(|| system_error("completed idempotency row is missing response_json"))?;
        let response = serde_json::from_str(&raw_response).map_err(system_error)?;
        return Ok(IdempotentCommandResult {
            response,
            replayed: true,
        });
    }

    Err(CommandError::user(
        codes::CONFLICT_DUPLICATE_IN_FLIGHT,
        "A command with this idempotency key is already in progress",
    )
    .with_request_id(ctx.request_id.clone()))
}

fn system_error(error: impl std::fmt::Display) -> CommandError {
    CommandError::system(codes::SYSTEM_INTERNAL_ERROR, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_error::codes;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> Pool<Sqlite> {
        let database_url = format!(
            "sqlite://file:{}?mode=memory&cache=shared",
            uuid::Uuid::new_v4()
        );

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .expect("failed to open sqlite test pool");

        crate::db::run_migrations(&pool)
            .await
            .expect("failed to run migrations");

        pool
    }

    #[tokio::test]
    async fn set_crash_reporting_preference_exact_retry_returns_cached_response() {
        let pool = test_pool().await;
        let ctx = WriteCommandContext::for_internal_test(
            "test-request-retry",
            "idem-crash-pref-true",
            SET_CRASH_REPORTING_PREFERENCE_COMMAND,
        );

        let first = set_crash_reporting_preference_idempotent(&pool, &ctx, true)
            .await
            .expect("first write succeeds");
        let second = set_crash_reporting_preference_idempotent(&pool, &ctx, true)
            .await
            .expect("exact retry succeeds");

        assert_eq!(first.response, serde_json::json!({ "ok": true }));
        assert!(!first.replayed);
        assert_eq!(second.response, serde_json::json!({ "ok": true }));
        assert!(second.replayed);
    }

    #[tokio::test]
    async fn set_crash_reporting_preference_same_key_different_payload_conflicts() {
        let pool = test_pool().await;
        let ctx = WriteCommandContext::for_internal_test(
            "test-request-conflict",
            "idem-crash-pref-conflict",
            SET_CRASH_REPORTING_PREFERENCE_COMMAND,
        );

        set_crash_reporting_preference_idempotent(&pool, &ctx, true)
            .await
            .expect("first write succeeds");

        let error = set_crash_reporting_preference_idempotent(&pool, &ctx, false)
            .await
            .expect_err("same key with different payload conflicts");

        assert_eq!(error.code, codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH);
        assert_eq!(error.request_id.as_deref(), Some(ctx.request_id.as_str()));
    }

    #[tokio::test]
    async fn set_crash_reporting_preference_same_key_in_progress_conflicts() {
        let pool = test_pool().await;
        let ctx = WriteCommandContext::for_internal_test(
            "test-request-in-flight",
            "idem-crash-pref-in-flight",
            SET_CRASH_REPORTING_PREFERENCE_COMMAND,
        );
        let intent = serde_json::json!({ "enabled": true });
        let now = ctx.issued_at.to_rfc3339();
        let lease_expires_at =
            (ctx.issued_at + chrono::Duration::seconds(CLAIM_LEASE_SECONDS)).to_rfc3339();

        sqlx::query(
            "INSERT INTO command_idempotency (
                idempotency_key,
                command_name,
                request_hash,
                intent_json,
                lock_keys_json,
                status,
                claim_token,
                retryable,
                lease_expires_at,
                created_at,
                updated_at,
                last_attempt_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&ctx.idempotency_key)
        .bind(&ctx.command_name)
        .bind(stable_request_hash(&intent).expect("intent hashes"))
        .bind(stable_json_string(&intent).expect("intent serializes"))
        .bind("[]")
        .bind("in_progress")
        .bind("other-claim-token")
        .bind(0_i64)
        .bind(&lease_expires_at)
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("seeds committed in-progress idempotency row");

        let error = set_crash_reporting_preference_idempotent(&pool, &ctx, true)
            .await
            .expect_err("matching in-progress claim conflicts");

        assert_eq!(error.code, codes::CONFLICT_DUPLICATE_IN_FLIGHT);
        assert_eq!(error.request_id.as_deref(), Some(ctx.request_id.as_str()));
    }

    #[tokio::test]
    async fn set_crash_reporting_preference_reclaims_expired_in_progress_claim() {
        let pool = test_pool().await;
        let ctx = WriteCommandContext::for_internal_test(
            "test-request-expired",
            "idem-crash-pref-expired",
            SET_CRASH_REPORTING_PREFERENCE_COMMAND,
        );
        let intent = serde_json::json!({ "enabled": true });
        let now = ctx.issued_at.to_rfc3339();
        let expired_lease = (Utc::now() - chrono::Duration::seconds(1)).to_rfc3339();

        sqlx::query(
            "INSERT INTO command_idempotency (
                idempotency_key,
                command_name,
                request_hash,
                intent_json,
                lock_keys_json,
                status,
                claim_token,
                retryable,
                lease_expires_at,
                created_at,
                updated_at,
                last_attempt_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&ctx.idempotency_key)
        .bind(&ctx.command_name)
        .bind(stable_request_hash(&intent).expect("intent hashes"))
        .bind(stable_json_string(&intent).expect("intent serializes"))
        .bind("[]")
        .bind("in_progress")
        .bind("expired-claim-token")
        .bind(0_i64)
        .bind(&expired_lease)
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("seeds expired in-progress idempotency row");

        let result = set_crash_reporting_preference_idempotent(&pool, &ctx, true)
            .await
            .expect("expired claim is reclaimed");

        assert!(!result.replayed);
        assert_eq!(result.response, serde_json::json!({ "ok": true }));

        let row = sqlx::query(
            "SELECT status, response_json, lease_expires_at
             FROM command_idempotency
             WHERE command_name = ? AND idempotency_key = ?",
        )
        .bind(&ctx.command_name)
        .bind(&ctx.idempotency_key)
        .fetch_one(&pool)
        .await
        .expect("reads reclaimed row");

        assert_eq!(row.get::<String, _>("status"), "completed");
        assert_eq!(
            row.get::<Option<String>, _>("response_json"),
            Some(serde_json::json!({ "ok": true }).to_string())
        );
        assert_eq!(row.get::<Option<String>, _>("lease_expires_at"), None);

        let setting: String =
            sqlx::query_scalar("SELECT value FROM settings WHERE key = 'send_crash_reports'")
                .fetch_one(&pool)
                .await
                .expect("reads crash reporting setting");
        assert_eq!(setting, "true");
    }
}
