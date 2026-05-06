use crate::{
    agent::{
        model::{AgentChannel, AgentProvider, AgentRole, DataSensitivity, MutationRisk},
        retention::{
            AUDIT_METADATA_RETENTION, MEMORY_RETENTION, RAW_PROMPT_RETENTION,
            RAW_PROVIDER_ERROR_RETENTION, RAW_RESPONSE_RETENTION, RAW_TOOL_OUTPUT_RETENTION,
            SESSION_METADATA_RETENTION,
        },
        store::{insert_agent_audit_event_tx, NewAgentAuditEvent},
    },
    app_error::{codes, CommandError, CommandResult},
    command_idempotency::{
        system_error, IdempotentCommandResult, WriteCommandContext, WriteCommandExecutor,
        WriteCommandRequest,
    },
    services::settings_store,
};
use serde_json::Value;
use sqlx::{Pool, Sqlite};

pub const CEO_CLOUD_DATA_OPT_IN_SETTING: &str = "ceo_cloud_data_opt_in";
pub const SET_CEO_CLOUD_DATA_OPT_IN_COMMAND: &str = "agent.set_ceo_cloud_data_opt_in";

pub async fn get_ceo_cloud_data_opt_in(pool: &Pool<Sqlite>) -> CommandResult<bool> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = ? LIMIT 1")
            .bind(CEO_CLOUD_DATA_OPT_IN_SETTING)
            .fetch_optional(pool)
            .await
            .map_err(|error| {
                CommandError::system(
                    codes::SYSTEM_INTERNAL_ERROR,
                    format!("Failed to read ceo_cloud_data_opt_in setting: {error}"),
                )
            })?;

    Ok(value
        .as_deref()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            normalized == "true" || normalized == "1"
        })
        .unwrap_or(false))
}

