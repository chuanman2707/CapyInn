use crate::{
    agent::{
        config::{CeoTelegramConfig, CeoTelegramGateMissing},
        model::{
            AgentChannel, AgentProvider, AgentRole, ChannelActor, DataSensitivity, MutationRisk,
        },
        provider::openai::{AiProvider, ProviderRequest, ProviderToolOutput, ProviderTurn},
        retention::SESSION_RETENTION_METADATA_ONLY,
        store::{
            create_agent_session, insert_agent_audit_event, NewAgentAuditEvent, NewAgentSession,
        },
        tools::ceo_read::{ceo_read_tool_schemas, dispatch_ceo_read_tool},
    },
    app_error::{codes, CommandError, CommandResult},
};
use serde_json::{json, Value};
use sqlx::{Pool, Sqlite};
use std::collections::HashSet;

pub const CEO_TOOL_LOOP_MAX_ITERATIONS: usize = 4;
pub const DATA_UNAVAILABLE_MESSAGE: &str =
    "Không có đủ dữ liệu PMS được phép để trả lời câu hỏi này.";

const CEO_CHAT_SYSTEM_PROMPT: &str = "You are CapyInn CEO Secretary. Answer in Vietnamese. Use only provided CEO PMS read tools for PMS facts. If no allowed tool result supports an answer, say data is unavailable.";

