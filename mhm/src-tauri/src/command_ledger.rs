use crate::app_error::{codes, CommandError, CommandResult};
use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqliteRow, Pool, QueryBuilder, Row, Sqlite};

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct CommandLedgerListOptions {
    pub attention_only: Option<bool>,
    pub include_completed: Option<bool>,
    pub status: Option<String>,
    pub command_name: Option<String>,
    pub primary_aggregate_key: Option<String>,
    pub attention_reason: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandLedgerSource {
    pub request_id: Option<String>,
    pub actor_type: String,
    pub actor_label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandLedgerListItem {
    pub id: i64,
    pub command_name: String,
    pub status: String,
    pub attention_reason: Option<String>,
    pub source: CommandLedgerSource,
    pub primary_aggregate_key: Option<String>,
    pub summary: serde_json::Value,
    pub error_code: Option<String>,
    pub retryable: bool,
    pub created_at: String,
    pub updated_at: String,
    pub last_attempt_at: Option<String>,
    pub completed_at: Option<String>,
    pub lease_expires_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandLedgerDetail {
    pub id: i64,
    pub command_name: String,
    pub status: String,
    pub attention_reason: Option<String>,
    pub source: CommandLedgerSource,
    pub primary_aggregate_key: Option<String>,
    pub conflict_refs: Vec<serde_json::Value>,
    pub ledger_intent: serde_json::Value,
    pub summary: serde_json::Value,
    pub result_summary: Option<serde_json::Value>,
    pub error_summary: Option<serde_json::Value>,
    pub error_code: Option<String>,
    pub retryable: bool,
    pub created_at: String,
    pub updated_at: String,
    pub last_attempt_at: Option<String>,
    pub completed_at: Option<String>,
    pub lease_expires_at: Option<String>,
}

pub async fn list_command_ledger(
    pool: &Pool<Sqlite>,
    options: CommandLedgerListOptions,
) -> CommandResult<Vec<CommandLedgerListItem>> {
    let now = chrono::Utc::now().to_rfc3339();
    let include_completed = options.include_completed.unwrap_or(false);
    let explicit_completed_request =
        include_completed || options.status.as_deref() == Some("completed");
    let attention_only = options
        .attention_only
        .unwrap_or(!explicit_completed_request);
    let limit = bounded_limit(options.limit);
    let offset = bounded_offset(options.offset);

    validate_status_filter(options.status.as_deref())?;
    validate_attention_reason_filter(options.attention_reason.as_deref())?;

    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT id, command_name, status, request_id, actor_type, actor_id,
                primary_aggregate_key, summary_json, error_code, retryable,
                created_at, updated_at, last_attempt_at, completed_at, lease_expires_at
         FROM command_idempotency WHERE 1 = 1",
    );

    if attention_only {
        builder.push(
            " AND (
                (status = 'in_progress' AND lease_expires_at IS NOT NULL AND lease_expires_at <= ",
        );
        builder.push_bind(now.clone());
        builder.push(") OR status IN ('failed_retryable', 'failed_terminal'))");
    } else if !explicit_completed_request {
        builder.push(" AND status != 'completed'");
    }

    if let Some(status) = options.status.as_deref() {
        builder.push(" AND status = ");
        builder.push_bind(status);
    }

    if let Some(command_name) = options.command_name.as_deref() {
        builder.push(" AND command_name = ");
        builder.push_bind(command_name);
    }

    if let Some(primary_aggregate_key) = options.primary_aggregate_key.as_deref() {
        builder.push(" AND primary_aggregate_key = ");
        builder.push_bind(primary_aggregate_key);
    }

    if let Some(reason) = options.attention_reason.as_deref() {
        match reason {
            "expired_in_progress" => {
                builder.push(
                    " AND status = 'in_progress'
                      AND lease_expires_at IS NOT NULL
                      AND lease_expires_at <= ",
                );
                builder.push_bind(now.clone());
            }
            "failed_retryable" => {
                builder.push(" AND status = 'failed_retryable'");
            }
            "failed_terminal" => {
                builder.push(" AND status = 'failed_terminal'");
            }
            _ => unreachable!("attention reason filter is pre-validated"),
        };
    }

    builder.push(
        " ORDER BY
            CASE
                WHEN status = 'in_progress'
                     AND lease_expires_at IS NOT NULL
                     AND lease_expires_at <= ",
    );
    builder.push_bind(now.clone());
    builder.push(
        " THEN 0
                WHEN status = 'failed_retryable' THEN 1
                WHEN status = 'failed_terminal' THEN 2
                WHEN status = 'in_progress' THEN 3
                ELSE 4
            END,
            updated_at DESC
          LIMIT ",
    );
    builder.push_bind(limit);
    builder.push(" OFFSET ");
    builder.push_bind(offset);

    let rows = builder
        .build()
        .fetch_all(pool)
        .await
        .map_err(system_error)?;

    rows.into_iter()
        .map(|row| list_item_from_row(row, &now))
        .collect()
}

pub async fn list_command_ledger_attention(
    pool: &Pool<Sqlite>,
) -> CommandResult<Vec<CommandLedgerListItem>> {
    list_command_ledger(
        pool,
        CommandLedgerListOptions {
            attention_only: Some(true),
            include_completed: Some(false),
            ..CommandLedgerListOptions::default()
        },
    )
    .await
}

pub async fn get_command_ledger_detail(
    pool: &Pool<Sqlite>,
    id: i64,
) -> CommandResult<CommandLedgerDetail> {
    let now = chrono::Utc::now().to_rfc3339();
    let row = sqlx::query(
        "SELECT id, command_name, status, request_id, actor_type, actor_id,
                primary_aggregate_key, intent_json, summary_json, result_summary_json,
                error_summary_json, error_code, retryable, created_at, updated_at,
                last_attempt_at, completed_at, lease_expires_at
         FROM command_idempotency
         WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(system_error)?
    .ok_or_else(|| {
        CommandError::user(
            codes::COMMAND_LEDGER_ROW_NOT_FOUND,
            "Command ledger row not found",
        )
    })?;

    detail_from_row(row, &now)
}

fn bounded_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

fn bounded_offset(offset: Option<i64>) -> i64 {
    offset.unwrap_or(0).max(0)
}

fn validate_status_filter(status: Option<&str>) -> CommandResult<()> {
    match status {
        Some("in_progress" | "completed" | "failed_retryable" | "failed_terminal") | None => Ok(()),
        Some(_) => Err(CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            "Invalid command ledger status filter",
        )),
    }
}

fn validate_attention_reason_filter(reason: Option<&str>) -> CommandResult<()> {
    match reason {
        Some("expired_in_progress" | "failed_retryable" | "failed_terminal") | None => Ok(()),
        Some(_) => Err(CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            "Invalid command ledger attention reason filter",
        )),
    }
}

fn attention_reason(status: &str, lease_expires_at: Option<&str>, now: &str) -> Option<String> {
    match status {
        "in_progress" if lease_expires_at.is_some_and(|lease| lease <= now) => {
            Some("expired_in_progress".to_string())
        }
        "failed_retryable" => Some("failed_retryable".to_string()),
        "failed_terminal" => Some("failed_terminal".to_string()),
        _ => None,
    }
}

fn parse_json(raw: Option<String>) -> serde_json::Value {
    raw.and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_else(|| serde_json::json!({}))
}

fn parse_optional_json(raw: Option<String>) -> Option<serde_json::Value> {
    raw.and_then(|value| serde_json::from_str(&value).ok())
}

fn source_from_row(row: &SqliteRow) -> CommandLedgerSource {
    let actor_label = row
        .try_get::<Option<String>, _>("actor_id")
        .ok()
        .flatten()
        .filter(|value| !value.is_empty());

    CommandLedgerSource {
        request_id: row.try_get("request_id").ok().flatten(),
        actor_type: row
            .try_get::<String, _>("actor_type")
            .unwrap_or_else(|_| "system".to_string()),
        actor_label,
    }
}

fn conflict_refs_from_summary(summary: &serde_json::Value) -> Vec<serde_json::Value> {
    summary
        .get("aggregate_refs")
        .and_then(|refs| refs.as_array())
        .cloned()
        .unwrap_or_default()
}

fn list_item_from_row(row: SqliteRow, now: &str) -> CommandResult<CommandLedgerListItem> {
    let status: String = row.try_get("status").map_err(system_error)?;
    let lease_expires_at: Option<String> = row.try_get("lease_expires_at").map_err(system_error)?;

    Ok(CommandLedgerListItem {
        id: row.try_get("id").map_err(system_error)?,
        command_name: row.try_get("command_name").map_err(system_error)?,
        status: status.clone(),
        attention_reason: attention_reason(&status, lease_expires_at.as_deref(), now),
        source: source_from_row(&row),
        primary_aggregate_key: row.try_get("primary_aggregate_key").map_err(system_error)?,
        summary: parse_json(row.try_get("summary_json").map_err(system_error)?),
        error_code: row.try_get("error_code").map_err(system_error)?,
        retryable: row.try_get::<i64, _>("retryable").map_err(system_error)? != 0,
        created_at: row.try_get("created_at").map_err(system_error)?,
        updated_at: row.try_get("updated_at").map_err(system_error)?,
        last_attempt_at: row.try_get("last_attempt_at").map_err(system_error)?,
        completed_at: row.try_get("completed_at").map_err(system_error)?,
        lease_expires_at,
    })
}

fn detail_from_row(row: SqliteRow, now: &str) -> CommandResult<CommandLedgerDetail> {
    let status: String = row.try_get("status").map_err(system_error)?;
    let lease_expires_at: Option<String> = row.try_get("lease_expires_at").map_err(system_error)?;
    let summary = parse_json(row.try_get("summary_json").map_err(system_error)?);

    Ok(CommandLedgerDetail {
        id: row.try_get("id").map_err(system_error)?,
        command_name: row.try_get("command_name").map_err(system_error)?,
        status: status.clone(),
        attention_reason: attention_reason(&status, lease_expires_at.as_deref(), now),
        source: source_from_row(&row),
        primary_aggregate_key: row.try_get("primary_aggregate_key").map_err(system_error)?,
        conflict_refs: conflict_refs_from_summary(&summary),
        ledger_intent: parse_json(row.try_get("intent_json").map_err(system_error)?),
        summary,
        result_summary: parse_optional_json(
            row.try_get("result_summary_json").map_err(system_error)?,
        ),
        error_summary: parse_optional_json(
            row.try_get("error_summary_json").map_err(system_error)?,
        ),
        error_code: row.try_get("error_code").map_err(system_error)?,
        retryable: row.try_get::<i64, _>("retryable").map_err(system_error)? != 0,
        created_at: row.try_get("created_at").map_err(system_error)?,
        updated_at: row.try_get("updated_at").map_err(system_error)?,
        last_attempt_at: row.try_get("last_attempt_at").map_err(system_error)?,
        completed_at: row.try_get("completed_at").map_err(system_error)?,
        lease_expires_at,
    })
}

fn system_error(error: impl std::fmt::Display) -> CommandError {
    CommandError::system(codes::SYSTEM_INTERNAL_ERROR, error.to_string())
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

    async fn seed_row(
        pool: &Pool<Sqlite>,
        status: &str,
        lease_expires_at: Option<&str>,
        completed_at: Option<&str>,
    ) -> i64 {
        seed_row_for(
            pool,
            status,
            lease_expires_at,
            completed_at,
            "test.command",
            "booking:123",
        )
        .await
    }

    async fn seed_row_for(
        pool: &Pool<Sqlite>,
        status: &str,
        lease_expires_at: Option<&str>,
        completed_at: Option<&str>,
        command_name: &str,
        primary_aggregate_key: &str,
    ) -> i64 {
        let now = Utc::now().to_rfc3339();
        let insert_sql = format!(
            "INSERT INTO command_idempotency (
                idempotency_key, command_name, {}, intent_json,
                primary_aggregate_key, {}, status, {},
                {}, error_code, {}, retryable, lease_expires_at,
                created_at, updated_at, completed_at, last_attempt_at, request_id,
                actor_type, actor_id, {}, {}, {}, issued_at,
                summary_json, result_summary_json, error_summary_json
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            ["request", "_hash"].concat(),
            ["lock", "_keys", "_json"].concat(),
            ["claim", "_token"].concat(),
            ["response", "_json"].concat(),
            ["error", "_json"].concat(),
            ["client", "_id"].concat(),
            ["session", "_id"].concat(),
            ["channel", "_id"].concat(),
        );
        let result = sqlx::query(&insert_sql)
            .bind(format!("idem-{status}-{}", uuid::Uuid::new_v4()))
            .bind(command_name)
            .bind("opaque-hash-value")
            .bind(r#"{"fields":{"booking_id":"123"}}"#)
            .bind(primary_aggregate_key)
            .bind(format!(r#"["{primary_aggregate_key}"]"#))
            .bind(status)
            .bind("opaque-claim-value")
            .bind(if status == "completed" {
                Some(r#"{"ok":true,"raw_value":"should-not-surface"}"#)
            } else {
                None
            })
            .bind(if status.starts_with("failed") {
                Some("DB_LOCKED_RETRYABLE")
            } else {
                None
            })
            .bind(if status.starts_with("failed") {
                Some(r#"{"message":"raw should not surface"}"#)
            } else {
                None
            })
            .bind(if status == "failed_retryable" {
                1_i64
            } else {
                0_i64
            })
            .bind(lease_expires_at)
            .bind(&now)
            .bind(&now)
            .bind(completed_at)
            .bind(&now)
            .bind("req-1")
            .bind("human")
            .bind("admin-1")
            .bind("secret-client-value")
            .bind("secret-session-value")
            .bind("secret-channel-value")
            .bind(&now)
            .bind(r#"{"label":"Booking #123","aggregate_refs":[{"type":"booking","id":"123","label":"Booking #123"}],"business_dates":[],"safe_fields":{"room_label":"205"}}"#)
            .bind(if status == "completed" {
                Some(r#"{"label":"Command completed"}"#)
            } else {
                None
            })
            .bind(if status.starts_with("failed") {
                Some(r#"{"code":"DB_LOCKED_RETRYABLE","kind":"system","retryable":true,"message":"locked","support_id":"SUP-TEST"}"#)
            } else {
                None
            })
            .execute(pool)
            .await
            .expect("seeds command row");
        result.last_insert_rowid()
    }

    #[tokio::test]
    async fn attention_list_includes_only_rows_needing_attention() {
        let pool = test_pool().await;
        let expired = (Utc::now() - chrono::Duration::seconds(1)).to_rfc3339();
        let live = (Utc::now() + chrono::Duration::seconds(60)).to_rfc3339();
        seed_row(&pool, "in_progress", Some(&expired), None).await;
        seed_row(&pool, "in_progress", Some(&live), None).await;
        seed_row(&pool, "failed_retryable", None, None).await;
        seed_row(&pool, "failed_terminal", None, Some(&expired)).await;
        seed_row(&pool, "completed", None, Some(&expired)).await;

        let rows = list_command_ledger_attention(&pool)
            .await
            .expect("lists attention rows");
        let reasons: Vec<_> = rows
            .iter()
            .map(|row| row.attention_reason.as_deref())
            .collect();

        assert_eq!(rows.len(), 3);
        assert!(reasons.contains(&Some("expired_in_progress")));
        assert!(reasons.contains(&Some("failed_retryable")));
        assert!(reasons.contains(&Some("failed_terminal")));
        assert!(!rows.iter().any(|row| row.status == "completed"));
    }

    #[tokio::test]
    async fn ledger_list_defaults_to_attention_only() {
        let pool = test_pool().await;
        let expired = (Utc::now() - chrono::Duration::seconds(1)).to_rfc3339();
        let live = (Utc::now() + chrono::Duration::seconds(60)).to_rfc3339();
        seed_row(&pool, "in_progress", Some(&expired), None).await;
        seed_row(&pool, "in_progress", Some(&live), None).await;
        seed_row(&pool, "failed_retryable", None, None).await;
        seed_row(&pool, "completed", None, Some(&expired)).await;

        let rows = list_command_ledger(&pool, CommandLedgerListOptions::default())
            .await
            .expect("lists default rows");

        assert_eq!(rows.len(), 2);
        assert!(rows
            .iter()
            .any(|row| row.attention_reason.as_deref() == Some("expired_in_progress")));
        assert!(rows
            .iter()
            .any(|row| row.attention_reason.as_deref() == Some("failed_retryable")));
        assert!(!rows.iter().any(|row| row.status == "completed"));
        assert!(!rows
            .iter()
            .any(|row| row.status == "in_progress" && row.attention_reason.is_none()));
    }

    #[tokio::test]
    async fn ledger_list_can_include_non_attention_rows_when_requested() {
        let pool = test_pool().await;
        seed_row(&pool, "in_progress", None, None).await;
        seed_row(&pool, "failed_retryable", None, None).await;

        let rows = list_command_ledger(
            &pool,
            CommandLedgerListOptions {
                attention_only: Some(false),
                ..CommandLedgerListOptions::default()
            },
        )
        .await
        .expect("lists non-attention rows");

        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| row.status == "in_progress"));
        assert!(rows.iter().any(|row| row.status == "failed_retryable"));
        assert!(!rows.iter().any(|row| row.status == "completed"));
    }

    #[tokio::test]
    async fn ledger_list_returns_completed_when_requested() {
        let pool = test_pool().await;
        let now = Utc::now().to_rfc3339();
        seed_row(&pool, "completed", None, Some(&now)).await;

        let completed_rows = list_command_ledger(
            &pool,
            CommandLedgerListOptions {
                include_completed: Some(true),
                status: Some("completed".to_string()),
                ..CommandLedgerListOptions::default()
            },
        )
        .await
        .expect("lists completed rows");

        assert_eq!(completed_rows.len(), 1);
    }

    #[tokio::test]
    async fn ledger_list_discovers_completed_by_explicit_status_without_include_flag() {
        let pool = test_pool().await;
        let now = Utc::now().to_rfc3339();
        seed_row(&pool, "completed", None, Some(&now)).await;

        let completed_rows = list_command_ledger(
            &pool,
            CommandLedgerListOptions {
                status: Some("completed".to_string()),
                ..CommandLedgerListOptions::default()
            },
        )
        .await
        .expect("lists completed rows by explicit status");

        assert_eq!(completed_rows.len(), 1);
        assert_eq!(completed_rows[0].status, "completed");
    }

    #[tokio::test]
    async fn ledger_list_filters_by_command_name() {
        let pool = test_pool().await;
        seed_row_for(
            &pool,
            "failed_retryable",
            None,
            None,
            "test.command",
            "booking:123",
        )
        .await;
        seed_row_for(
            &pool,
            "failed_retryable",
            None,
            None,
            "other.command",
            "booking:456",
        )
        .await;

        let rows = list_command_ledger(
            &pool,
            CommandLedgerListOptions {
                command_name: Some("other.command".to_string()),
                ..CommandLedgerListOptions::default()
            },
        )
        .await
        .expect("filters by command name");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].command_name, "other.command");
    }

    #[tokio::test]
    async fn ledger_list_filters_by_primary_aggregate_key() {
        let pool = test_pool().await;
        seed_row_for(
            &pool,
            "failed_retryable",
            None,
            None,
            "test.command",
            "booking:123",
        )
        .await;
        seed_row_for(
            &pool,
            "failed_retryable",
            None,
            None,
            "test.command",
            "booking:456",
        )
        .await;

        let rows = list_command_ledger(
            &pool,
            CommandLedgerListOptions {
                primary_aggregate_key: Some("booking:456".to_string()),
                ..CommandLedgerListOptions::default()
            },
        )
        .await
        .expect("filters by primary aggregate key");

        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].primary_aggregate_key.as_deref(),
            Some("booking:456")
        );
    }

    #[tokio::test]
    async fn ledger_list_enforces_max_limit() {
        let pool = test_pool().await;
        for _ in 0..(MAX_LIMIT + 5) {
            seed_row_for(
                &pool,
                "failed_retryable",
                None,
                None,
                "bulk.command",
                "booking:bulk",
            )
            .await;
        }

        let rows = list_command_ledger(
            &pool,
            CommandLedgerListOptions {
                command_name: Some("bulk.command".to_string()),
                limit: Some(MAX_LIMIT + 100),
                ..CommandLedgerListOptions::default()
            },
        )
        .await
        .expect("clamps oversized limits");

        assert_eq!(rows.len(), MAX_LIMIT as usize);
    }

    #[tokio::test]
    async fn detail_returns_sanitized_fields_without_raw_internals() {
        let pool = test_pool().await;
        let now = Utc::now().to_rfc3339();
        let id = seed_row(&pool, "failed_retryable", None, Some(&now)).await;

        let detail = get_command_ledger_detail(&pool, id)
            .await
            .expect("gets detail");
        let serialized = serde_json::to_string(&detail).expect("serializes detail");

        assert_eq!(detail.id, id);
        assert_eq!(detail.source.request_id.as_deref(), Some("req-1"));
        assert_eq!(detail.source.actor_type, "human");
        assert_eq!(detail.source.actor_label.as_deref(), Some("admin-1"));
        assert_eq!(detail.conflict_refs.len(), 1);
        assert_eq!(detail.ledger_intent["fields"]["booking_id"], "123");
        assert!(serialized.contains("Booking #123"));
        assert!(!serialized.contains("opaque-claim-value"));
        assert!(!serialized.contains("opaque-hash-value"));
        assert!(!serialized.contains("secret-client-value"));
        assert!(!serialized.contains("secret-session-value"));
        assert!(!serialized.contains("secret-channel-value"));
        assert!(!serialized.contains("raw should not surface"));
        for forbidden_field in [
            ["request", "_hash"].concat(),
            ["claim", "_token"].concat(),
            ["response", "_json"].concat(),
            ["error", "_json"].concat(),
            ["client", "_id"].concat(),
            ["session", "_id"].concat(),
            ["channel", "_id"].concat(),
            ["lock", "_keys", "_json"].concat(),
        ] {
            assert!(!serialized.contains(&forbidden_field));
        }
    }

    #[tokio::test]
    async fn missing_detail_returns_not_found_error() {
        let pool = test_pool().await;

        let error = get_command_ledger_detail(&pool, 999)
            .await
            .expect_err("missing row should be a not-found error");

        assert_eq!(error.code, codes::COMMAND_LEDGER_ROW_NOT_FOUND);
    }
}