pub async fn set_ceo_cloud_data_opt_in_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    enabled: bool,
) -> CommandResult<IdempotentCommandResult<Value>> {
    let request = WriteCommandRequest::new_low_risk(
        serde_json::json!({ "enabled": enabled }),
        "Set CEO cloud-data opt-in",
    )?
    .with_primary_aggregate_key(format!("settings:{CEO_CLOUD_DATA_OPT_IN_SETTING}"))
    .with_lock_key_deriver(|_intent| Ok(vec![format!("settings:{CEO_CLOUD_DATA_OPT_IN_SETTING}")]));

    let event_type = if enabled {
        "ceo_cloud_data_opt_in.enabled"
    } else {
        "ceo_cloud_data_opt_in.revoked"
    }
    .to_string();

    let actor_id = ctx.actor_id.clone();
    WriteCommandExecutor::new(pool.clone())
        .execute_atomic(ctx, request, move |tx| {
            Box::pin(async move {
                settings_store::save_setting_tx(
                    tx,
                    CEO_CLOUD_DATA_OPT_IN_SETTING,
                    if enabled { "true" } else { "false" },
                )
                .await
                .map_err(system_error)?;

                let summary = serde_json::json!({
                    "enabled": enabled,
                    "setting": CEO_CLOUD_DATA_OPT_IN_SETTING,
                    "retention": {
                        "raw_items": [
                            { "category": "raw_prompt", "retention": RAW_PROMPT_RETENTION },
                            { "category": "raw_response", "retention": RAW_RESPONSE_RETENTION },
                            { "category": "raw_tool_output", "retention": RAW_TOOL_OUTPUT_RETENTION },
                            { "category": "raw_provider_error", "retention": RAW_PROVIDER_ERROR_RETENTION },
                        ],
                        "session_meta": SESSION_METADATA_RETENTION,
                        "audit_meta": AUDIT_METADATA_RETENTION,
                        "memory": MEMORY_RETENTION,
                    },
                });

                insert_agent_audit_event_tx(
                    tx,
                    NewAgentAuditEvent {
                        session_id: None,
                        event_type,
                        actor_id,
                        role: Some(AgentRole::CeoSecretary),
                        channel: Some(AgentChannel::Desktop),
                        tool_name: None,
                        provider: Some(AgentProvider::None),
                        policy_outcome: "allowed".to_string(),
                        mutation_risk: Some(MutationRisk::LowWrite),
                        data_sensitivity: Some(DataSensitivity::CeoSensitive),
                        summary,
                    },
                )
                .await?;

                Ok(serde_json::json!({ "enabled": enabled }))
            })
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_idempotency::ActorType;
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

        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("failed to enable foreign keys");

        crate::db::run_migrations(&pool)
            .await
            .expect("failed to run migrations");

        pool
    }

    async fn get_setting_value(pool: &Pool<Sqlite>) -> Option<String> {
        sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
            .bind(CEO_CLOUD_DATA_OPT_IN_SETTING)
            .fetch_optional(pool)
            .await
            .expect("query settings")
    }

    async fn save_setting_value(pool: &Pool<Sqlite>, value: &str) {
        sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?)")
            .bind(CEO_CLOUD_DATA_OPT_IN_SETTING)
            .bind(value)
            .execute(pool)
            .await
            .expect("seed setting");
    }

    async fn audit_rows(pool: &Pool<Sqlite>) -> Vec<(String, String)> {
        // (event_type, summary_json)
        sqlx::query_as::<_, (String, String)>(
            "SELECT event_type, summary_json FROM agent_audit_events ORDER BY id ASC",
        )
        .fetch_all(pool)
        .await
        .expect("read audit rows")
    }

    fn write_ctx(request_id: &str, idempotency_key: &str) -> WriteCommandContext {
        let mut ctx = WriteCommandContext::for_internal_test(
            request_id,
            idempotency_key,
            SET_CEO_CLOUD_DATA_OPT_IN_COMMAND,
        );
        ctx.actor_type = ActorType::Human;
        ctx.actor_id = Some("admin-1".to_string());
        ctx
    }

    #[tokio::test]
    async fn default_missing_setting_returns_false() {
        let pool = test_pool().await;
        let value = get_ceo_cloud_data_opt_in(&pool)
            .await
            .expect("read helper succeeds");
        assert!(!value);
    }

    #[tokio::test]
    async fn stored_numeric_true_setting_returns_true() {
        let pool = test_pool().await;
        save_setting_value(&pool, "1").await;
        let value = get_ceo_cloud_data_opt_in(&pool)
            .await
            .expect("read helper succeeds");
        assert!(value);
    }

    #[tokio::test]
    async fn enabling_persists_setting_and_writes_one_metadata_only_audit_event() {
        let pool = test_pool().await;
        let ctx = write_ctx("req-1", "idem-1");

        set_ceo_cloud_data_opt_in_idempotent(&pool, &ctx, true)
            .await
            .expect("set should succeed");

        assert_eq!(get_setting_value(&pool).await.as_deref(), Some("true"));

        let rows = audit_rows(&pool).await;
        assert_eq!(rows.len(), 1, "exactly one audit event must be written");
        assert_eq!(rows[0].0, "ceo_cloud_data_opt_in.enabled");

        let summary: serde_json::Value =
            serde_json::from_str(&rows[0].1).expect("summary json parses");
        let top_level_keys = summary
            .as_object()
            .expect("summary must be an object")
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(top_level_keys, vec!["enabled", "retention", "setting"]);
        assert_eq!(summary["enabled"], serde_json::json!(true));
        assert_eq!(
            summary["retention"]["raw_items"],
            serde_json::json!([
                { "category": "raw_prompt", "retention": RAW_PROMPT_RETENTION },
                { "category": "raw_response", "retention": RAW_RESPONSE_RETENTION },
                { "category": "raw_tool_output", "retention": RAW_TOOL_OUTPUT_RETENTION },
                { "category": "raw_provider_error", "retention": RAW_PROVIDER_ERROR_RETENTION },
            ])
        );
        assert_eq!(
            summary["retention"]["session_meta"],
            SESSION_METADATA_RETENTION
        );
        assert_eq!(summary["retention"]["audit_meta"], AUDIT_METADATA_RETENTION);
        assert_eq!(summary["retention"]["memory"], MEMORY_RETENTION);
    }

    #[tokio::test]
    async fn revoking_persists_setting_and_writes_revoked_audit_event() {
        let pool = test_pool().await;
        let ctx = write_ctx("req-2", "idem-2");
        set_ceo_cloud_data_opt_in_idempotent(&pool, &ctx, false)
            .await
            .expect("revoke should succeed");

        assert_eq!(get_setting_value(&pool).await.as_deref(), Some("false"));
        let rows = audit_rows(&pool).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "ceo_cloud_data_opt_in.revoked");
    }

    #[tokio::test]
    async fn retry_same_idempotency_key_same_payload_replays_without_duplicate_audit_event() {
        let pool = test_pool().await;
        let ctx = write_ctx("req-3", "idem-3");
        let first = set_ceo_cloud_data_opt_in_idempotent(&pool, &ctx, true)
            .await
            .expect("first succeeds");
        assert!(!first.replayed);

        let second = set_ceo_cloud_data_opt_in_idempotent(&pool, &ctx, true)
            .await
            .expect("second replays");
        assert!(second.replayed);

        let rows = audit_rows(&pool).await;
        assert_eq!(rows.len(), 1, "replay must not duplicate audit events");
    }

    #[tokio::test]
    async fn same_idempotency_key_different_payload_is_rejected() {
        let pool = test_pool().await;
        let ctx = write_ctx("req-4", "idem-4");

        set_ceo_cloud_data_opt_in_idempotent(&pool, &ctx, true)
            .await
            .expect("first succeeds");

        let error = set_ceo_cloud_data_opt_in_idempotent(&pool, &ctx, false)
            .await
            .expect_err("hash mismatch must be rejected");

        assert_eq!(error.code, codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH);
    }
}