#[derive(Debug, Clone)]
pub struct CeoChatMessage {
    pub actor: ChannelActor,
    pub chat_id: i64,
    pub text: String,
    pub config: CeoTelegramConfig,
    pub ceo_cloud_data_opt_in: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CeoChatReply {
    pub text: String,
    pub tools_called: Vec<String>,
    pub termination_reason: String,
}

pub struct CeoChatRuntime<P> {
    pool: Pool<Sqlite>,
    provider: P,
}

impl<P> CeoChatRuntime<P>
where
    P: AiProvider,
{
    pub fn new(pool: Pool<Sqlite>, provider: P) -> Self {
        Self { pool, provider }
    }

    pub async fn handle_message(&self, message: CeoChatMessage) -> CommandResult<CeoChatReply> {
        let gate_result = self.evaluate_gate(&message);
        let session_id = format!("ceo-chat-{}", uuid::Uuid::new_v4().simple());
        create_agent_session(
            &self.pool,
            NewAgentSession {
                id: session_id.clone(),
                role: AgentRole::CeoSecretary,
                channel: AgentChannel::Telegram,
                channel_actor_id: message.actor.stable_actor_id.clone(),
                uses_memory: false,
                retention_policy: SESSION_RETENTION_METADATA_ONLY.to_string(),
                metadata: json!({
                    "chat_id": message.chat_id,
                    "source": "telegram_read_only_chat",
                }),
            },
        )
        .await?;

        self.audit(
            Some(&session_id),
            "ceo_chat.message_received",
            message.actor.stable_actor_id.clone(),
            None,
            "received",
            None,
            json!({
                "chat_id": message.chat_id,
                "source": "telegram_read_only_chat",
                "message_chars": message.text.chars().count(),
            }),
        )
        .await?;

        if let Err(error) = gate_result {
            self.audit(
                Some(&session_id),
                "ceo_chat.denied",
                message.actor.stable_actor_id.clone(),
                None,
                "denied",
                None,
                json!({
                    "chat_id": message.chat_id,
                    "source": "telegram_read_only_chat",
                    "reason_code": error.code,
                }),
            )
            .await?;
            return Err(error);
        }

        match self.run_provider_loop(&session_id, &message).await {
            Ok(reply) => Ok(reply),
            Err(error) => {
                self.audit(
                    Some(&session_id),
                    "ceo_chat.error",
                    message.actor.stable_actor_id.clone(),
                    Some(AgentProvider::OpenAi),
                    "error",
                    None,
                    json!({
                        "chat_id": message.chat_id,
                        "source": "telegram_read_only_chat",
                        "reason_code": error.code,
                    }),
                )
                .await?;
                Err(error)
            }
        }
    }

    fn evaluate_gate(&self, message: &CeoChatMessage) -> CommandResult<()> {
        let gate = message.config.evaluate_gate(message.ceo_cloud_data_opt_in);
        if let Some(missing) = gate.missing.first() {
            return Err(gate_error(missing));
        }

        let actor_id = message.actor.stable_actor_id.as_deref().map(str::trim);
        let owner_id = message.config.telegram_user_id.as_deref().map(str::trim);
        if actor_id != owner_id {
            return Err(CommandError::user(
                codes::AGENT_TELEGRAM_USER_DENIED,
                "Telegram user is not allowed to use CEO chat.",
            ));
        }

        Ok(())
    }

    async fn run_provider_loop(
        &self,
        session_id: &str,
        message: &CeoChatMessage,
    ) -> CommandResult<CeoChatReply> {
        let mut response_items = Vec::new();
        let mut tool_outputs = Vec::new();
        let mut seen_tool_calls = HashSet::new();
        let mut tools_called = Vec::new();

        for _ in 0..CEO_TOOL_LOOP_MAX_ITERATIONS {
            let request = ProviderRequest::new(
                message.config.openai_model.clone(),
                CEO_CHAT_SYSTEM_PROMPT,
                message.text.clone(),
                ceo_read_tool_schemas(),
            )
            .with_response_items(response_items)
            .with_tool_outputs(tool_outputs);

            match self.provider.create_turn(request).await? {
                ProviderTurn::FinalText(text) => {
                    let reply = CeoChatReply {
                        text,
                        tools_called,
                        termination_reason: "final_text".to_string(),
                    };
                    self.audit(
                        Some(session_id),
                        "ceo_chat.final_reply",
                        message.actor.stable_actor_id.clone(),
                        Some(AgentProvider::OpenAi),
                        "allowed",
                        None,
                        json!({
                            "chat_id": message.chat_id,
                            "source": "telegram_read_only_chat",
                            "termination_reason": reply.termination_reason,
                            "tools_called": reply.tools_called,
                            "reply_chars": reply.text.chars().count(),
                            "data_unavailable": reply.text.trim() == DATA_UNAVAILABLE_MESSAGE,
                        }),
                    )
                    .await?;
                    return Ok(reply);
                }
                ProviderTurn::ToolCalls {
                    calls,
                    response_items: next_response_items,
                } => {
                    response_items = next_response_items;
                    tool_outputs = Vec::with_capacity(calls.len());

                    for call in calls {
                        let canonical_args = canonical_json(&call.arguments);
                        if !seen_tool_calls.insert((call.name.clone(), canonical_args.clone())) {
                            return Err(tool_loop_limit_error());
                        }

                        let envelope =
                            dispatch_ceo_read_tool(&self.pool, &call.name, call.arguments).await?;
                        self.audit(
                            Some(session_id),
                            "ceo_chat.tool_called",
                            message.actor.stable_actor_id.clone(),
                            Some(AgentProvider::OpenAi),
                            "allowed",
                            Some(call.name.clone()),
                            json!({
                                "chat_id": message.chat_id,
                                "source": "telegram_read_only_chat",
                                "tool_name": call.name,
                                "args_hash": stable_hash(&canonical_args),
                                "tool_metadata": envelope.metadata,
                            }),
                        )
                        .await?;
                        tools_called.push(envelope.tool.clone());
                        tool_outputs.push(ProviderToolOutput {
                            call_id: call.call_id,
                            output: serde_json::to_value(envelope).map_err(|_| {
                                CommandError::system(
                                    codes::SYSTEM_INTERNAL_ERROR,
                                    "Cannot serialize CEO tool output.",
                                )
                            })?,
                        });
                    }
                }
            }
        }

        Err(tool_loop_limit_error())
    }

    async fn audit(
        &self,
        session_id: Option<&str>,
        event_type: &str,
        actor_id: Option<String>,
        provider: Option<AgentProvider>,
        policy_outcome: &str,
        tool_name: Option<String>,
        summary: Value,
    ) -> CommandResult<()> {
        insert_agent_audit_event(
            &self.pool,
            NewAgentAuditEvent {
                session_id: session_id.map(str::to_string),
                event_type: event_type.to_string(),
                actor_id,
                role: Some(AgentRole::CeoSecretary),
                channel: Some(AgentChannel::Telegram),
                tool_name,
                provider,
                policy_outcome: policy_outcome.to_string(),
                mutation_risk: Some(MutationRisk::ReadOnly),
                data_sensitivity: Some(DataSensitivity::CeoSensitive),
                summary,
            },
        )
        .await
    }
}

fn gate_error(missing: &CeoTelegramGateMissing) -> CommandError {
    match missing {
        CeoTelegramGateMissing::CloudDataOptIn => CommandError::user(
            codes::AGENT_CLOUD_DATA_OPT_IN_REQUIRED,
            "CEO cloud-data opt-in is required.",
        ),
        CeoTelegramGateMissing::RuntimeEnabled => CommandError::user(
            codes::AGENT_RUNTIME_DISABLED,
            "CEO Telegram chat runtime is disabled.",
        ),
        CeoTelegramGateMissing::TelegramOwnerBinding => CommandError::user(
            codes::AGENT_TELEGRAM_OWNER_NOT_BOUND,
            "CEO Telegram owner is not bound.",
        ),
        CeoTelegramGateMissing::TelegramBotToken | CeoTelegramGateMissing::OpenAiApiKey => {
            CommandError::user(
                codes::AGENT_SECRET_MISSING,
                "CEO Telegram chat credential is missing.",
            )
        }
    }
}

fn tool_loop_limit_error() -> CommandError {
    CommandError::user(
        codes::AGENT_TOOL_LOOP_LIMIT,
        "CEO chat tool loop limit reached.",
    )
}

fn stable_hash(value: &str) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(value.as_bytes());
    format!("{digest:x}")
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).expect("serialize string"),
        Value::Array(values) => {
            let items = values.iter().map(canonical_json).collect::<Vec<_>>();
            format!("[{}]", items.join(","))
        }
        Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            let items = entries
                .into_iter()
                .map(|(key, child)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("serialize key"),
                        canonical_json(child)
                    )
                })
                .collect::<Vec<_>>();
            format!("{{{}}}", items.join(","))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        agent::{
            config::CeoTelegramConfig,
            model::{AgentChannel, ChannelActor},
            provider::openai::{AiProvider, ProviderRequest, ProviderToolCall, ProviderTurn},
        },
        app_error::{codes, CommandError, CommandResult},
    };
    use serde_json::{json, Value};
    use sqlx::{sqlite::SqlitePoolOptions, Pool, Row, Sqlite};
    use std::{
        future::Future,
        pin::Pin,
        sync::{Arc, Mutex},
    };

    #[derive(Clone)]
    struct RecordingProvider {
        turns: Arc<Mutex<Vec<Result<ProviderTurn, CommandError>>>>,
        requests: Arc<Mutex<Vec<ProviderRequest>>>,
    }

    impl RecordingProvider {
        fn new(turns: Vec<Result<ProviderTurn, CommandError>>) -> Self {
            Self {
                turns: Arc::new(Mutex::new(turns)),
                requests: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn request_count(&self) -> usize {
            self.requests.lock().expect("requests lock").len()
        }
    }

    impl AiProvider for RecordingProvider {
        fn create_turn<'a>(
            &'a self,
            request: ProviderRequest,
        ) -> Pin<Box<dyn Future<Output = CommandResult<ProviderTurn>> + Send + 'a>> {
            Box::pin(async move {
                self.requests.lock().expect("requests lock").push(request);
                self.turns.lock().expect("turns lock").remove(0)
            })
        }
    }

    async fn test_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect sqlite");
        crate::db::run_migrations(&pool).await.expect("migrations");
        pool
    }

    fn ready_config() -> CeoTelegramConfig {
        CeoTelegramConfig {
            runtime_enabled: true,
            telegram_user_id: Some("12345".to_string()),
            telegram_bot_token_present: true,
            openai_api_key_present: true,
            openai_model: "gpt-test".to_string(),
            last_update_id: None,
        }
    }

    fn message(config: CeoTelegramConfig) -> CeoChatMessage {
        CeoChatMessage {
            actor: ChannelActor {
                channel: AgentChannel::Telegram,
                stable_actor_id: Some("12345".to_string()),
                display_name: Some("CEO".to_string()),
                username: Some("ceo".to_string()),
            },
            chat_id: 42,
            text: "Doanh thu hôm nay thế nào?".to_string(),
            config,
            ceo_cloud_data_opt_in: true,
        }
    }

    fn tool_call(name: &str, arguments: Value) -> ProviderToolCall {
        ProviderToolCall {
            call_id: format!("call-{name}"),
            name: name.to_string(),
            arguments,
        }
    }

    #[tokio::test]
    async fn missing_gate_prevents_provider_and_tools() {
        let pool = test_pool().await;
        let provider = RecordingProvider::new(vec![Ok(ProviderTurn::FinalText(
            "should not run".to_string(),
        ))]);
        let mut config = ready_config();
        config.openai_api_key_present = false;

        let error = CeoChatRuntime::new(pool.clone(), provider.clone())
            .handle_message(message(config))
            .await
            .expect_err("secret gate must deny before provider");

        assert_eq!(error.code, codes::AGENT_SECRET_MISSING);
        assert_eq!(provider.request_count(), 0);

        let events: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_audit_events WHERE event_type = 'ceo_chat.denied'",
        )
        .fetch_one(&pool)
        .await
        .expect("count audit events");
        assert_eq!(events, 1);
    }

    #[tokio::test]
    async fn paired_ceo_tool_call_returns_final_answer_and_tools_called() {
        let pool = test_pool().await;
        let provider = RecordingProvider::new(vec![
            Ok(ProviderTurn::ToolCalls {
                calls: vec![tool_call("get_hotel_status", json!({}))],
                response_items: vec![
                    json!({"type": "function_call", "call_id": "call-get_hotel_status"}),
                ],
            }),
            Ok(ProviderTurn::FinalText(
                "Hôm nay khách sạn đang ổn.".to_string(),
            )),
        ]);

        let reply = CeoChatRuntime::new(pool.clone(), provider.clone())
            .handle_message(message(ready_config()))
            .await
            .expect("chat succeeds");

        assert_eq!(reply.text, "Hôm nay khách sạn đang ổn.");
        assert_eq!(reply.tools_called, vec!["get_hotel_status"]);
        assert_eq!(reply.termination_reason, "final_text");
        assert_eq!(provider.request_count(), 2);

        let tool_events: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM agent_audit_events WHERE event_type = 'ceo_chat.tool_called'",
        )
        .fetch_one(&pool)
        .await
        .expect("count tool audits");
        assert_eq!(tool_events, 1);
    }

    #[tokio::test]
    async fn repeated_identical_tool_call_stops_loop() {
        let pool = test_pool().await;
        let provider = RecordingProvider::new(vec![
            Ok(ProviderTurn::ToolCalls {
                calls: vec![tool_call("get_hotel_status", json!({}))],
                response_items: vec![json!({"type": "function_call", "call_id": "call-1"})],
            }),
            Ok(ProviderTurn::ToolCalls {
                calls: vec![tool_call("get_hotel_status", json!({}))],
                response_items: vec![json!({"type": "function_call", "call_id": "call-2"})],
            }),
        ]);

        let error = CeoChatRuntime::new(pool, provider)
            .handle_message(message(ready_config()))
            .await
            .expect_err("duplicate tool call must stop loop");

        assert_eq!(error.code, codes::AGENT_TOOL_LOOP_LIMIT);
    }

    #[tokio::test]
    async fn final_data_unavailable_message_returns_no_tools() {
        let pool = test_pool().await;
        let provider = RecordingProvider::new(vec![Ok(ProviderTurn::FinalText(
            DATA_UNAVAILABLE_MESSAGE.to_string(),
        ))]);

        let reply = CeoChatRuntime::new(pool, provider)
            .handle_message(message(ready_config()))
            .await
            .expect("final data-unavailable reply succeeds");

        assert_eq!(reply.text, DATA_UNAVAILABLE_MESSAGE);
        assert!(reply.tools_called.is_empty());
        assert_eq!(reply.termination_reason, "final_text");
    }

    #[tokio::test]
    async fn business_table_counts_are_unchanged_across_mocked_chat_turn() {
        let pool = test_pool().await;
        let before = business_table_counts(&pool).await;
        let provider = RecordingProvider::new(vec![
            Ok(ProviderTurn::ToolCalls {
                calls: vec![tool_call("list_room_status", json!({}))],
                response_items: vec![
                    json!({"type": "function_call", "call_id": "call-list_room_status"}),
                ],
            }),
            Ok(ProviderTurn::FinalText(
                "Không có thay đổi dữ liệu PMS.".to_string(),
            )),
        ]);

        let _reply = CeoChatRuntime::new(pool.clone(), provider)
            .handle_message(message(ready_config()))
            .await
            .expect("chat succeeds");
        let after = business_table_counts(&pool).await;

        assert_eq!(after, before);
    }

    async fn business_table_counts(pool: &Pool<Sqlite>) -> Vec<(String, i64)> {
        let rows = sqlx::query(
            "SELECT name FROM sqlite_master
             WHERE type = 'table'
               AND name NOT LIKE 'sqlite_%'
               AND name NOT IN (
                 'schema_version',
                 'settings',
                 'agent_sessions',
                 'agent_audit_events',
                 'agent_memory_items',
                 'command_idempotency',
                 'command_recovery_actions',
                 'outbox_events'
               )
             ORDER BY name ASC",
        )
        .fetch_all(pool)
        .await
        .expect("list tables");

        let mut counts = Vec::new();
        for row in rows {
            let table: String = row.get("name");
            let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(pool)
                .await
                .expect("count table rows");
            counts.push((table, count));
        }
        counts
    }
}
