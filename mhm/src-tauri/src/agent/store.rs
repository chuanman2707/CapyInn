use crate::{
    agent::{
        model::{AgentChannel, AgentProvider, AgentRole, DataSensitivity, MutationRisk},
        retention::validate_session_retention_policy,
    },
    app_error::{codes, CommandError, CommandResult},
    command_idempotency::system_error,
};
use serde_json::{Map, Value};
use sqlx::{Pool, Sqlite, Transaction};

#[derive(Debug, Clone)]
pub struct NewAgentSession {
    pub id: String,
    pub role: AgentRole,
    pub channel: AgentChannel,
    pub channel_actor_id: Option<String>,
    pub uses_memory: bool,
    pub retention_policy: String,
    pub metadata: Value,
}

#[derive(Debug, Clone)]
pub struct NewAgentAuditEvent {
    pub session_id: Option<String>,
    pub event_type: String,
    pub actor_id: Option<String>,
    pub role: Option<AgentRole>,
    pub channel: Option<AgentChannel>,
    pub tool_name: Option<String>,
    pub provider: Option<AgentProvider>,
    pub policy_outcome: String,
    pub mutation_risk: Option<MutationRisk>,
    pub data_sensitivity: Option<DataSensitivity>,
    pub summary: Value,
}

#[derive(Debug, Clone)]
pub struct NewAgentMemoryItem {
    pub id: String,
    pub role: AgentRole,
    pub scope: String,
    pub key: String,
    pub value: Value,
}

fn role_as_str(role: AgentRole) -> &'static str {
    match role {
        AgentRole::CeoSecretary => "ceo_secretary",
        AgentRole::GuestReceptionist => "guest_receptionist",
    }
}

fn channel_as_str(channel: AgentChannel) -> &'static str {
    match channel {
        AgentChannel::Desktop => "desktop",
        AgentChannel::Telegram => "telegram",
    }
}

fn provider_as_str(provider: AgentProvider) -> &'static str {
    match provider {
        AgentProvider::None => "none",
        AgentProvider::OpenAi => "open_ai",
    }
}

fn mutation_risk_as_str(value: MutationRisk) -> &'static str {
    match value {
        MutationRisk::ReadOnly => "read_only",
        MutationRisk::LowWrite => "low_write",
        MutationRisk::HighWrite => "high_write",
    }
}

fn data_sensitivity_as_str(value: DataSensitivity) -> &'static str {
    match value {
        DataSensitivity::PublicHotelInfo => "public_hotel_info",
        DataSensitivity::GuestScoped => "guest_scoped",
        DataSensitivity::StaffOperational => "staff_operational",
        DataSensitivity::CeoSensitive => "ceo_sensitive",
    }
}

fn stable_json(value: &Value) -> CommandResult<String> {
    serde_json::to_string(value).map_err(system_error)
}

fn sanitize_metadata(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let sanitized = map
                .iter()
                .map(|(key, child)| {
                    if is_sensitive_metadata_key(key) {
                        (key.clone(), Value::String("[redacted]".to_string()))
                    } else {
                        (key.clone(), sanitize_metadata(child))
                    }
                })
                .collect::<Map<String, Value>>();
            Value::Object(sanitized)
        }
        Value::Array(items) => Value::Array(items.iter().map(sanitize_metadata).collect()),
        Value::String(text) if contains_obvious_secret_marker(text) => {
            Value::String("[redacted]".to_string())
        }
        _ => value.clone(),
    }
}

fn is_sensitive_metadata_key(key: &str) -> bool {
    let normalized = normalized_category(key);
    [
        "raw_prompt",
        "prompt",
        "raw_response",
        "response",
        "raw_tool_output",
        "tool_output",
        "provider_key",
        "api_key",
        "openai_api_key",
        "bot_token",
        "telegram_bot_token",
        "token",
        "secret",
        "password",
        "authorization",
    ]
    .iter()
    .map(|marker| normalized_category(marker))
    .any(|marker| normalized.contains(&marker))
}

fn contains_obvious_secret_marker(value: &str) -> bool {
    let normalized = value.to_ascii_lowercase();
    [
        "capyinn_sk_",
        "openai_api_key",
        "sk-",
        "bot_token",
        "telegram_bot_token",
        "provider_key",
        "bearer ",
    ]
    .iter()
    .any(|marker| normalized.contains(marker))
}

