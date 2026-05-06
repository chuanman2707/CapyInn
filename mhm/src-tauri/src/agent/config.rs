use crate::{
    agent::{
        model::{AgentChannel, AgentProvider, AgentRole, DataSensitivity, MutationRisk},
        store::{insert_agent_audit_event_tx, NewAgentAuditEvent},
    },
    app_error::CommandResult,
    command_idempotency::{
        system_error, IdempotentCommandResult, WriteCommandContext, WriteCommandExecutor,
        WriteCommandRequest,
    },
    services::settings_store,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Sqlite, Transaction};
use std::future::Future;

pub const CEO_TELEGRAM_USER_ID_SETTING: &str = "ceo_telegram_user_id";
pub const CEO_TELEGRAM_RUNTIME_ENABLED_SETTING: &str = "ceo_telegram_runtime_enabled";
pub const CEO_TELEGRAM_OPENAI_MODEL_SETTING: &str = "ceo_telegram_openai_model";
pub const CEO_TELEGRAM_LAST_UPDATE_ID_SETTING: &str = "ceo_telegram_last_update_id";
pub const CEO_TELEGRAM_TOKEN_PRESENT_SETTING: &str = "ceo_telegram_bot_token_present";
pub const CEO_OPENAI_KEY_PRESENT_SETTING: &str = "ceo_openai_api_key_present";

pub const SET_CEO_TELEGRAM_CONFIG_COMMAND: &str = "agent.set_ceo_telegram_config";
pub const SET_CEO_TELEGRAM_SECRET_STATUS_COMMAND: &str = "agent.set_ceo_telegram_secret_status";

pub const DEFAULT_CEO_OPENAI_MODEL: &str = "gpt-5";

