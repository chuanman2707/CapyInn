use crate::{
    agent::{
        config::get_ceo_telegram_config,
        config::CeoTelegramConfig,
        model::{AgentChannel, AgentProvider, AgentRole, DataSensitivity, MutationRisk},
        store::{insert_agent_audit_event_tx, NewAgentAuditEvent},
    },
    app_error::CommandResult,
    command_idempotency::{
        system_error, CommandLedgerSummary, IdempotentCommandResult, SanitizedLedgerIntent,
        WriteCommandContext, WriteCommandExecutor, WriteCommandRequest,
    },
    services::settings_store,
};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite, Transaction};

pub const CEO_HOURLY_DIGEST_ENABLED_SETTING: &str = "ceo_hourly_digest_enabled";
pub const CEO_TELEGRAM_DELIVERY_CHAT_ID_SETTING: &str = "ceo_telegram_delivery_chat_id";
pub const SET_CEO_DIGEST_CONFIG_COMMAND: &str = "agent.set_ceo_digest_config";
pub const SET_CEO_TELEGRAM_DELIVERY_CHAT_ID_COMMAND: &str =
    "agent.set_ceo_telegram_delivery_chat_id";

const CEO_DIGEST_SETTINGS_AGGREGATE: &str = "settings:ceo_hourly_digest";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CeoDigestConfig {
    pub digest_enabled: bool,
    pub telegram_user_id: Option<String>,
    pub telegram_delivery_chat_id: Option<i64>,
    pub telegram_bot_token_present: bool,
    pub openai_api_key_present: bool,
    pub openai_model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CeoDigestGateMissing {
    CloudDataOptIn,
    DigestEnabled,
    TelegramOwnerBinding,
    TelegramDeliveryChatId,
    TelegramBotToken,
    OpenAiApiKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CeoDigestGateStatus {
    pub ready: bool,
    pub missing: Vec<CeoDigestGateMissing>,
}

impl CeoDigestConfig {
    pub fn from_telegram_config(
        telegram: CeoTelegramConfig,
        digest_enabled: bool,
        telegram_delivery_chat_id: Option<i64>,
    ) -> Self {
        Self {
            digest_enabled,
            telegram_user_id: telegram.telegram_user_id,
            telegram_delivery_chat_id,
            telegram_bot_token_present: telegram.telegram_bot_token_present,
            openai_api_key_present: telegram.openai_api_key_present,
            openai_model: telegram.openai_model,
        }
    }

    pub fn evaluate_gate(&self, ceo_cloud_data_opt_in: bool) -> CeoDigestGateStatus {
        let mut missing = Vec::new();
        if !ceo_cloud_data_opt_in {
            missing.push(CeoDigestGateMissing::CloudDataOptIn);
        }
        if !self.digest_enabled {
            missing.push(CeoDigestGateMissing::DigestEnabled);
        }
        if self
            .telegram_user_id
            .as_deref()
            .map(|value| value.trim().is_empty())
            .unwrap_or(true)
        {
            missing.push(CeoDigestGateMissing::TelegramOwnerBinding);
        }
        if self.telegram_delivery_chat_id.is_none() {
            missing.push(CeoDigestGateMissing::TelegramDeliveryChatId);
        }
        if !self.telegram_bot_token_present {
            missing.push(CeoDigestGateMissing::TelegramBotToken);
        }
        if !self.openai_api_key_present {
            missing.push(CeoDigestGateMissing::OpenAiApiKey);
        }

        CeoDigestGateStatus {
            ready: missing.is_empty(),
            missing,
        }
    }
}

pub async fn get_ceo_digest_config(pool: &Pool<Sqlite>) -> CommandResult<CeoDigestConfig> {
    let telegram = get_ceo_telegram_config(pool).await?;
    let digest_enabled = read_bool_setting(pool, CEO_HOURLY_DIGEST_ENABLED_SETTING).await?;
    let telegram_delivery_chat_id =
        read_optional_i64_setting(pool, CEO_TELEGRAM_DELIVERY_CHAT_ID_SETTING).await?;

    Ok(CeoDigestConfig::from_telegram_config(
        telegram,
        digest_enabled,
        telegram_delivery_chat_id,
    ))
}

pub async fn set_ceo_digest_config_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    digest_enabled: bool,
    telegram_delivery_chat_id: Option<i64>,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let delivery_chat_id_present = telegram_delivery_chat_id.is_some();

    let hash_payload = serde_json::json!({
        "digest_enabled": digest_enabled,
        "telegram_delivery_chat_id": telegram_delivery_chat_id,
        "setting_scope": "ceo_hourly_digest",
    });
    let ledger_intent = SanitizedLedgerIntent::from_pairs([
        ("digest_enabled", serde_json::json!(digest_enabled)),
        (
            "delivery_chat_id_present",
            serde_json::json!(delivery_chat_id_present),
        ),
        ("setting_scope", serde_json::json!("ceo_hourly_digest")),
    ])?;

    let request = WriteCommandRequest::new_sanitized(
        hash_payload,
        ledger_intent,
        CommandLedgerSummary::new("Set CEO hourly digest configuration")?,
    )?
    .with_primary_aggregate_key(CEO_DIGEST_SETTINGS_AGGREGATE)
    .with_lock_key_deriver(ceo_digest_settings_lock_keys);

    let actor_id = ctx.actor_id.clone();
    WriteCommandExecutor::new(pool.clone())
        .execute_atomic(ctx, request, move |tx| {
            Box::pin(async move {
                settings_store::save_setting_tx(
                    tx,
                    CEO_HOURLY_DIGEST_ENABLED_SETTING,
                    bool_setting(digest_enabled),
                )
                .await
                .map_err(system_error)?;
                settings_store::save_setting_tx(
                    tx,
                    CEO_TELEGRAM_DELIVERY_CHAT_ID_SETTING,
                    &telegram_delivery_chat_id
                        .map(|value| value.to_string())
                        .unwrap_or_default(),
                )
                .await
                .map_err(system_error)?;

                insert_agent_audit_event_tx(
                    tx,
                    NewAgentAuditEvent {
                        session_id: None,
                        event_type: "ceo_hourly_digest_config.updated".to_string(),
                        actor_id,
                        role: Some(AgentRole::CeoSecretary),
                        channel: Some(AgentChannel::Desktop),
                        tool_name: None,
                        provider: Some(AgentProvider::None),
                        policy_outcome: "allowed".to_string(),
                        mutation_risk: Some(MutationRisk::LowWrite),
                        data_sensitivity: Some(DataSensitivity::CeoSensitive),
                        summary: serde_json::json!({
                            "digest_enabled": digest_enabled,
                            "delivery_chat_id_present": delivery_chat_id_present,
                            "setting_scope": "ceo_hourly_digest",
                        }),
                    },
                )
                .await?;

                Ok(serde_json::json!({
                    "digest_enabled": digest_enabled,
                    "delivery_chat_id_present": delivery_chat_id_present,
                    "setting_scope": "ceo_hourly_digest",
                }))
            })
        })
        .await
}

fn ceo_digest_settings_lock_keys(_intent: &serde_json::Value) -> CommandResult<Vec<String>> {
    Ok(vec![CEO_DIGEST_SETTINGS_AGGREGATE.to_string()])
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

async fn read_bool_setting(pool: &Pool<Sqlite>, key: &str) -> CommandResult<bool> {
    let value = settings_store::get_setting(pool, key)
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

#[allow(dead_code)]
async fn read_bool_setting_tx(tx: &mut Transaction<'_, Sqlite>, key: &str) -> CommandResult<bool> {
    let value = sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(&mut **tx)
        .await
        .map_err(system_error)?;
    Ok(parse_bool_setting(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::config::{
            set_ceo_telegram_config_idempotent, set_ceo_telegram_secret_presence_idempotent,
        },
        agent::config::{
            DEFAULT_CEO_OPENAI_MODEL, SET_CEO_TELEGRAM_CONFIG_COMMAND,
            SET_CEO_TELEGRAM_SECRET_STATUS_COMMAND,
        },
        command_idempotency::{ActorType, WriteCommandContext},
    };
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

    async fn command_intents(pool: &Pool<Sqlite>, command_name: &str) -> Vec<String> {
        sqlx::query_scalar::<_, String>(
            "SELECT intent_json FROM command_idempotency WHERE command_name = ? ORDER BY id ASC",
        )
        .bind(command_name)
        .fetch_all(pool)
        .await
        .expect("read command intents")
    }

    #[test]
    fn digest_gate_does_not_require_chat_runtime_enabled() {
        let telegram = CeoTelegramConfig {
            runtime_enabled: false,
            telegram_user_id: Some("123456789".to_string()),
            telegram_bot_token_present: true,
            openai_api_key_present: true,
            openai_model: DEFAULT_CEO_OPENAI_MODEL.to_string(),
            last_update_id: None,
        };
        let config = CeoDigestConfig::from_telegram_config(telegram, true, Some(987654321));

        let gate = config.evaluate_gate(true);

        assert!(gate.ready);
        assert!(gate.missing.is_empty());
    }

    #[test]
    fn digest_gate_requires_delivery_chat_id() {
        let telegram = CeoTelegramConfig {
            runtime_enabled: false,
            telegram_user_id: Some("123456789".to_string()),
            telegram_bot_token_present: true,
            openai_api_key_present: true,
            openai_model: DEFAULT_CEO_OPENAI_MODEL.to_string(),
            last_update_id: None,
        };
        let config = CeoDigestConfig::from_telegram_config(telegram, true, None);

        let gate = config.evaluate_gate(true);

        assert!(!gate.ready);
        assert_eq!(
            gate.missing,
            vec![CeoDigestGateMissing::TelegramDeliveryChatId]
        );
    }

    #[tokio::test]
    async fn digest_config_write_persists_and_replays() {
        let pool = test_pool().await;
        let telegram_ctx = write_ctx(
            "req-telegram-config-1",
            "idem-telegram-config-1",
            SET_CEO_TELEGRAM_CONFIG_COMMAND,
        );
        set_ceo_telegram_config_idempotent(
            &pool,
            &telegram_ctx,
            false,
            Some("123456789".to_string()),
            DEFAULT_CEO_OPENAI_MODEL.to_string(),
        )
        .await
        .expect("telegram config write succeeds");

        let secret_ctx = write_ctx(
            "req-secret-status-1",
            "idem-secret-status-1",
            SET_CEO_TELEGRAM_SECRET_STATUS_COMMAND,
        );
        set_ceo_telegram_secret_presence_idempotent(&pool, &secret_ctx, true, true)
            .await
            .expect("secret presence write succeeds");

        let ctx = write_ctx(
            "req-digest-config-1",
            "idem-digest-config-1",
            SET_CEO_DIGEST_CONFIG_COMMAND,
        );
        let first = set_ceo_digest_config_idempotent(&pool, &ctx, true, Some(987654321))
            .await
            .expect("first digest config write succeeds");
        assert!(!first.replayed);

        let second = set_ceo_digest_config_idempotent(&pool, &ctx, true, Some(987654321))
            .await
            .expect("second digest config write replays");
        assert!(second.replayed);

        assert_eq!(
            setting_value(&pool, CEO_HOURLY_DIGEST_ENABLED_SETTING)
                .await
                .as_deref(),
            Some("true")
        );
        assert_eq!(
            setting_value(&pool, CEO_TELEGRAM_DELIVERY_CHAT_ID_SETTING)
                .await
                .as_deref(),
            Some("987654321")
        );

        let config = get_ceo_digest_config(&pool)
            .await
            .expect("digest config reads back");
        assert!(config.digest_enabled);
        assert_eq!(config.telegram_delivery_chat_id, Some(987654321));
        assert_eq!(config.telegram_user_id.as_deref(), Some("123456789"));
        assert!(config.telegram_bot_token_present);
        assert!(config.openai_api_key_present);

        let digest_rows: Vec<_> = audit_rows(&pool)
            .await
            .into_iter()
            .filter(|row| row.0 == "ceo_hourly_digest_config.updated")
            .collect();
        assert_eq!(
            digest_rows.len(),
            1,
            "idempotent replay must not duplicate digest audit"
        );
        assert!(!digest_rows[0].1.contains("987654321"));
        assert!(digest_rows[0]
            .1
            .contains("\"delivery_chat_id_present\":true"));

        let digest_intents = command_intents(&pool, SET_CEO_DIGEST_CONFIG_COMMAND).await;
        assert_eq!(digest_intents.len(), 1);
        assert!(!digest_intents[0].contains("987654321"));
        assert!(!digest_intents[0].contains("fingerprint"));
        assert!(digest_intents[0].contains("\"delivery_chat_id_present\":true"));
    }
}