fn contains_forbidden_memory_truth(value: &str) -> bool {
    let normalized = normalized_category(value);
    [
        "canonical_booking",
        "canonical_booking_state",
        "booking_truth",
        "room_availability_truth",
        "payment_truth",
        "folio_truth",
        "invoice_truth",
        "ledger_truth",
        "housekeeping_truth",
        "night_audit_truth",
        "audit_truth",
        "auto_mutating_recovery_commands",
    ]
    .iter()
    .map(|forbidden| normalized_category(forbidden))
    .any(|forbidden| normalized.contains(&forbidden))
}

fn normalized_category(value: &str) -> String {
    value
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|character| character.is_ascii_alphanumeric())
        .collect()
}

fn validate_memory_item(input: &NewAgentMemoryItem) -> CommandResult<()> {
    let value_text = input.value.to_string();
    if contains_forbidden_memory_truth(&input.scope)
        || contains_forbidden_memory_truth(&input.key)
        || contains_forbidden_memory_truth(&value_text)
    {
        return Err(CommandError::user(
            codes::AGENT_MEMORY_FORBIDDEN_TRUTH,
            "Agent memory cannot store PMS truth",
        ));
    }
    Ok(())
}

pub async fn create_agent_session(
    pool: &Pool<Sqlite>,
    input: NewAgentSession,
) -> CommandResult<()> {
    validate_session_retention_policy(&input.retention_policy)?;
    let now = chrono::Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO agent_sessions (
            id, role, channel, channel_actor_id, status, uses_memory,
            retention_policy, metadata_json, started_at, last_seen_at, ended_at
         ) VALUES (?, ?, ?, ?, 'active', ?, ?, ?, ?, ?, NULL)",
    )
    .bind(&input.id)
    .bind(role_as_str(input.role))
    .bind(channel_as_str(input.channel))
    .bind(&input.channel_actor_id)
    .bind(if input.uses_memory { 1_i64 } else { 0_i64 })
    .bind(&input.retention_policy)
    .bind(stable_json(&sanitize_metadata(&input.metadata))?)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(system_error)?;

    Ok(())
}

pub async fn insert_agent_audit_event(
    pool: &Pool<Sqlite>,
    input: NewAgentAuditEvent,
) -> CommandResult<()> {
    let mut tx = pool.begin().await.map_err(system_error)?;
    insert_agent_audit_event_tx(&mut tx, input).await?;
    tx.commit().await.map_err(system_error)?;
    Ok(())
}

pub async fn insert_agent_audit_event_tx(
    tx: &mut Transaction<'_, Sqlite>,
    input: NewAgentAuditEvent,
) -> CommandResult<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let summary = sanitize_metadata(&input.summary);

    sqlx::query(
        "INSERT INTO agent_audit_events (
            session_id, event_type, actor_id, role, channel, tool_name, provider,
            policy_outcome, mutation_risk, data_sensitivity, summary_json, created_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&input.session_id)
    .bind(&input.event_type)
    .bind(&input.actor_id)
    .bind(input.role.map(role_as_str))
    .bind(input.channel.map(channel_as_str))
    .bind(&input.tool_name)
    .bind(input.provider.map(provider_as_str))
    .bind(&input.policy_outcome)
    .bind(input.mutation_risk.map(mutation_risk_as_str))
    .bind(input.data_sensitivity.map(data_sensitivity_as_str))
    .bind(stable_json(&summary)?)
    .bind(&now)
    .execute(&mut **tx)
    .await
    .map_err(system_error)?;

    Ok(())
}