const CEO_TELEGRAM_SETTINGS_AGGREGATE: &str = "settings:ceo_telegram_chat";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CeoTelegramConfig {
    pub runtime_enabled: bool,
    pub telegram_user_id: Option<String>,
    pub telegram_bot_token_present: bool,
    pub openai_api_key_present: bool,
    pub openai_model: String,
    pub last_update_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CeoTelegramGateMissing {
    CloudDataOptIn,
    RuntimeEnabled,
    TelegramOwnerBinding,
    TelegramBotToken,
    OpenAiApiKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CeoTelegramGateStatus {
    pub ready: bool,
    pub missing: Vec<CeoTelegramGateMissing>,
}

impl CeoTelegramConfig {
    pub fn evaluate_gate(&self, ceo_cloud_data_opt_in: bool) -> CeoTelegramGateStatus {
        let mut missing = Vec::new();
        if !ceo_cloud_data_opt_in {
            missing.push(CeoTelegramGateMissing::CloudDataOptIn);
        }
        if !self.runtime_enabled {
            missing.push(CeoTelegramGateMissing::RuntimeEnabled);
        }
        if self
            .telegram_user_id
            .as_deref()
            .map(|value| value.trim().is_empty())
            .unwrap_or(true)
        {
            missing.push(CeoTelegramGateMissing::TelegramOwnerBinding);
        }
        if !self.telegram_bot_token_present {
            missing.push(CeoTelegramGateMissing::TelegramBotToken);
        }
        if !self.openai_api_key_present {
            missing.push(CeoTelegramGateMissing::OpenAiApiKey);
        }

        CeoTelegramGateStatus {
            ready: missing.is_empty(),
            missing,
        }
    }
}

pub async fn get_ceo_telegram_config(pool: &Pool<Sqlite>) -> CommandResult<CeoTelegramConfig> {
    let runtime_enabled = read_bool_setting(pool, CEO_TELEGRAM_RUNTIME_ENABLED_SETTING).await?;
    let telegram_user_id =
        read_optional_trimmed_setting(pool, CEO_TELEGRAM_USER_ID_SETTING).await?;
    let telegram_bot_token_present =
        read_bool_setting(pool, CEO_TELEGRAM_TOKEN_PRESENT_SETTING).await?;
    let openai_api_key_present = read_bool_setting(pool, CEO_OPENAI_KEY_PRESENT_SETTING).await?;
    let openai_model = read_optional_trimmed_setting(pool, CEO_TELEGRAM_OPENAI_MODEL_SETTING)
        .await?
        .unwrap_or_else(|| DEFAULT_CEO_OPENAI_MODEL.to_string());
    let last_update_id =
        read_optional_i64_setting(pool, CEO_TELEGRAM_LAST_UPDATE_ID_SETTING).await?;

    Ok(CeoTelegramConfig {
        runtime_enabled,
        telegram_user_id,
        telegram_bot_token_present,
        openai_api_key_present,
        openai_model,
        last_update_id,
    })
}

pub async fn set_ceo_telegram_config_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    runtime_enabled: bool,
    telegram_user_id: Option<String>,
    openai_model: String,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let telegram_user_id = telegram_user_id.and_then(|value| {
        let trimmed = value.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    });
    let openai_model = normalize_openai_model(openai_model);
    let owner_fingerprint = telegram_user_id
        .as_deref()
        .map(safe_fingerprint)
        .unwrap_or_else(|| "unbound".to_string());

    let request = WriteCommandRequest::new_low_risk(
        serde_json::json!({
            "runtime_enabled": runtime_enabled,
            "owner_binding": if telegram_user_id.is_some() { "bound" } else { "unbound" },
            "owner_fingerprint": owner_fingerprint,
            "model": openai_model,
        }),
        "Set CEO Telegram configuration",
    )?
    .with_primary_aggregate_key(CEO_TELEGRAM_SETTINGS_AGGREGATE)
    .with_lock_key_deriver(ceo_telegram_settings_lock_keys);

    let actor_id = ctx.actor_id.clone();
    WriteCommandExecutor::new(pool.clone())
        .execute_atomic(ctx, request, move |tx| {
            Box::pin(async move {
                let previous_runtime =
                    read_bool_setting_tx(tx, CEO_TELEGRAM_RUNTIME_ENABLED_SETTING).await?;

                settings_store::save_setting_tx(
                    tx,
                    CEO_TELEGRAM_RUNTIME_ENABLED_SETTING,
                    bool_setting(runtime_enabled),
                )
                .await
                .map_err(system_error)?;
                settings_store::save_setting_tx(
                    tx,
                    CEO_TELEGRAM_USER_ID_SETTING,
                    telegram_user_id.as_deref().unwrap_or(""),
                )
                .await
                .map_err(system_error)?;
                settings_store::save_setting_tx(
                    tx,
                    CEO_TELEGRAM_OPENAI_MODEL_SETTING,
                    &openai_model,
                )
                .await
                .map_err(system_error)?;

                let event_type = runtime_event_type(previous_runtime, runtime_enabled);
                insert_agent_audit_event_tx(
                    tx,
                    NewAgentAuditEvent {
                        session_id: None,
                        event_type: event_type.to_string(),
                        actor_id,
                        role: Some(AgentRole::CeoSecretary),
                        channel: Some(AgentChannel::Desktop),
                        tool_name: None,
                        provider: Some(AgentProvider::None),
                        policy_outcome: "allowed".to_string(),
                        mutation_risk: Some(MutationRisk::LowWrite),
                        data_sensitivity: Some(DataSensitivity::CeoSensitive),
                        summary: serde_json::json!({
                            "runtime_enabled": runtime_enabled,
                            "owner_binding": if telegram_user_id.is_some() { "bound" } else { "unbound" },
                            "model": openai_model,
                            "setting_scope": "ceo_telegram_chat",
                        }),
                    },
                )
                .await?;

                Ok(serde_json::json!({
                    "runtime_enabled": runtime_enabled,
                    "owner_binding": if telegram_user_id.is_some() { "bound" } else { "unbound" },
                    "model": openai_model,
                }))
            })
        })
        .await
}

pub async fn set_ceo_telegram_secret_presence_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    telegram_bot_token_present: bool,
    openai_api_key_present: bool,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    update_ceo_telegram_secret_presence_idempotent(
        pool,
        ctx,
        Some(telegram_bot_token_present),
        Some(openai_api_key_present),
    )
    .await
}

