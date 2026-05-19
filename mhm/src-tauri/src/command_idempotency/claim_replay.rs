use super::{
    actor_type_as_str, system_error, CommandStatus, IdempotentCommandResult,
    PreparedWriteCommandRequest, WriteCommandContext, CLAIM_LEASE_SECONDS,
};
use crate::app_error::{codes, CommandError, CommandResult};
use chrono::{DateTime, Utc};
use sqlx::{sqlite::SqliteRow, Pool, Row, Sqlite, Transaction};

pub(super) enum ClaimOutcome {
    Claimed { claim_token: String },
    Replayed(IdempotentCommandResult<serde_json::Value>),
}

pub(super) async fn claim_or_reclaim(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    prepared: &PreparedWriteCommandRequest,
) -> CommandResult<ClaimOutcome> {
    let now = Utc::now();
    let now_string = now.to_rfc3339();
    let lease_expires_at = (now + chrono::Duration::seconds(CLAIM_LEASE_SECONDS)).to_rfc3339();
    let claim_token = uuid::Uuid::new_v4().to_string();

    if let Some(row) = fetch_existing_claim(pool, ctx).await? {
        if existing_claim_is_reclaimable(&row, &prepared.request_hash, now)? {
            if reclaim_claim(
                pool,
                ctx,
                prepared,
                &claim_token,
                &now_string,
                &lease_expires_at,
            )
            .await?
            {
                return Ok(ClaimOutcome::Claimed { claim_token });
            }

            return resolve_existing_claim(pool, ctx, &prepared.request_hash).await;
        }

        return resolve_existing_claim_row(ctx, &prepared.request_hash, row)
            .map(ClaimOutcome::Replayed);
    }

    let mut claim_tx = pool.begin().await.map_err(system_error)?;
    let claim_result = sqlx::query(
        "INSERT OR IGNORE INTO command_idempotency (
            idempotency_key,
            command_name,
            request_id,
            actor_type,
            actor_id,
            client_id,
            session_id,
            channel_id,
            issued_at,
            request_hash,
            intent_json,
            summary_json,
            primary_aggregate_key,
            lock_keys_json,
            status,
            claim_token,
            response_json,
            result_summary_json,
            error_code,
            error_json,
            error_summary_json,
            retryable,
            lease_expires_at,
            created_at,
            updated_at,
            completed_at,
            last_attempt_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&ctx.idempotency_key)
    .bind(&ctx.command_name)
    .bind(&ctx.request_id)
    .bind(actor_type_as_str(ctx.actor_type))
    .bind(&ctx.actor_id)
    .bind(&ctx.client_id)
    .bind(&ctx.session_id)
    .bind(&ctx.channel_id)
    .bind(ctx.issued_at.to_rfc3339())
    .bind(&prepared.request_hash)
    .bind(&prepared.intent_json)
    .bind(&prepared.summary_json)
    .bind(&prepared.primary_aggregate_key)
    .bind(&prepared.lock_keys_json)
    .bind(CommandStatus::InProgress.as_str())
    .bind(&claim_token)
    .bind(Option::<String>::None)
    .bind(Option::<String>::None)
    .bind(Option::<String>::None)
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
        return resolve_existing_claim(pool, ctx, &prepared.request_hash).await;
    }

    Ok(ClaimOutcome::Claimed { claim_token })
}

async fn resolve_existing_claim(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    request_hash: &str,
) -> CommandResult<ClaimOutcome> {
    let row = fetch_existing_claim(pool, ctx)
        .await?
        .ok_or_else(|| system_error("Idempotency claim was not inserted and no row exists"))?;

    resolve_existing_claim_row(ctx, request_hash, row).map(ClaimOutcome::Replayed)
}

async fn reclaim_claim(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    prepared: &PreparedWriteCommandRequest,
    claim_token: &str,
    now: &str,
    lease_expires_at: &str,
) -> CommandResult<bool> {
    let mut tx = pool.begin().await.map_err(system_error)?;
    let result = sqlx::query(
        "UPDATE command_idempotency
         SET status = 'in_progress',
             claim_token = ?,
             response_json = NULL,
             result_summary_json = NULL,
             error_code = NULL,
             error_json = NULL,
             error_summary_json = NULL,
             retryable = 0,
             lease_expires_at = ?,
             recovery_dismissed_at = NULL,
             recovery_dismissed_by = NULL,
             updated_at = ?,
             completed_at = NULL,
             last_attempt_at = ?
         WHERE command_name = ?
           AND idempotency_key = ?
           AND request_hash = ?
           AND (
                status = 'failed_retryable'
                OR (
                    status = 'in_progress'
                    AND (lease_expires_at IS NULL OR lease_expires_at <= ?)
                )
           )",
    )
    .bind(claim_token)
    .bind(lease_expires_at)
    .bind(now)
    .bind(now)
    .bind(&ctx.command_name)
    .bind(&ctx.idempotency_key)
    .bind(&prepared.request_hash)
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
        "SELECT request_hash, status, response_json, error_json, lease_expires_at
         FROM command_idempotency
         WHERE command_name = ? AND idempotency_key = ?",
    )
    .bind(&ctx.command_name)
    .bind(&ctx.idempotency_key)
    .fetch_optional(pool)
    .await
    .map_err(system_error)
}

pub(super) async fn fetch_existing_claim_tx(
    tx: &mut Transaction<'_, Sqlite>,
    ctx: &WriteCommandContext,
) -> CommandResult<Option<SqliteRow>> {
    sqlx::query(
        "SELECT request_hash, status, response_json, error_json, lease_expires_at
         FROM command_idempotency
         WHERE command_name = ? AND idempotency_key = ?",
    )
    .bind(&ctx.command_name)
    .bind(&ctx.idempotency_key)
    .fetch_optional(&mut **tx)
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

    let status = CommandStatus::from_str(row.get::<String, _>("status").as_str())?;
    match status {
        CommandStatus::FailedRetryable => return Ok(true),
        CommandStatus::InProgress => {}
        CommandStatus::Completed | CommandStatus::FailedTerminal => return Ok(false),
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

pub(super) fn resolve_existing_claim_row(
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

    let status = CommandStatus::from_str(row.get::<String, _>("status").as_str())?;
    match status {
        CommandStatus::Completed => {
            let raw_response = row
                .try_get::<Option<String>, _>("response_json")
                .map_err(system_error)?
                .ok_or_else(|| {
                    system_error("completed idempotency row is missing response_json")
                })?;
            let response = serde_json::from_str(&raw_response).map_err(system_error)?;
            Ok(IdempotentCommandResult {
                response,
                replayed: true,
            })
        }
        CommandStatus::FailedTerminal => {
            let raw_error = row
                .try_get::<Option<String>, _>("error_json")
                .map_err(system_error)?
                .ok_or_else(|| system_error("terminal idempotency row is missing error_json"))?;
            let mut error: CommandError = serde_json::from_str(&raw_error).map_err(system_error)?;
            error.request_id = Some(ctx.request_id.clone());
            Err(error)
        }
        CommandStatus::InProgress | CommandStatus::FailedRetryable => Err(CommandError::user(
            codes::CONFLICT_DUPLICATE_IN_FLIGHT,
            "A command with this idempotency key is already in progress",
        )
        .with_request_id(ctx.request_id.clone())),
    }
}