pub async fn upsert_agent_memory_item(
    pool: &Pool<Sqlite>,
    input: NewAgentMemoryItem,
) -> CommandResult<()> {
    validate_memory_item(&input)?;
    let now = chrono::Utc::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO agent_memory_items (
            id, role, scope, key, value_json, created_at, updated_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(role, scope, key) DO UPDATE SET
            value_json = excluded.value_json,
            updated_at = excluded.updated_at",
    )
    .bind(&input.id)
    .bind(role_as_str(input.role))
    .bind(&input.scope)
    .bind(&input.key)
    .bind(stable_json(&input.value)?)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(system_error)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{
        model::{AgentChannel, AgentRole, DataSensitivity, MutationRisk},
        retention::SESSION_RETENTION_METADATA_ONLY,
    };
    use sqlx::{sqlite::SqlitePoolOptions, Pool, Row, Sqlite};

    async fn test_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connects");
        crate::db::run_migrations(&pool).await.expect("migrations");
        pool
    }

    #[tokio::test]
    async fn create_session_rejects_unknown_retention_policy() {
        let pool = test_pool().await;

        let error = create_agent_session(
            &pool,
            NewAgentSession {
                id: "session-bad-retention".to_string(),
                role: AgentRole::CeoSecretary,
                channel: AgentChannel::Telegram,
                channel_actor_id: Some("12345".to_string()),
                uses_memory: false,
                retention_policy: "raw_prompt_30_days".to_string(),
                metadata: serde_json::json!({}),
            },
        )
        .await
        .expect_err("unknown retention must fail");

        assert_eq!(
            error.code,
            crate::app_error::codes::VALIDATION_INVALID_INPUT
        );
    }

    #[tokio::test]
    async fn create_session_stores_metadata_only_policy() {
        let pool = test_pool().await;

        create_agent_session(
            &pool,
            NewAgentSession {
                id: "session-ok".to_string(),
                role: AgentRole::CeoSecretary,
                channel: AgentChannel::Telegram,
                channel_actor_id: Some("12345".to_string()),
                uses_memory: false,
                retention_policy: SESSION_RETENTION_METADATA_ONLY.to_string(),
                metadata: serde_json::json!({ "source": "test" }),
            },
        )
        .await
        .expect("session stored");

        let row =
            sqlx::query("SELECT retention_policy, metadata_json FROM agent_sessions WHERE id = ?")
                .bind("session-ok")
                .fetch_one(&pool)
                .await
                .expect("reads session");

        assert_eq!(
            row.get::<String, _>("retention_policy"),
            SESSION_RETENTION_METADATA_ONLY
        );
        assert_eq!(
            row.get::<String, _>("metadata_json"),
            "{\"source\":\"test\"}"
        );
    }

    #[tokio::test]
    async fn create_session_stores_sanitized_metadata() {
        let pool = test_pool().await;

        create_agent_session(
            &pool,
            NewAgentSession {
                id: "session-sanitized".to_string(),
                role: AgentRole::CeoSecretary,
                channel: AgentChannel::Telegram,
                channel_actor_id: Some("12345".to_string()),
                uses_memory: false,
                retention_policy: SESSION_RETENTION_METADATA_ONLY.to_string(),
                metadata: serde_json::json!({
                    "source": "test",
                    "raw_prompt": "show private details",
                    "apiKey": "abc123",
                    "provider_key": "capyinn_sk_secret",
                    "providerKey": "plain-provider-key",
                    "authorization": "Bearer abc",
                    "nested": {
                        "OPENAI_API_KEY": "sk-secret",
                        "openaiApiKey": "plain-openai-key",
                        "telegram_bot_token": "telegram-token"
                    }
                }),
            },
        )
        .await
        .expect("session stored");

        let metadata: String =
            sqlx::query_scalar("SELECT metadata_json FROM agent_sessions WHERE id = ?")
                .bind("session-sanitized")
                .fetch_one(&pool)
                .await
                .expect("reads metadata");

        assert!(metadata.contains("\"source\":\"test\""));
        assert!(metadata.contains("\"raw_prompt\":\"[redacted]\""));
        assert!(metadata.contains("\"apiKey\":\"[redacted]\""));
        assert!(metadata.contains("\"provider_key\":\"[redacted]\""));
        assert!(metadata.contains("\"providerKey\":\"[redacted]\""));
        assert!(metadata.contains("\"authorization\":\"[redacted]\""));
        assert!(metadata.contains("\"OPENAI_API_KEY\":\"[redacted]\""));
        assert!(metadata.contains("\"openaiApiKey\":\"[redacted]\""));
        assert!(metadata.contains("\"telegram_bot_token\":\"[redacted]\""));
        assert!(!metadata.contains(' '));
        assert!(!metadata.contains("show private details"));
        assert!(!metadata.contains("abc123"));
        assert!(!metadata.contains("capyinn_sk_secret"));
        assert!(!metadata.contains("plain-provider-key"));
        assert!(!metadata.contains("Bearer abc"));
        assert!(!metadata.contains("sk-secret"));
        assert!(!metadata.contains("plain-openai-key"));
        assert!(!metadata.contains("telegram-token"));
    }

    #[tokio::test]
    async fn audit_event_stores_sanitized_metadata() {
        let pool = test_pool().await;

        insert_agent_audit_event(
            &pool,
            NewAgentAuditEvent {
                session_id: None,
                event_type: "runtime.disabled".to_string(),
                actor_id: Some("admin-1".to_string()),
                role: Some(AgentRole::CeoSecretary),
                channel: Some(AgentChannel::Telegram),
                tool_name: None,
                provider: None,
                policy_outcome: "denied".to_string(),
                mutation_risk: Some(MutationRisk::ReadOnly),
                data_sensitivity: Some(DataSensitivity::CeoSensitive),
                summary: serde_json::json!({
                    "reason": "runtime_disabled",
                    "raw_prompt": "show yesterday revenue",
                    "provider_key": "capyinn_sk_test",
                    "nested": {
                        "bot_token": "OPENAI_TOKEN_TEST"
                    }
                }),
            },
        )
        .await
        .expect("audit event stored");

        let summary: String = sqlx::query_scalar(
            "SELECT summary_json FROM agent_audit_events WHERE event_type = 'runtime.disabled'",
        )
        .fetch_one(&pool)
        .await
        .expect("reads summary");
        assert!(summary.contains("\"reason\":\"runtime_disabled\""));
        assert!(summary.contains("\"raw_prompt\":\"[redacted]\""));
        assert!(summary.contains("\"provider_key\":\"[redacted]\""));
        assert!(summary.contains("\"bot_token\":\"[redacted]\""));
        assert!(!summary.contains(' '));
        assert!(!summary.contains("show yesterday revenue"));
        assert!(!summary.contains("capyinn_sk_"));
        assert!(!summary.contains("OPENAI"));
    }

    #[tokio::test]
    async fn memory_rejects_pms_truth_categories() {
        let pool = test_pool().await;

        let error = upsert_agent_memory_item(
            &pool,
            NewAgentMemoryItem {
                id: "memory-truth".to_string(),
                role: AgentRole::CeoSecretary,
                scope: "canonical_booking_state".to_string(),
                key: "booking-1".to_string(),
                value: serde_json::json!({ "status": "active" }),
            },
        )
        .await
        .expect_err("PMS truth must not be stored in memory");

        assert_eq!(
            error.code,
            crate::app_error::codes::AGENT_MEMORY_FORBIDDEN_TRUTH
        );
    }

    #[tokio::test]
    async fn memory_rejects_normalized_pms_truth_categories() {
        let pool = test_pool().await;

        for (id, scope, key, value) in [
            (
                "memory-auto-scope",
                "auto_mutating_recovery_commands",
                "preference",
                serde_json::json!({ "summary": "ok" }),
            ),
            (
                "memory-auto-key",
                "preference",
                "auto_mutating_recovery_commands",
                serde_json::json!({ "summary": "ok" }),
            ),
            (
                "memory-auto-value",
                "preference",
                "summary",
                serde_json::json!({ "category": "auto_mutating_recovery_commands" }),
            ),
            (
                "memory-camel",
                "canonicalBookingState",
                "booking-1",
                serde_json::json!({ "summary": "ok" }),
            ),
            (
                "memory-hyphen",
                "room-availability truth",
                "room-1",
                serde_json::json!({ "summary": "ok" }),
            ),
            (
                "memory-space",
                "preference",
                "payment truth",
                serde_json::json!({ "summary": "ok" }),
            ),
        ] {
            let error = upsert_agent_memory_item(
                &pool,
                NewAgentMemoryItem {
                    id: id.to_string(),
                    role: AgentRole::CeoSecretary,
                    scope: scope.to_string(),
                    key: key.to_string(),
                    value,
                },
            )
            .await
            .expect_err("normalized PMS truth must not be stored in memory");

            assert_eq!(
                error.code,
                crate::app_error::codes::AGENT_MEMORY_FORBIDDEN_TRUTH
            );
        }
    }
}