pub async fn update_ceo_telegram_secret_presence_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    telegram_bot_token_present: Option<bool>,
    openai_api_key_present: Option<bool>,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let payload = serde_json::json!({
        "telegram_credential": presence_intent_value(telegram_bot_token_present),
        "ai_provider_credential": presence_intent_value(openai_api_key_present),
    });

    update_ceo_telegram_secret_presence_with_guard_idempotent(
        pool,
        ctx,
        telegram_bot_token_present,
        openai_api_key_present,
        payload,
        || async { Ok(()) },
    )
    .await
}

pub async fn update_ceo_telegram_secret_presence_with_guard_idempotent<Before, Fut>(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    telegram_bot_token_present: Option<bool>,
    openai_api_key_present: Option<bool>,
    idempotency_payload: serde_json::Value,
    before_transaction: Before,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>>
where
    Before: FnOnce() -> Fut,
    Fut: Future<Output = CommandResult<()>> + Send,
{
    let request = WriteCommandRequest::new_low_risk(
        idempotency_payload,
        "Set CEO Telegram credential status",
    )?
    .with_primary_aggregate_key(CEO_TELEGRAM_SETTINGS_AGGREGATE)
    .with_lock_key_deriver(ceo_telegram_settings_lock_keys);

    let actor_id = ctx.actor_id.clone();
    WriteCommandExecutor::new(pool.clone())
        .execute_with_pre_transaction_guard(ctx, request, before_transaction, move |tx| {
            Box::pin(async move {
                let current_telegram =
                    read_bool_setting_tx(tx, CEO_TELEGRAM_TOKEN_PRESENT_SETTING).await?;
                let current_openai =
                    read_bool_setting_tx(tx, CEO_OPENAI_KEY_PRESENT_SETTING).await?;
                let telegram_present = telegram_bot_token_present.unwrap_or(current_telegram);
                let openai_present = openai_api_key_present.unwrap_or(current_openai);

                settings_store::save_setting_tx(
                    tx,
                    CEO_TELEGRAM_TOKEN_PRESENT_SETTING,
                    bool_setting(telegram_present),
                )
                .await
                .map_err(system_error)?;
                settings_store::save_setting_tx(
                    tx,
                    CEO_OPENAI_KEY_PRESENT_SETTING,
                    bool_setting(openai_present),
                )
                .await
                .map_err(system_error)?;

                insert_agent_audit_event_tx(
                    tx,
                    NewAgentAuditEvent {
                        session_id: None,
                        event_type: "ceo_telegram_secret_status.updated".to_string(),
                        actor_id,
                        role: Some(AgentRole::CeoSecretary),
                        channel: Some(AgentChannel::Desktop),
                        tool_name: None,
                        provider: Some(AgentProvider::None),
                        policy_outcome: "allowed".to_string(),
                        mutation_risk: Some(MutationRisk::LowWrite),
                        data_sensitivity: Some(DataSensitivity::CeoSensitive),
                        summary: serde_json::json!({
                            "telegram_credential_present": telegram_present,
                            "ai_provider_credential_present": openai_present,
                            "setting_scope": "ceo_telegram_chat",
                        }),
                    },
                )
                .await?;

                Ok(serde_json::json!({
                    "telegram_credential_present": telegram_present,
                    "ai_provider_credential_present": openai_present,
                }))
            })
        })
        .await
}

pub async fn set_ceo_telegram_last_update_id_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    last_update_id: Option<i64>,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let request = WriteCommandRequest::new_low_risk(
        serde_json::json!({
            "offset_present": last_update_id.is_some(),
            "offset_fingerprint": last_update_id
                .map(|value| safe_fingerprint(&value.to_string()))
                .unwrap_or_else(|| "none".to_string()),
        }),
        "Set CEO Telegram update offset",
    )?
    .with_primary_aggregate_key(CEO_TELEGRAM_SETTINGS_AGGREGATE)
    .with_lock_key_deriver(ceo_telegram_settings_lock_keys);

    let actor_id = ctx.actor_id.clone();
    WriteCommandExecutor::new(pool.clone())
        .execute_atomic(ctx, request, move |tx| {
            Box::pin(async move {
                settings_store::save_setting_tx(
                    tx,
                    CEO_TELEGRAM_LAST_UPDATE_ID_SETTING,
                    &last_update_id
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                )
                .await
                .map_err(system_error)?;

                insert_agent_audit_event_tx(
                    tx,
                    NewAgentAuditEvent {
                        session_id: None,
                        event_type: "ceo_telegram_config.updated".to_string(),
                        actor_id,
                        role: Some(AgentRole::CeoSecretary),
                        channel: Some(AgentChannel::Desktop),
                        tool_name: None,
                        provider: Some(AgentProvider::None),
                        policy_outcome: "allowed".to_string(),
                        mutation_risk: Some(MutationRisk::LowWrite),
                        data_sensitivity: Some(DataSensitivity::CeoSensitive),
                        summary: serde_json::json!({
                            "offset_present": last_update_id.is_some(),
                            "setting_scope": "ceo_telegram_chat",
                        }),
                    },
                )
                .await?;

                Ok(serde_json::json!({
                    "offset_present": last_update_id.is_some(),
                }))
            })
        })
        .await
}

