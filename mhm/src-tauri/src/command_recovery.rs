use crate::app_error::{codes, CommandError, CommandResult};
use crate::command_ledger::{
    get_command_ledger_detail, CommandLedgerDetail, CommandLedgerListItem,
};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Row, Sqlite};

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
