use crate::app_error::{codes, CommandError, CommandResult};
use crate::command_ledger::{
    get_command_ledger_detail, CommandLedgerDetail, CommandLedgerListItem,
};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Row, Sqlite, Transaction};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryRiskLevel {
    Low,
    High,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandRecoveryQueueItem {
    pub ledger: CommandLedgerListItem,
    pub recovery_status: String,
    pub risk_level: RecoveryRiskLevel,
    pub requires_confirmation: bool,
    pub allowed_actions: Vec<String>,
}

fn system_error(error: impl std::fmt::Display) -> CommandError {
    CommandError::system(codes::SYSTEM_INTERNAL_ERROR, error.to_string())
}

fn recovery_status(status: &str, lease_expires_at: Option<&str>, now: &str) -> Option<String> {
    match status {
        "in_progress" if lease_expires_at.is_some_and(|lease| lease <= now) => {
            Some("expired_in_progress".to_string())
        }
        "failed_retryable" => Some("failed_retryable".to_string()),
        _ => None,
    }
}

pub fn command_recovery_risk_level(command_name: &str) -> RecoveryRiskLevel {
    match command_name {
        "check_out"
        | "record_payment"
        | "add_folio_line"
        | "modify_reservation"
        | "cancel_reservation"
        | "confirm_reservation"
        | "check_in"
        | "extend_stay"
        | "group_checkin"
        | "group_checkout"
        | "generate_invoice" => RecoveryRiskLevel::High,
        name if name.contains("night_audit") => RecoveryRiskLevel::High,
        _ => RecoveryRiskLevel::Low,
    }
}

fn requires_confirmation(command_name: &str) -> bool {
    command_recovery_risk_level(command_name) == RecoveryRiskLevel::High
}

fn allowed_actions_for_queue_item() -> Vec<String> {
    ["inspect", "request_retry", "dismiss", "mark_terminal"]
        .into_iter()
        .map(str::to_string)
        .collect()
}

pub async fn list_command_recovery_queue(
    pool: &Pool<Sqlite>,
) -> CommandResult<Vec<CommandRecoveryQueueItem>> {
    let now = chrono::Utc::now().to_rfc3339();
    let rows = sqlx::query(
        "SELECT id
         FROM command_idempotency
         WHERE recovery_dismissed_at IS NULL
           AND (
                (status = 'in_progress' AND lease_expires_at IS NOT NULL AND lease_expires_at <= ?)
                OR status = 'failed_retryable'
           )
         ORDER BY
            CASE
                WHEN status = 'in_progress' THEN 0
                WHEN status = 'failed_retryable' THEN 1
                ELSE 2
            END,
            updated_at DESC",
    )
    .bind(&now)
    .fetch_all(pool)
    .await
    .map_err(system_error)?;

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let id: i64 = row.try_get("id").map_err(system_error)?;
        let detail = get_command_ledger_detail(pool, id).await?;
        let Some(status) =
            recovery_status(&detail.status, detail.lease_expires_at.as_deref(), &now)
        else {
            continue;
        };
        let risk_level = command_recovery_risk_level(&detail.command_name);
        items.push(CommandRecoveryQueueItem {
            ledger: CommandLedgerListItem {
                id: detail.id,
                command_name: detail.command_name.clone(),
                status: detail.status.clone(),
                attention_reason: Some(status.clone()),
                source: detail.source.clone(),
                primary_aggregate_key: detail.primary_aggregate_key.clone(),
                summary: detail.summary.clone(),
                error_code: detail.error_code.clone(),
                retryable: detail.retryable,
                created_at: detail.created_at.clone(),
                updated_at: detail.updated_at.clone(),
                last_attempt_at: detail.last_attempt_at.clone(),
                completed_at: detail.completed_at.clone(),
                lease_expires_at: detail.lease_expires_at.clone(),
            },
            recovery_status: status,
            requires_confirmation: requires_confirmation(&detail.command_name),
            risk_level,
            allowed_actions: allowed_actions_for_queue_item(),
        });
    }
    Ok(items)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandRecoveryActionRecord {
    pub id: i64,
    pub action: String,
    pub operator_id: Option<String>,
    pub operator_role: Option<String>,
    pub reason: Option<String>,
    pub confirmed: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandRecoveryActionRequest {
    pub command_idempotency_id: i64,
    pub confirmed: Option<bool>,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryActionResponse {
    pub command_idempotency_id: i64,
    pub action: String,
    pub status: String,
    pub code: Option<String>,
    pub message: String,
    pub next_step: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryOperator {
    pub id: String,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandRecoveryDetail {
    pub ledger: CommandLedgerDetail,
    pub recovery_status: Option<String>,
    pub risk_level: RecoveryRiskLevel,
    pub requires_confirmation: bool,
    pub allowed_actions: Vec<String>,
    pub actions: Vec<CommandRecoveryActionRecord>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandRecoveryStartupScan {
    pub expired_in_progress: i64,
    pub failed_retryable: i64,
}

async fn recovery_actions(
    pool: &Pool<Sqlite>,
    command_idempotency_id: i64,
) -> CommandResult<Vec<CommandRecoveryActionRecord>> {
    let rows = sqlx::query(
        "SELECT id, action, operator_id, operator_role, reason, confirmed, created_at
         FROM command_recovery_actions
         WHERE command_idempotency_id = ?
         ORDER BY created_at DESC, id DESC",
    )
    .bind(command_idempotency_id)
    .fetch_all(pool)
    .await
    .map_err(system_error)?;

    rows.into_iter()
        .map(|row| {
            Ok(CommandRecoveryActionRecord {
                id: row.try_get("id").map_err(system_error)?,
                action: row.try_get("action").map_err(system_error)?,
                operator_id: row.try_get("operator_id").map_err(system_error)?,
                operator_role: row.try_get("operator_role").map_err(system_error)?,
                reason: row.try_get("reason").map_err(system_error)?,
                confirmed: row.try_get::<i64, _>("confirmed").map_err(system_error)? != 0,
                created_at: row.try_get("created_at").map_err(system_error)?,
            })
        })
        .collect()
}

pub async fn inspect_command_recovery(
    pool: &Pool<Sqlite>,
    id: i64,
) -> CommandResult<CommandRecoveryDetail> {
    let now = chrono::Utc::now().to_rfc3339();
    let detail = get_command_ledger_detail(pool, id).await?;
    let recovery_status = recovery_status(&detail.status, detail.lease_expires_at.as_deref(), &now);
    let risk_level = command_recovery_risk_level(&detail.command_name);
    let actions = recovery_actions(pool, id).await?;
    Ok(CommandRecoveryDetail {
        ledger: detail,
        recovery_status,
        requires_confirmation: risk_level == RecoveryRiskLevel::High,
        risk_level,
        allowed_actions: allowed_actions_for_queue_item(),
        actions,
    })
}

pub async fn scan_command_recovery_startup(
    pool: &Pool<Sqlite>,
) -> CommandResult<CommandRecoveryStartupScan> {
    let now = chrono::Utc::now().to_rfc3339();
    let expired_in_progress: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM command_idempotency
         WHERE status = 'in_progress'
           AND lease_expires_at IS NOT NULL
           AND lease_expires_at <= ?
           AND recovery_dismissed_at IS NULL",
    )
    .bind(&now)
    .fetch_one(pool)
    .await
    .map_err(system_error)?;

    let failed_retryable: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM command_idempotency
         WHERE status = 'failed_retryable'
           AND recovery_dismissed_at IS NULL",
    )
    .fetch_one(pool)
    .await
    .map_err(system_error)?;

    Ok(CommandRecoveryStartupScan {
        expired_in_progress,
        failed_retryable,
    })
}

#[derive(Debug)]
struct EligibleRecoveryRow {
    command_name: String,
}

fn invalid_recovery_state_error() -> CommandError {
    CommandError::user(
        codes::CONFLICT_INVALID_STATE_TRANSITION,
        codes::CONFLICT_INVALID_STATE_TRANSITION_DEFAULT_MESSAGE,
    )
}

fn approval_required_error() -> CommandError {
    CommandError::user(
        codes::APPROVAL_REQUIRED,
        "High-risk recovery actions require confirmation and a reason.",
    )
}

fn normalized_reason(request: &CommandRecoveryActionRequest) -> Option<String> {
    request
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .map(str::to_string)
}

fn validate_confirmation(
    command_name: &str,
    request: &CommandRecoveryActionRequest,
    reason: Option<&str>,
) -> CommandResult<()> {
    if requires_confirmation(command_name) && (request.confirmed != Some(true) || reason.is_none())
    {
        return Err(approval_required_error());
    }
    Ok(())
}

async fn eligible_recovery_row_tx(
    tx: &mut Transaction<'_, Sqlite>,
    id: i64,
    now: &str,
) -> CommandResult<EligibleRecoveryRow> {
    let row = sqlx::query(
        "SELECT command_name
         FROM command_idempotency
         WHERE id = ?
           AND recovery_dismissed_at IS NULL
           AND (
                status = 'failed_retryable'
                OR (status = 'in_progress' AND lease_expires_at IS NOT NULL AND lease_expires_at <= ?)
           )",
    )
    .bind(id)
    .bind(now)
    .fetch_optional(&mut **tx)
    .await
    .map_err(system_error)?;

    let Some(row) = row else {
        return Err(invalid_recovery_state_error());
    };

    Ok(EligibleRecoveryRow {
        command_name: row.try_get("command_name").map_err(system_error)?,
    })
}

async fn insert_recovery_action_tx(
    tx: &mut Transaction<'_, Sqlite>,
    command_idempotency_id: i64,
    action: &str,
    operator: &RecoveryOperator,
    reason: Option<&str>,
    confirmed: bool,
    now: &str,
) -> CommandResult<()> {
    sqlx::query(
        "INSERT INTO command_recovery_actions (
            command_idempotency_id, action, operator_id, operator_role,
            reason, confirmed, created_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(command_idempotency_id)
    .bind(action)
    .bind(&operator.id)
    .bind(&operator.role)
    .bind(reason)
    .bind(if confirmed { 1_i64 } else { 0_i64 })
    .bind(now)
    .execute(&mut **tx)
    .await
    .map_err(system_error)?;

    Ok(())
}

fn recovery_action_response(
    command_idempotency_id: i64,
    action: &str,
    status: &str,
    code: Option<&str>,
    message: &str,
    next_step: Option<&str>,
) -> RecoveryActionResponse {
    RecoveryActionResponse {
        command_idempotency_id,
        action: action.to_string(),
        status: status.to_string(),
        code: code.map(str::to_string),
        message: message.to_string(),
        next_step: next_step.map(str::to_string),
    }
}

fn recovery_required_error_json() -> CommandResult<(String, String)> {
    let error = CommandError::user(
        codes::RECOVERY_REQUIRED,
        "Command recovery was marked terminal by an operator.",
    );
    let error_json = serde_json::to_string(&error).map_err(system_error)?;
    let error_summary_json = serde_json::to_string(&serde_json::json!({
        "code": codes::RECOVERY_REQUIRED,
        "kind": "user",
        "retryable": false,
        "message": error.message,
        "support_id": null,
    }))
    .map_err(system_error)?;

    Ok((error_json, error_summary_json))
}

pub async fn request_command_recovery_retry(
    pool: &Pool<Sqlite>,
    operator: RecoveryOperator,
    request: CommandRecoveryActionRequest,
) -> CommandResult<RecoveryActionResponse> {
    let now = chrono::Utc::now().to_rfc3339();
    let reason = normalized_reason(&request);
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(system_error)?;

    let result = async {
        let row = eligible_recovery_row_tx(&mut tx, request.command_idempotency_id, &now).await?;
        validate_confirmation(&row.command_name, &request, reason.as_deref())?;
        insert_recovery_action_tx(
            &mut tx,
            request.command_idempotency_id,
            "retry_requested",
            &operator,
            reason.as_deref(),
            request.confirmed.unwrap_or(false),
            &now,
        )
        .await?;

        Ok(recovery_action_response(
            request.command_idempotency_id,
            "retry_requested",
            "recovery_required",
            Some(codes::RECOVERY_REQUIRED),
            "Recovery retry was requested; CapyInn will not replay the business command automatically.",
            Some(
                "Retry the original business command through the command boundary with the original valid payload.",
            ),
        ))
    }
    .await;

    match result {
        Ok(response) => {
            tx.commit().await.map_err(system_error)?;
            Ok(response)
        }
        Err(error) => {
            tx.rollback().await.map_err(system_error)?;
            Err(error)
        }
    }
}

pub async fn dismiss_command_recovery(
    pool: &Pool<Sqlite>,
    operator: RecoveryOperator,
    request: CommandRecoveryActionRequest,
) -> CommandResult<RecoveryActionResponse> {
    let now = chrono::Utc::now().to_rfc3339();
    let reason = normalized_reason(&request);
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(system_error)?;

    let result = async {
        let row = eligible_recovery_row_tx(&mut tx, request.command_idempotency_id, &now).await?;
        validate_confirmation(&row.command_name, &request, reason.as_deref())?;
        insert_recovery_action_tx(
            &mut tx,
            request.command_idempotency_id,
            "dismissed",
            &operator,
            reason.as_deref(),
            request.confirmed.unwrap_or(false),
            &now,
        )
        .await?;

        let update_result = sqlx::query(
            "UPDATE command_idempotency
             SET recovery_dismissed_at = ?,
                 recovery_dismissed_by = ?,
                 updated_at = ?
             WHERE id = ?
               AND recovery_dismissed_at IS NULL
               AND (
                    status = 'failed_retryable'
                    OR (status = 'in_progress' AND lease_expires_at IS NOT NULL AND lease_expires_at <= ?)
               )",
        )
        .bind(&now)
        .bind(&operator.id)
        .bind(&now)
        .bind(request.command_idempotency_id)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(system_error)?;

        if update_result.rows_affected() != 1 {
            return Err(invalid_recovery_state_error());
        }

        Ok(recovery_action_response(
            request.command_idempotency_id,
            "dismissed",
            "dismissed",
            None,
            "Recovery item dismissed from the active queue.",
            None,
        ))
    }
    .await;

    match result {
        Ok(response) => {
            tx.commit().await.map_err(system_error)?;
            Ok(response)
        }
        Err(error) => {
            tx.rollback().await.map_err(system_error)?;
            Err(error)
        }
    }
}

pub async fn mark_command_recovery_terminal(
    pool: &Pool<Sqlite>,
    operator: RecoveryOperator,
    request: CommandRecoveryActionRequest,
) -> CommandResult<RecoveryActionResponse> {
    let now = chrono::Utc::now().to_rfc3339();
    let reason = normalized_reason(&request);
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(system_error)?;

    let result = async {
        let row = eligible_recovery_row_tx(&mut tx, request.command_idempotency_id, &now).await?;
        validate_confirmation(&row.command_name, &request, reason.as_deref())?;
        insert_recovery_action_tx(
            &mut tx,
            request.command_idempotency_id,
            "marked_terminal",
            &operator,
            reason.as_deref(),
            request.confirmed.unwrap_or(false),
            &now,
        )
        .await?;

        let (error_json, error_summary_json) = recovery_required_error_json()?;
        let update_result = sqlx::query(
            "UPDATE command_idempotency
             SET status = 'failed_terminal',
                 response_json = NULL,
                 result_summary_json = NULL,
                 error_code = ?,
                 error_json = ?,
                 error_summary_json = ?,
                 retryable = 0,
                 lease_expires_at = NULL,
                 updated_at = ?,
                 completed_at = ?,
                 recovery_dismissed_at = NULL,
                 recovery_dismissed_by = NULL
             WHERE id = ?
               AND recovery_dismissed_at IS NULL
               AND (
                    status = 'failed_retryable'
                    OR (status = 'in_progress' AND lease_expires_at IS NOT NULL AND lease_expires_at <= ?)
               )",
        )
        .bind(codes::RECOVERY_REQUIRED)
        .bind(&error_json)
        .bind(&error_summary_json)
        .bind(&now)
        .bind(&now)
        .bind(request.command_idempotency_id)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(system_error)?;

        if update_result.rows_affected() != 1 {
            return Err(invalid_recovery_state_error());
        }

        Ok(recovery_action_response(
            request.command_idempotency_id,
            "marked_terminal",
            "failed_terminal",
            Some(codes::RECOVERY_REQUIRED),
            "Command marked terminal with recovery-required error details.",
            None,
        ))
    }
    .await;

    match result {
        Ok(response) => {
            tx.commit().await.map_err(system_error)?;
            Ok(response)
        }
        Err(error) => {
            tx.rollback().await.map_err(system_error)?;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
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
            .expect("opens sqlite test pool");
        crate::db::run_migrations(&pool)
            .await
            .expect("runs migrations");
        pool
    }

    async fn seed_recovery_row(
        pool: &Pool<Sqlite>,
        command_name: &str,
        status: &str,
        lease_expires_at: Option<String>,
        dismissed: bool,
    ) -> i64 {
        let now = Utc::now().to_rfc3339();
        let dismissed_at = if dismissed { Some(now.clone()) } else { None };
        let result = sqlx::query(
            "INSERT INTO command_idempotency (
                idempotency_key, command_name, request_hash, intent_json,
                primary_aggregate_key, lock_keys_json, status, claim_token,
                response_json, error_code, error_json, retryable, lease_expires_at,
                created_at, updated_at, completed_at, last_attempt_at, request_id,
                actor_type, actor_id, issued_at, summary_json, result_summary_json,
                error_summary_json, recovery_dismissed_at, recovery_dismissed_by
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, ?)",
        )
        .bind(format!("{command_name}-{status}-{}", uuid::Uuid::new_v4()))
        .bind(command_name)
        .bind("hash")
        .bind(r#"{"fields":{"safe":"value"}}"#)
        .bind("booking:123")
        .bind(r#"["booking:123"]"#)
        .bind(status)
        .bind("claim-token")
        .bind(if status.starts_with("failed") { Some("DB_LOCKED_RETRYABLE") } else { None })
        .bind(if status.starts_with("failed") { Some(r#"{"code":"DB_LOCKED_RETRYABLE","message":"locked","kind":"system","support_id":"SUP-TEST","retryable":true,"request_id":null}"#) } else { None })
        .bind(if status == "failed_retryable" { 1_i64 } else { 0_i64 })
        .bind(lease_expires_at)
        .bind(&now)
        .bind(&now)
        .bind(if status == "failed_terminal" || status == "completed" { Some(now.clone()) } else { None })
        .bind(&now)
        .bind("req-recovery")
        .bind("human")
        .bind("admin-1")
        .bind(&now)
        .bind(r#"{"label":"Booking #123","aggregate_refs":[{"type":"booking","id":"123"}],"business_dates":[],"safe_fields":{}}"#)
        .bind(if status.starts_with("failed") { Some(r#"{"code":"DB_LOCKED_RETRYABLE","kind":"system","retryable":true,"message":"locked","support_id":"SUP-TEST"}"#) } else { None })
        .bind(dismissed_at)
        .bind(if dismissed { Some("admin-1") } else { None })
        .execute(pool)
        .await
        .expect("seeds recovery row");
        result.last_insert_rowid()
    }

    fn recovery_operator() -> RecoveryOperator {
        RecoveryOperator {
            id: "ops-1".to_string(),
            role: "admin".to_string(),
        }
    }

    fn recovery_request(
        command_idempotency_id: i64,
        confirmed: Option<bool>,
        reason: Option<&str>,
    ) -> CommandRecoveryActionRequest {
        CommandRecoveryActionRequest {
            command_idempotency_id,
            confirmed,
            reason: reason.map(str::to_string),
        }
    }

    async fn action_count(pool: &Pool<Sqlite>, command_idempotency_id: i64) -> i64 {
        sqlx::query_scalar(
            "SELECT COUNT(*) FROM command_recovery_actions WHERE command_idempotency_id = ?",
        )
        .bind(command_idempotency_id)
        .fetch_one(pool)
        .await
        .expect("counts recovery actions")
    }

    async fn action_field(pool: &Pool<Sqlite>, command_idempotency_id: i64, field: &str) -> String {
        let sql = format!(
            "SELECT {field} FROM command_recovery_actions WHERE command_idempotency_id = ? ORDER BY id DESC LIMIT 1"
        );
        sqlx::query_scalar(&sql)
            .bind(command_idempotency_id)
            .fetch_one(pool)
            .await
            .expect("reads recovery action field")
    }

    #[tokio::test]
    async fn retry_request_audits_and_returns_recovery_required_without_status_change() {
        let pool = test_pool().await;
        let id = seed_recovery_row(&pool, "check_out", "failed_retryable", None, false).await;

        let response = request_command_recovery_retry(
            &pool,
            recovery_operator(),
            recovery_request(id, Some(true), Some("operator reviewed retry")),
        )
        .await
        .expect("requests retry");

        assert_eq!(response.command_idempotency_id, id);
        assert_eq!(response.action, "retry_requested");
        assert_eq!(response.status, "recovery_required");
        assert_eq!(response.code.as_deref(), Some(codes::RECOVERY_REQUIRED));
        assert!(response
            .next_step
            .as_deref()
            .is_some_and(|step| step.contains("command boundary")));
        assert_eq!(
            sqlx::query_scalar::<_, String>("SELECT status FROM command_idempotency WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .expect("reads status"),
            "failed_retryable"
        );
        assert_eq!(action_count(&pool, id).await, 1);
        assert_eq!(action_field(&pool, id, "action").await, "retry_requested");
    }

    #[tokio::test]
    async fn dismiss_audits_sets_marker_and_leaves_status_unchanged() {
        let pool = test_pool().await;
        let id = seed_recovery_row(&pool, "refresh_cache", "failed_retryable", None, false).await;

        let response = dismiss_command_recovery(
            &pool,
            recovery_operator(),
            recovery_request(id, None, Some("handled manually")),
        )
        .await
        .expect("dismisses recovery row");

        assert_eq!(response.action, "dismissed");
        assert_eq!(response.status, "dismissed");
        assert!(response.code.is_none());
        assert!(response.next_step.is_none());
        let row = sqlx::query(
            "SELECT status, recovery_dismissed_at, recovery_dismissed_by
             FROM command_idempotency WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .expect("reads recovery marker");
        assert_eq!(row.get::<String, _>("status"), "failed_retryable");
        assert!(row
            .get::<Option<String>, _>("recovery_dismissed_at")
            .is_some());
        assert_eq!(
            row.get::<Option<String>, _>("recovery_dismissed_by")
                .as_deref(),
            Some("ops-1")
        );
        assert_eq!(action_count(&pool, id).await, 1);
        assert_eq!(action_field(&pool, id, "action").await, "dismissed");
    }

    #[tokio::test]
    async fn high_risk_dismiss_requires_confirmation_and_reason() {
        let pool = test_pool().await;
        let id = seed_recovery_row(&pool, "check_out", "failed_retryable", None, false).await;

        let missing_confirmation = dismiss_command_recovery(
            &pool,
            recovery_operator(),
            recovery_request(id, Some(false), Some("reviewed")),
        )
        .await
        .expect_err("requires confirmation");
        assert_eq!(missing_confirmation.code, codes::APPROVAL_REQUIRED);

        let missing_reason = dismiss_command_recovery(
            &pool,
            recovery_operator(),
            recovery_request(id, Some(true), Some("   ")),
        )
        .await
        .expect_err("requires reason");
        assert_eq!(missing_reason.code, codes::APPROVAL_REQUIRED);
        assert_eq!(action_count(&pool, id).await, 0);
    }

    #[tokio::test]
    async fn high_risk_mark_terminal_requires_confirmation_and_reason() {
        let pool = test_pool().await;
        let id = seed_recovery_row(&pool, "check_out", "failed_retryable", None, false).await;

        let missing_confirmation = mark_command_recovery_terminal(
            &pool,
            recovery_operator(),
            recovery_request(id, Some(false), Some("reviewed")),
        )
        .await
        .expect_err("requires confirmation");
        assert_eq!(missing_confirmation.code, codes::APPROVAL_REQUIRED);

        let missing_reason = mark_command_recovery_terminal(
            &pool,
            recovery_operator(),
            recovery_request(id, Some(true), Some("   ")),
        )
        .await
        .expect_err("requires reason");
        assert_eq!(missing_reason.code, codes::APPROVAL_REQUIRED);
        assert_eq!(action_count(&pool, id).await, 0);
    }

    #[tokio::test]
    async fn high_risk_retry_request_requires_confirmation_and_reason() {
        let pool = test_pool().await;
        let id = seed_recovery_row(&pool, "record_payment", "failed_retryable", None, false).await;

        let missing_confirmation = request_command_recovery_retry(
            &pool,
            recovery_operator(),
            recovery_request(id, Some(false), Some("reviewed")),
        )
        .await
        .expect_err("requires confirmation");
        assert_eq!(missing_confirmation.code, codes::APPROVAL_REQUIRED);

        let missing_reason = request_command_recovery_retry(
            &pool,
            recovery_operator(),
            recovery_request(id, Some(true), None),
        )
        .await
        .expect_err("requires reason");
        assert_eq!(missing_reason.code, codes::APPROVAL_REQUIRED);
        assert_eq!(action_count(&pool, id).await, 0);
    }

    #[tokio::test]
    async fn mark_terminal_closes_row_with_recovery_required_error() {
        let pool = test_pool().await;
        let id = seed_recovery_row(&pool, "check_out", "failed_retryable", None, false).await;

        let response = mark_command_recovery_terminal(
            &pool,
            recovery_operator(),
            recovery_request(id, Some(true), Some("cannot safely retry")),
        )
        .await
        .expect("marks terminal");

        assert_eq!(response.action, "marked_terminal");
        assert_eq!(response.status, "failed_terminal");
        let row = sqlx::query(
            "SELECT status, retryable, lease_expires_at, completed_at, error_code, error_json,
                    error_summary_json, recovery_dismissed_at, recovery_dismissed_by
             FROM command_idempotency WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .expect("reads terminal row");

        assert_eq!(row.get::<String, _>("status"), "failed_terminal");
        assert_eq!(row.get::<i64, _>("retryable"), 0);
        assert!(row.get::<Option<String>, _>("lease_expires_at").is_none());
        assert!(row.get::<Option<String>, _>("completed_at").is_some());
        assert_eq!(
            row.get::<Option<String>, _>("error_code").as_deref(),
            Some(codes::RECOVERY_REQUIRED)
        );
        assert!(row
            .get::<Option<String>, _>("error_json")
            .expect("stores error json")
            .contains(codes::RECOVERY_REQUIRED));
        assert!(row
            .get::<Option<String>, _>("error_summary_json")
            .expect("stores error summary")
            .contains(codes::RECOVERY_REQUIRED));
        assert!(row
            .get::<Option<String>, _>("recovery_dismissed_at")
            .is_none());
        assert!(row
            .get::<Option<String>, _>("recovery_dismissed_by")
            .is_none());
        assert_eq!(action_count(&pool, id).await, 1);
    }

    #[tokio::test]
    async fn low_risk_mark_terminal_does_not_require_confirmation() {
        let pool = test_pool().await;
        let id = seed_recovery_row(&pool, "refresh_cache", "failed_retryable", None, false).await;

        let response = mark_command_recovery_terminal(
            &pool,
            recovery_operator(),
            recovery_request(id, None, None),
        )
        .await
        .expect("marks low risk command terminal without confirmation");

        assert_eq!(response.status, "failed_terminal");
        assert_eq!(action_field(&pool, id, "action").await, "marked_terminal");
    }

    #[tokio::test]
    async fn dismiss_rejects_stale_or_already_dismissed_row() {
        let pool = test_pool().await;
        let live = (Utc::now() + chrono::Duration::seconds(60)).to_rfc3339();
        let already_dismissed =
            seed_recovery_row(&pool, "check_out", "failed_retryable", None, true).await;
        let live_in_progress =
            seed_recovery_row(&pool, "check_out", "in_progress", Some(live), false).await;

        let dismissed_error = dismiss_command_recovery(
            &pool,
            recovery_operator(),
            recovery_request(already_dismissed, None, None),
        )
        .await
        .expect_err("already dismissed is stale");
        assert_eq!(
            dismissed_error.code,
            codes::CONFLICT_INVALID_STATE_TRANSITION
        );

        let live_error = dismiss_command_recovery(
            &pool,
            recovery_operator(),
            recovery_request(live_in_progress, None, None),
        )
        .await
        .expect_err("live in-progress row is not eligible");
        assert_eq!(live_error.code, codes::CONFLICT_INVALID_STATE_TRANSITION);
        assert_eq!(action_count(&pool, already_dismissed).await, 0);
        assert_eq!(action_count(&pool, live_in_progress).await, 0);
    }

    #[tokio::test]
    async fn recovery_queue_includes_expired_in_progress_and_failed_retryable_only() {
        let pool = test_pool().await;
        let expired = (Utc::now() - chrono::Duration::seconds(5)).to_rfc3339();
        let live = (Utc::now() + chrono::Duration::seconds(60)).to_rfc3339();

        let expired_id =
            seed_recovery_row(&pool, "check_out", "in_progress", Some(expired), false).await;
        let retryable_id =
            seed_recovery_row(&pool, "record_payment", "failed_retryable", None, false).await;
        seed_recovery_row(&pool, "check_in", "in_progress", Some(live), false).await;
        seed_recovery_row(&pool, "check_in", "completed", None, false).await;
        seed_recovery_row(&pool, "check_in", "failed_terminal", None, false).await;
        seed_recovery_row(&pool, "check_in", "failed_retryable", None, true).await;

        let rows = list_command_recovery_queue(&pool)
            .await
            .expect("lists recovery queue");
        let ids = rows.iter().map(|row| row.ledger.id).collect::<Vec<_>>();

        assert_eq!(rows.len(), 2);
        assert!(ids.contains(&expired_id));
        assert!(ids.contains(&retryable_id));
        assert!(rows
            .iter()
            .all(|row| row.allowed_actions.contains(&"inspect".to_string())));
        assert!(rows.iter().all(|row| row.requires_confirmation));
    }

    #[tokio::test]
    async fn inspect_recovery_detail_returns_actions_without_writing_an_action() {
        let pool = test_pool().await;
        let id = seed_recovery_row(&pool, "check_out", "failed_retryable", None, false).await;
        sqlx::query(
            "INSERT INTO command_recovery_actions (
                command_idempotency_id, action, operator_id, operator_role,
                reason, confirmed, created_at
            ) VALUES (?, 'dismissed', 'admin-1', 'admin', 'checked', 0, ?)",
        )
        .bind(id)
        .bind(Utc::now().to_rfc3339())
        .execute(&pool)
        .await
        .expect("seed action");

        let detail = inspect_command_recovery(&pool, id)
            .await
            .expect("inspect recovery detail");

        assert_eq!(detail.ledger.id, id);
        assert_eq!(detail.recovery_status.as_deref(), Some("failed_retryable"));
        assert_eq!(detail.actions.len(), 1);
        assert_eq!(detail.actions[0].action, "dismissed");

        let action_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM command_recovery_actions WHERE command_idempotency_id = ?",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .expect("action count");
        assert_eq!(action_count, 1);
    }

    #[tokio::test]
    async fn startup_scan_counts_recovery_rows_without_mutating() {
        let pool = test_pool().await;
        let expired = (Utc::now() - chrono::Duration::seconds(5)).to_rfc3339();
        let expired_id =
            seed_recovery_row(&pool, "check_out", "in_progress", Some(expired), false).await;
        seed_recovery_row(&pool, "record_payment", "failed_retryable", None, false).await;

        let before_updated_at: String =
            sqlx::query_scalar("SELECT updated_at FROM command_idempotency WHERE id = ?")
                .bind(expired_id)
                .fetch_one(&pool)
                .await
                .expect("before updated_at");

        let scan = scan_command_recovery_startup(&pool)
            .await
            .expect("startup scan");

        assert_eq!(scan.expired_in_progress, 1);
        assert_eq!(scan.failed_retryable, 1);

        let after_updated_at: String =
            sqlx::query_scalar("SELECT updated_at FROM command_idempotency WHERE id = ?")
                .bind(expired_id)
                .fetch_one(&pool)
                .await
                .expect("after updated_at");
        assert_eq!(after_updated_at, before_updated_at);
    }
}