fn ceo_telegram_settings_lock_keys(_intent: &serde_json::Value) -> CommandResult<Vec<String>> {
    Ok(vec![CEO_TELEGRAM_SETTINGS_AGGREGATE.to_string()])
}

fn bool_setting(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn parse_bool_setting(value: Option<String>) -> bool {
    value
        .as_deref()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            normalized == "true" || normalized == "1"
        })
        .unwrap_or(false)
}

fn normalize_openai_model(value: String) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        DEFAULT_CEO_OPENAI_MODEL.to_string()
    } else {
        trimmed.to_string()
    }
}

fn runtime_event_type(previous: bool, next: bool) -> &'static str {
    match (previous, next) {
        (false, true) => "ceo_telegram_runtime.enabled",
        (true, false) => "ceo_telegram_runtime.disabled",
        _ => "ceo_telegram_config.updated",
    }
}

fn presence_intent_value(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "present",
        Some(false) => "absent",
        None => "preserve",
    }
}

fn safe_fingerprint(value: &str) -> String {
    Sha256::digest(value.as_bytes())
        .iter()
        .flat_map(|byte| {
            [
                char::from(b'a' + (byte >> 4)),
                char::from(b'a' + (byte & 0x0f)),
            ]
        })
        .collect()
}

async fn read_bool_setting(pool: &Pool<Sqlite>, key: &str) -> CommandResult<bool> {
    let value = settings_store::get_setting(pool, key)
        .await
        .map_err(system_error)?;
    Ok(parse_bool_setting(value))
}

async fn read_bool_setting_tx(tx: &mut Transaction<'_, Sqlite>, key: &str) -> CommandResult<bool> {
    let value = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(&mut **tx)
        .await
        .map_err(system_error)?;
    Ok(parse_bool_setting(value))
}

async fn read_optional_trimmed_setting(
    pool: &Pool<Sqlite>,
    key: &str,
) -> CommandResult<Option<String>> {
    Ok(settings_store::get_setting(pool, key)
        .await
        .map_err(system_error)?
        .and_then(|value| {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }))
}

async fn read_optional_i64_setting(pool: &Pool<Sqlite>, key: &str) -> CommandResult<Option<i64>> {
    let value = read_optional_trimmed_setting(pool, key).await?;
    value
        .map(|value| value.parse::<i64>().map_err(system_error))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_idempotency::{ActorType, WriteCommandContext};
    use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};

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

    fn write_ctx(
        request_id: &str,
        idempotency_key: &str,
        command_name: &str,
    ) -> WriteCommandContext {
        let mut ctx =
            WriteCommandContext::for_internal_test(request_id, idempotency_key, command_name);
        ctx.actor_type = ActorType::Human;
        ctx.actor_id = Some("admin-1".to_string());
        ctx
    }

    async fn setting_value(pool: &Pool<Sqlite>, key: &str) -> Option<String> {
        sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
            .bind(key)
            .fetch_optional(pool)
            .await
            .expect("query settings")
    }

    async fn audit_rows(pool: &Pool<Sqlite>) -> Vec<(String, String)> {
        sqlx::query_as::<_, (String, String)>(
            "SELECT event_type, summary_json FROM agent_audit_events ORDER BY id ASC",
        )
        .fetch_all(pool)
        .await
        .expect("read audit rows")
    }

    #[test]
    fn gate_requires_every_runtime_dependency() {
        let config = CeoTelegramConfig {
            runtime_enabled: true,
            telegram_user_id: Some("123456".to_string()),
            telegram_bot_token_present: true,
            openai_api_key_present: true,
            openai_model: "gpt-5".to_string(),
            last_update_id: None,
        };

        let gate = config.evaluate_gate(true);

        assert!(gate.ready);
        assert!(gate.missing.is_empty());
    }

    #[test]
    fn gate_reports_missing_owner_binding() {
        let config = CeoTelegramConfig {
            runtime_enabled: true,
            telegram_user_id: None,
            telegram_bot_token_present: true,
            openai_api_key_present: true,
            openai_model: "gpt-5".to_string(),
            last_update_id: None,
        };

        let gate = config.evaluate_gate(true);

        assert!(!gate.ready);
        assert_eq!(
            gate.missing,
            vec![CeoTelegramGateMissing::TelegramOwnerBinding]
        );
    }

    #[tokio::test]
    async fn config_write_persists_settings_and_replays_idempotently() {
        let pool = test_pool().await;
        let ctx = write_ctx(
            "req-config-1",
            "idem-config-1",
            SET_CEO_TELEGRAM_CONFIG_COMMAND,
        );

        let first = set_ceo_telegram_config_idempotent(
            &pool,
            &ctx,
            true,
            Some("123456789".to_string()),
            DEFAULT_CEO_OPENAI_MODEL.to_string(),
        )
        .await
        .expect("first write succeeds");
        assert!(!first.replayed);

        let second = set_ceo_telegram_config_idempotent(
            &pool,
            &ctx,
            true,
            Some("123456789".to_string()),
            DEFAULT_CEO_OPENAI_MODEL.to_string(),
        )
        .await
        .expect("second write replays");
        assert!(second.replayed);

        assert_eq!(
            setting_value(&pool, CEO_TELEGRAM_RUNTIME_ENABLED_SETTING)
                .await
                .as_deref(),
            Some("true")
        );
        assert_eq!(
            setting_value(&pool, CEO_TELEGRAM_USER_ID_SETTING)
                .await
                .as_deref(),
            Some("123456789")
        );
        assert_eq!(
            setting_value(&pool, CEO_TELEGRAM_OPENAI_MODEL_SETTING)
                .await
                .as_deref(),
            Some(DEFAULT_CEO_OPENAI_MODEL)
        );

        let rows = audit_rows(&pool).await;
        assert_eq!(rows.len(), 1, "idempotent replay must not duplicate audit");
        assert_eq!(rows[0].0, "ceo_telegram_runtime.enabled");
        assert!(!rows[0].1.contains("123456789"));
    }

    #[tokio::test]
    async fn secret_presence_metadata_persists_without_secret_values() {
        let pool = test_pool().await;
        let ctx = write_ctx(
            "req-secret-status-1",
            "idem-secret-status-1",
            SET_CEO_TELEGRAM_SECRET_STATUS_COMMAND,
        );

        set_ceo_telegram_secret_presence_idempotent(&pool, &ctx, true, false)
            .await
            .expect("secret presence write succeeds");

        assert_eq!(
            setting_value(&pool, CEO_TELEGRAM_TOKEN_PRESENT_SETTING)
                .await
                .as_deref(),
            Some("true")
        );
        assert_eq!(
            setting_value(&pool, CEO_OPENAI_KEY_PRESENT_SETTING)
                .await
                .as_deref(),
            Some("false")
        );

        let rows = audit_rows(&pool).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "ceo_telegram_secret_status.updated");
        assert!(!rows[0].1.contains("telegram_bot_token"));
        assert!(!rows[0].1.contains("openai_api_key"));
        assert!(!rows[0].1.contains("sk-"));
    }
}
