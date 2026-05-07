use crate::{
    agent::{
        channel::telegram::TelegramTransport,
        digest::{
            config::get_ceo_digest_config,
            store::{mark_digest_telegram_send_started, ClaimedDigestRun},
        },
        model::{AgentChannel, AgentProvider, AgentRole, DataSensitivity, MutationRisk},
        provider::openai::{AiProvider, ProviderRequest, ProviderTurn},
        retention::SESSION_RETENTION_METADATA_ONLY,
        secrets::redact_agent_secret_markers,
        store::{
            create_agent_session, insert_agent_audit_event, NewAgentAuditEvent, NewAgentSession,
        },
        tools::ceo_read::dispatch_ceo_read_tool,
    },
    app_error::{codes, CommandError, CommandResult},
};
use log::error;
use serde_json::json;
use sqlx::{Pool, Sqlite};

pub const CEO_DIGEST_TOOL_NAMES: &[&str] = &[
    "get_hotel_status",
    "list_room_status",
    "list_today_arrivals",
    "list_today_checkouts",
    "list_unpaid_balances",
    "get_revenue_snapshot",
    "get_audit_readiness",
    "summarize_operational_risks",
];

pub const CEO_DIGEST_SYSTEM_PROMPT: &str =
    "You are CapyInn CEO Secretary. Write a concise Vietnamese hourly digest from the provided JSON only. Mark unavailable sections clearly. Do not invent PMS facts.";

#[derive(Debug, Clone, PartialEq)]
pub struct DigestDeliveryResult {
    pub reply_chars: usize,
    pub unavailable_tools: Vec<String>,
}

pub struct CeoDigestRuntime<P, T> {
    pool: Pool<Sqlite>,
    provider: P,
    telegram: T,
}

impl<P, T> CeoDigestRuntime<P, T>
where
    P: AiProvider,
    T: TelegramTransport,
{
    pub fn new(pool: Pool<Sqlite>, provider: P, telegram: T) -> Self {
        Self {
            pool,
            provider,
            telegram,
        }
    }

    pub async fn deliver_digest(
        &self,
        run: &ClaimedDigestRun,
        model: String,
    ) -> CommandResult<DigestDeliveryResult> {
        let chat_id = run.delivery_chat_id.ok_or_else(|| {
            CommandError::user(
                codes::AGENT_RUNTIME_NOT_CONFIGURED,
                "CEO digest delivery chat ID is missing.",
            )
        })?;
        let session_id = self.create_digest_session(run).await?;
        self.audit(
            Some(&session_id),
            "ceo_digest.started",
            run.channel_actor_id.clone(),
            None,
            "allowed",
            None,
            json!({
                "digest_run_id": run.id,
                "due_at": run.due_at,
                "attempt_count": run.attempt_count,
                "max_attempts": run.max_attempts,
                "delivery_chat_id_present": true,
            }),
        )
        .await?;

        let mut tool_results = Vec::with_capacity(CEO_DIGEST_TOOL_NAMES.len());
        let mut unavailable_tools = Vec::new();

        for tool_name in CEO_DIGEST_TOOL_NAMES {
            match dispatch_ceo_read_tool(&self.pool, tool_name, json!({})).await {
                Ok(envelope) => {
                    self.audit(
                        Some(&session_id),
                        "ceo_digest.tool_read",
                        run.channel_actor_id.clone(),
                        None,
                        "allowed",
                        Some((*tool_name).to_string()),
                        json!({
                            "digest_run_id": run.id,
                            "due_at": run.due_at,
                            "tool_name": tool_name,
                            "tool_metadata": envelope.metadata,
                        }),
                    )
                    .await?;
                    tool_results.push(serde_json::to_value(envelope).map_err(|_| {
                        CommandError::system(
                            codes::SYSTEM_INTERNAL_ERROR,
                            "Cannot serialize digest tool envelope.",
                        )
                    })?)
                }
                Err(_) => {
                    unavailable_tools.push((*tool_name).to_string());
                    self.audit(
                        Some(&session_id),
                        "ceo_digest.tool_unavailable",
                        run.channel_actor_id.clone(),
                        None,
                        "allowed",
                        Some((*tool_name).to_string()),
                        json!({
                            "digest_run_id": run.id,
                            "due_at": run.due_at,
                            "tool_name": tool_name,
                            "unavailable": true,
                        }),
                    )
                    .await?;
                    tool_results.push(json!({
                        "ok": false,
                        "tool": tool_name,
                        "error": {
                            "message": "data unavailable",
                        },
                        "metadata": {
                            "unavailable": true,
                        },
                    }));
                }
            }
        }

        let payload = json!({
            "digest_kind": "ceo_hourly_digest",
            "due_at": run.due_at,
            "tools": tool_results,
            "unavailable_tools": &unavailable_tools,
        });

        let reply = if unavailable_tools.len() == CEO_DIGEST_TOOL_NAMES.len() {
            "Không có đủ dữ liệu PMS được phép để gửi digest hiện tại.".to_string()
        } else {
            match self
                .provider
                .create_turn(ProviderRequest::new(
                    model.clone(),
                    CEO_DIGEST_SYSTEM_PROMPT,
                    payload.to_string(),
                    Vec::new(),
                ))
                .await?
            {
                ProviderTurn::FinalText(text) => {
                    self.audit(
                        Some(&session_id),
                        "ceo_digest.provider_final",
                        run.channel_actor_id.clone(),
                        Some(AgentProvider::OpenAi),
                        "allowed",
                        None,
                        json!({
                            "digest_run_id": run.id,
                            "due_at": run.due_at,
                            "model": model,
                            "unavailable_tools": &unavailable_tools,
                            "reply_chars": text.chars().count(),
                        }),
                    )
                    .await?;
                    redact_agent_secret_markers(&text)
                }
                ProviderTurn::ToolCalls { .. } => {
                    self.audit(
                        Some(&session_id),
                        "ceo_digest.provider_tool_calls_rejected",
                        run.channel_actor_id.clone(),
                        Some(AgentProvider::OpenAi),
                        "denied",
                        None,
                        json!({
                            "digest_run_id": run.id,
                            "due_at": run.due_at,
                            "model": model,
                            "reason_code": codes::AGENT_TOOL_NOT_ALLOWED,
                        }),
                    )
                    .await?;
                    return Err(CommandError::user(
                        codes::AGENT_TOOL_NOT_ALLOWED,
                        "CEO digest summarizer cannot request tools.",
                    ));
                }
            }
        };

        let result = DigestDeliveryResult {
            reply_chars: reply.chars().count(),
            unavailable_tools,
        };
        self.ensure_current_delivery_target(run).await?;
        mark_digest_telegram_send_started(
            &self.pool,
            &run.id,
            &run.claim_token,
            json!({
                "telegram_send_started": true,
                "reply_char_count": result.reply_chars,
                "unavailable_tools": &result.unavailable_tools,
            }),
        )
        .await?;
        self.audit(
            Some(&session_id),
            "ceo_digest.telegram_send_started",
            run.channel_actor_id.clone(),
            None,
            "allowed",
            None,
            json!({
                "digest_run_id": run.id,
                "due_at": run.due_at,
                "delivery_chat_id_present": true,
                "reply_chars": result.reply_chars,
            }),
        )
        .await?;
        self.ensure_current_delivery_target(run).await?;

        self.telegram.send_message(chat_id, reply.clone()).await?;

        if let Err(audit_error) = self
            .audit(
                Some(&session_id),
                "ceo_digest.telegram_delivered",
                run.channel_actor_id.clone(),
                None,
                "allowed",
                None,
                json!({
                    "digest_run_id": run.id,
                    "due_at": run.due_at,
                    "delivery_chat_id_present": true,
                    "reply_chars": result.reply_chars,
                    "unavailable_tools": &result.unavailable_tools,
                }),
            )
            .await
        {
            error!(
                "Failed to audit CEO digest Telegram delivery after send: {} {}",
                audit_error.code, audit_error.message
            );
        }

        Ok(result)
    }

    async fn create_digest_session(&self, run: &ClaimedDigestRun) -> CommandResult<String> {
        let session_id = format!("ceo-digest-{}", uuid::Uuid::new_v4().simple());
        create_agent_session(
            &self.pool,
            NewAgentSession {
                id: session_id.clone(),
                role: AgentRole::CeoSecretary,
                channel: AgentChannel::Telegram,
                channel_actor_id: run.channel_actor_id.clone(),
                uses_memory: false,
                retention_policy: SESSION_RETENTION_METADATA_ONLY.to_string(),
                metadata: json!({
                    "source": "telegram_hourly_digest",
                    "digest_run_id": run.id,
                    "due_at": run.due_at,
                    "attempt_count": run.attempt_count,
                    "max_attempts": run.max_attempts,
                    "delivery_chat_id_present": run.delivery_chat_id.is_some(),
                }),
            },
        )
        .await?;
        Ok(session_id)
    }

    async fn ensure_current_delivery_target(&self, run: &ClaimedDigestRun) -> CommandResult<()> {
        let current = get_ceo_digest_config(&self.pool).await?;
        if current.telegram_user_id.as_deref() == run.channel_actor_id.as_deref()
            && current.telegram_delivery_chat_id == run.delivery_chat_id
        {
            return Ok(());
        }

        Err(CommandError::user(
            codes::AGENT_RUNTIME_NOT_CONFIGURED,
            "CEO digest delivery target changed before delivery.",
        ))
    }

    #[allow(clippy::too_many_arguments)]
    async fn audit(
        &self,
        session_id: Option<&str>,
        event_type: &str,
        actor_id: Option<String>,
        provider: Option<AgentProvider>,
        policy_outcome: &str,
        tool_name: Option<String>,
        summary: serde_json::Value,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{
        channel::telegram::{FakeTelegramTransport, TelegramUpdate},
        config::CEO_TELEGRAM_USER_ID_SETTING,
        digest::config::CEO_TELEGRAM_DELIVERY_CHAT_ID_SETTING,
        digest::store::{claim_due_digest_run, create_digest_run_if_absent, NewDigestRun},
        provider::openai::{ProviderRequest, ProviderTurn},
        test_support::phase_one_pms_table_snapshots as business_table_snapshots,
    };
    use chrono::{Duration, Local};
    use serde_json::Value;
    use sqlx::{sqlite::SqlitePoolOptions, Pool, Row, Sqlite};
    use std::{
        future::Future,
        pin::Pin,
        sync::{Arc, Mutex},
    };

    #[derive(Clone)]
    struct RecordingProvider {
        requests: Arc<Mutex<Vec<ProviderRequest>>>,
        turn: ProviderTurn,
    }

    #[derive(Clone, Default)]
    struct RecordingTelegram {
        sent_messages: Arc<Mutex<Vec<(i64, String)>>>,
    }

    #[derive(Clone)]
    struct TargetDriftProvider {
        pool: Pool<Sqlite>,
        requests: Arc<Mutex<Vec<ProviderRequest>>>,
        next_delivery_chat_id: i64,
    }

    impl AiProvider for RecordingProvider {
        fn create_turn<'a>(
            &'a self,
            request: ProviderRequest,
        ) -> Pin<Box<dyn Future<Output = CommandResult<ProviderTurn>> + Send + 'a>> {
            Box::pin(async move {
                self.requests.lock().expect("request lock").push(request);
                Ok(self.turn.clone())
            })
        }
    }

    impl AiProvider for TargetDriftProvider {
        fn create_turn<'a>(
            &'a self,
            request: ProviderRequest,
        ) -> Pin<Box<dyn Future<Output = CommandResult<ProviderTurn>> + Send + 'a>> {
            let pool = self.pool.clone();
            let requests = Arc::clone(&self.requests);
            let next_delivery_chat_id = self.next_delivery_chat_id;
            Box::pin(async move {
                requests.lock().expect("request lock").push(request);
                crate::services::settings_store::save_setting(
                    &pool,
                    CEO_TELEGRAM_DELIVERY_CHAT_ID_SETTING,
                    &next_delivery_chat_id.to_string(),
                )
                .await
                .expect("drift delivery chat setting");
                Ok(ProviderTurn::FinalText(
                    "Digest should not send".to_string(),
                ))
            })
        }
    }

    impl TelegramTransport for RecordingTelegram {
        fn get_updates<'a>(
            &'a self,
            _offset: Option<i64>,
        ) -> Pin<Box<dyn Future<Output = CommandResult<Vec<TelegramUpdate>>> + Send + 'a>> {
            Box::pin(async { Ok(Vec::new()) })
        }

        fn send_message<'a>(
            &'a self,
            chat_id: i64,
            text: String,
        ) -> Pin<Box<dyn Future<Output = CommandResult<()>> + Send + 'a>> {
            Box::pin(async move {
                self.sent_messages
                    .lock()
                    .expect("sent message lock")
                    .push((chat_id, text));
                Ok(())
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

    fn claimed_run() -> ClaimedDigestRun {
        ClaimedDigestRun {
            id: "digest-1".to_string(),
            channel_actor_id: Some("123".to_string()),
            delivery_chat_id: Some(55),
            due_at: "2026-05-07T01:00:00Z".to_string(),
            attempt_count: 1,
            max_attempts: 3,
            claim_token: "claim-1".to_string(),
        }
    }

    async fn persisted_claimed_run(pool: &Pool<Sqlite>, id: &str) -> ClaimedDigestRun {
        crate::services::settings_store::save_setting(pool, CEO_TELEGRAM_USER_ID_SETTING, "123")
            .await
            .expect("seed telegram user id");
        crate::services::settings_store::save_setting(
            pool,
            CEO_TELEGRAM_DELIVERY_CHAT_ID_SETTING,
            "55",
        )
        .await
        .expect("seed delivery chat id");
        create_digest_run_if_absent(
            pool,
            NewDigestRun {
                id: id.to_string(),
                channel_actor_id: Some("123".to_string()),
                delivery_chat_id: Some(55),
                due_at: "2026-05-07T01:00:00Z".to_string(),
                max_attempts: 3,
            },
        )
        .await
        .expect("create digest run");
        claim_due_digest_run(pool, "2026-05-07T01:00:01Z", &format!("claim-{id}"))
            .await
            .expect("claim digest run")
            .expect("claimed run")
    }

    async fn drop_digest_source_tables(pool: &Pool<Sqlite>) {
        sqlx::query("PRAGMA foreign_keys=OFF")
            .execute(pool)
            .await
            .expect("disable foreign keys for unavailable fixture");
        for table in [
            "rooms",
            "bookings",
            "transactions",
            "folio_lines",
            "expenses",
            "night_audit_logs",
        ] {
            let sql = format!("DROP TABLE IF EXISTS {table}");
            sqlx::query(&sql)
                .execute(pool)
                .await
                .unwrap_or_else(|error| panic!("drop {table}: {error}"));
        }
        sqlx::query("PRAGMA foreign_keys=ON")
            .execute(pool)
            .await
            .expect("restore foreign keys for unavailable fixture");
    }

    #[tokio::test]
    async fn digest_runtime_sends_fixed_tool_payload_to_provider_and_telegram() {
        let pool = test_pool().await;
        let run = persisted_claimed_run(&pool, "digest-payload").await;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let provider = RecordingProvider {
            requests: Arc::clone(&requests),
            turn: ProviderTurn::FinalText("Digest hiện tại ổn.".to_string()),
        };
        let telegram = FakeTelegramTransport::with_updates(Vec::<TelegramUpdate>::new());

        let result = CeoDigestRuntime::new(pool, provider, telegram)
            .deliver_digest(&run, "gpt-test".to_string())
            .await
            .expect("deliver digest");

        assert_eq!(result.reply_chars, "Digest hiện tại ổn.".chars().count());
        let request = requests
            .lock()
            .expect("request lock")
            .first()
            .cloned()
            .expect("request");
        assert_eq!(
            request.tools.len(),
            0,
            "digest summarizer must not expose model-selected tools"
        );
        for tool in CEO_DIGEST_TOOL_NAMES {
            assert!(request.user.contains(tool), "payload includes {tool}");
        }
    }

    #[tokio::test]
    async fn unavailable_tool_payload_does_not_expose_internal_error_details() {
        let pool = test_pool().await;
        let run = persisted_claimed_run(&pool, "digest-unavailable").await;
        sqlx::query("DROP TABLE rooms")
            .execute(&pool)
            .await
            .expect("make room-backed tools unavailable");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let provider = RecordingProvider {
            requests: Arc::clone(&requests),
            turn: ProviderTurn::FinalText("Digest có một phần thiếu dữ liệu.".to_string()),
        };
        let telegram = FakeTelegramTransport::with_updates(Vec::<TelegramUpdate>::new());

        let result = CeoDigestRuntime::new(pool, provider, telegram)
            .deliver_digest(&run, "gpt-test".to_string())
            .await
            .expect("deliver digest");

        assert!(
            !result.unavailable_tools.is_empty(),
            "test setup must force at least one unavailable tool"
        );
        let request = requests
            .lock()
            .expect("request lock")
            .first()
            .cloned()
            .expect("request");
        assert!(request.user.contains("data unavailable"));
        assert!(request.user.contains("\"unavailable\":true"));
        assert!(!request.user.contains(codes::SYSTEM_INTERNAL_ERROR));
        assert!(!request.user.contains("Cannot execute CEO read tool"));

        let payload: Value = serde_json::from_str(&request.user).expect("provider payload is json");
        let unavailable_entry = payload["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .find(|entry| entry["metadata"]["unavailable"] == true)
            .expect("unavailable tool entry");
        assert_eq!(unavailable_entry["error"]["message"], "data unavailable");
        assert!(
            unavailable_entry["error"].get("code").is_none(),
            "unavailable entries must not expose original error codes"
        );
    }

    #[tokio::test]
    async fn all_unavailable_tools_send_fallback_without_provider_call() {
        let pool = test_pool().await;
        let run = persisted_claimed_run(&pool, "digest-all-unavailable").await;
        drop_digest_source_tables(&pool).await;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let provider = RecordingProvider {
            requests: Arc::clone(&requests),
            turn: ProviderTurn::FinalText("provider should not run".to_string()),
        };
        let telegram = RecordingTelegram::default();
        let sent_messages = Arc::clone(&telegram.sent_messages);

        let result = CeoDigestRuntime::new(pool, provider, telegram)
            .deliver_digest(&run, "gpt-test".to_string())
            .await
            .expect("deliver fallback digest");

        assert!(requests.lock().expect("request lock").is_empty());
        assert_eq!(result.unavailable_tools.len(), CEO_DIGEST_TOOL_NAMES.len());
        assert_eq!(
            *sent_messages.lock().expect("sent message lock"),
            vec![(
                55,
                "Không có đủ dữ liệu PMS được phép để gửi digest hiện tại.".to_string()
            )]
        );
    }

    #[tokio::test]
    async fn missing_delivery_chat_id_fails_closed_before_provider_call() {
        let pool = test_pool().await;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let provider = RecordingProvider {
            requests: Arc::clone(&requests),
            turn: ProviderTurn::FinalText("should not run".to_string()),
        };
        let telegram = FakeTelegramTransport::with_updates(Vec::<TelegramUpdate>::new());
        let mut run = claimed_run();
        run.delivery_chat_id = None;

        let error = CeoDigestRuntime::new(pool, provider, telegram)
            .deliver_digest(&run, "gpt-test".to_string())
            .await
            .expect_err("missing chat id fails");

        assert_eq!(error.code, codes::AGENT_RUNTIME_NOT_CONFIGURED);
        assert!(requests.lock().expect("request lock").is_empty());
    }

    #[tokio::test]
    async fn digest_runtime_does_not_mutate_business_tables() {
        let pool = test_pool().await;
        let run = persisted_claimed_run(&pool, "digest-no-mutation").await;
        seed_digest_business_room(&pool).await;
        let before = business_table_snapshots(&pool).await;
        let provider = RecordingProvider {
            requests: Arc::new(Mutex::new(Vec::new())),
            turn: ProviderTurn::FinalText("Không có thay đổi dữ liệu PMS.".to_string()),
        };
        let telegram = RecordingTelegram::default();

        CeoDigestRuntime::new(pool.clone(), provider, telegram)
            .deliver_digest(&run, "gpt-test".to_string())
            .await
            .expect("deliver digest");
        let after = business_table_snapshots(&pool).await;

        assert_eq!(after, before);
    }

    #[tokio::test]
    async fn business_table_snapshots_detect_existing_row_updates() {
        let pool = test_pool().await;
        seed_digest_business_room(&pool).await;
        let before = business_table_snapshots(&pool).await;

        sqlx::query("UPDATE rooms SET status = 'cleaning' WHERE id = 'DIGEST-SNAPSHOT-ROOM'")
            .execute(&pool)
            .await
            .expect("mutate seeded room");
        let after = business_table_snapshots(&pool).await;

        assert_ne!(after, before);
    }

    #[tokio::test]
    async fn digest_snapshot_includes_outbox_and_agent_memory_tables() {
        let pool = test_pool().await;
        let snapshots = business_table_snapshots(&pool).await;
        let names = snapshots
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();

        assert!(names.contains(&"outbox_events".to_string()));
        assert!(names.contains(&"agent_memory_items".to_string()));
        assert!(names.contains(&"settings".to_string()));
        assert!(!names.contains(&"agent_sessions".to_string()));
        assert!(!names.contains(&"agent_audit_events".to_string()));
        assert!(!names.contains(&"agent_digest_runs".to_string()));
    }

    #[tokio::test]
    async fn digest_business_fixture_covers_digest_source_tables() {
        let pool = test_pool().await;
        seed_digest_business_room(&pool).await;
        let snapshots = business_table_snapshots(&pool).await;

        for table in [
            "rooms",
            "guests",
            "bookings",
            "transactions",
            "folio_lines",
            "expenses",
        ] {
            let (_, rows) = snapshots
                .iter()
                .find(|(name, _)| name == table)
                .unwrap_or_else(|| panic!("missing {table} snapshot"));
            assert!(!rows.is_empty(), "{table} fixture should not be empty");
        }
    }

    #[test]
    fn digest_tool_list_matches_phase_one_read_registry() {
        let registry_names = crate::agent::registry::CEO_PHASE_A_TOOLS
            .iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert_eq!(CEO_DIGEST_TOOL_NAMES, registry_names.as_slice());
    }

    #[tokio::test]
    async fn digest_runtime_sends_final_reply_to_delivery_chat() {
        let pool = test_pool().await;
        let run = persisted_claimed_run(&pool, "digest-final-reply").await;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let provider = RecordingProvider {
            requests,
            turn: ProviderTurn::FinalText("Báo cáo đã sẵn sàng.".to_string()),
        };
        let telegram = RecordingTelegram::default();
        let sent_messages = Arc::clone(&telegram.sent_messages);

        CeoDigestRuntime::new(pool, provider, telegram)
            .deliver_digest(&run, "gpt-test".to_string())
            .await
            .expect("deliver digest");

        assert_eq!(
            *sent_messages.lock().expect("sent message lock"),
            vec![(55, "Báo cáo đã sẵn sàng.".to_string())]
        );
    }

    #[tokio::test]
    async fn digest_final_reply_redacts_secret_like_markers_before_telegram_send() {
        let pool = test_pool().await;
        let run = persisted_claimed_run(&pool, "digest-redacted-reply").await;
        let provider = RecordingProvider {
            requests: Arc::new(Mutex::new(Vec::new())),
            turn: ProviderTurn::FinalText(
                "Digest không gửi sk-live-secret hoặc https://api.telegram.org/bot123456:ABC-secret/sendMessage".to_string(),
            ),
        };
        let telegram = RecordingTelegram::default();
        let sent_messages = Arc::clone(&telegram.sent_messages);

        CeoDigestRuntime::new(pool, provider, telegram)
            .deliver_digest(&run, "gpt-test".to_string())
            .await
            .expect("deliver digest");

        let messages = sent_messages.lock().expect("sent message lock");
        let (_, text) = messages.first().expect("sent digest message");
        assert!(!text.contains("sk-live-secret"));
        assert!(!text.contains("123456:ABC-secret"));
        assert!(text.contains("[redacted]"));
    }

    #[tokio::test]
    async fn digest_runtime_marks_telegram_send_started_before_returning_success() {
        let pool = test_pool().await;
        let run = persisted_claimed_run(&pool, "digest-send-started").await;
        let provider = RecordingProvider {
            requests: Arc::new(Mutex::new(Vec::new())),
            turn: ProviderTurn::FinalText("Báo cáo đã sẵn sàng.".to_string()),
        };
        let telegram = RecordingTelegram::default();

        CeoDigestRuntime::new(pool.clone(), provider, telegram)
            .deliver_digest(&run, "gpt-test".to_string())
            .await
            .expect("deliver digest");

        let row = sqlx::query(
            "SELECT status, delivery_summary_json
             FROM agent_digest_runs
             WHERE id = ?",
        )
        .bind(&run.id)
        .fetch_one(&pool)
        .await
        .expect("digest row");
        assert_eq!(row.get::<String, _>("status"), "in_progress");
        let summary: Value = serde_json::from_str(&row.get::<String, _>("delivery_summary_json"))
            .expect("delivery summary json");
        assert_eq!(summary["telegram_send_started"], true);
        assert_eq!(
            summary["reply_char_count"],
            "Báo cáo đã sẵn sàng.".chars().count()
        );
        assert_eq!(summary["unavailable_tools"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn digest_runtime_records_metadata_only_session_and_audit_events() {
        let pool = test_pool().await;
        let run = persisted_claimed_run(&pool, "digest-audit").await;
        let provider = RecordingProvider {
            requests: Arc::new(Mutex::new(Vec::new())),
            turn: ProviderTurn::FinalText("Báo cáo đã sẵn sàng.".to_string()),
        };
        let telegram = RecordingTelegram::default();

        CeoDigestRuntime::new(pool.clone(), provider, telegram)
            .deliver_digest(&run, "gpt-test".to_string())
            .await
            .expect("deliver digest");

        let session = sqlx::query(
            "SELECT role, channel, channel_actor_id, uses_memory, retention_policy, metadata_json
             FROM agent_sessions
             WHERE id LIKE 'ceo-digest-%'",
        )
        .fetch_one(&pool)
        .await
        .expect("digest session");
        assert_eq!(session.get::<String, _>("role"), "ceo_secretary");
        assert_eq!(session.get::<String, _>("channel"), "telegram");
        assert_eq!(
            session.get::<Option<String>, _>("channel_actor_id"),
            Some("123".to_string())
        );
        assert_eq!(session.get::<i64, _>("uses_memory"), 0);
        assert_eq!(
            session.get::<String, _>("retention_policy"),
            "metadata_only_v1"
        );
        let metadata: Value = serde_json::from_str(&session.get::<String, _>("metadata_json"))
            .expect("session metadata json");
        assert_eq!(metadata["digest_run_id"], "digest-audit");
        assert_eq!(metadata["delivery_chat_id_present"], true);
        assert!(metadata.get("chat_id").is_none());

        let events = sqlx::query(
            "SELECT event_type, provider, policy_outcome, summary_json
             FROM agent_audit_events
             ORDER BY id ASC",
        )
        .fetch_all(&pool)
        .await
        .expect("audit events");
        let event_types = events
            .iter()
            .map(|row| row.get::<String, _>("event_type"))
            .collect::<Vec<_>>();
        assert!(event_types.contains(&"ceo_digest.started".to_string()));
        assert!(event_types.contains(&"ceo_digest.provider_final".to_string()));
        assert!(event_types.contains(&"ceo_digest.telegram_send_started".to_string()));
        assert!(event_types.contains(&"ceo_digest.telegram_delivered".to_string()));
        assert!(
            events.iter().any(|row| {
                row.get::<Option<String>, _>("provider") == Some("open_ai".to_string())
                    && row.get::<String, _>("policy_outcome") == "allowed"
            }),
            "provider audit event should be metadata-only and allowed"
        );
        let serialized_events = events
            .iter()
            .map(|row| row.get::<String, _>("summary_json"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!serialized_events.contains("Báo cáo đã sẵn sàng."));
        assert!(!serialized_events.contains("raw_prompt"));
        assert!(!serialized_events.contains("raw_response"));
    }

    #[tokio::test]
    async fn digest_runtime_revalidates_delivery_target_after_provider_turn() {
        let pool = test_pool().await;
        let run = persisted_claimed_run(&pool, "digest-target-drift").await;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let provider = TargetDriftProvider {
            pool: pool.clone(),
            requests: Arc::clone(&requests),
            next_delivery_chat_id: 66,
        };
        let telegram = RecordingTelegram::default();
        let sent_messages = Arc::clone(&telegram.sent_messages);

        let error = CeoDigestRuntime::new(pool.clone(), provider, telegram)
            .deliver_digest(&run, "gpt-test".to_string())
            .await
            .expect_err("target drift fails before Telegram send");

        assert_eq!(error.code, codes::AGENT_RUNTIME_NOT_CONFIGURED);
        assert_eq!(requests.lock().expect("request lock").len(), 1);
        assert!(sent_messages.lock().expect("sent message lock").is_empty());

        let summary_json: String = sqlx::query_scalar(
            "SELECT delivery_summary_json
             FROM agent_digest_runs
             WHERE id = ?",
        )
        .bind(&run.id)
        .fetch_one(&pool)
        .await
        .expect("delivery summary");
        let summary: Value = serde_json::from_str(&summary_json).expect("delivery summary json");
        assert!(
            summary.get("telegram_send_started").is_none(),
            "send-started marker must not be written after target drift"
        );
    }

    #[tokio::test]
    async fn provider_tool_calls_are_rejected_without_telegram_delivery() {
        let pool = test_pool().await;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let provider = RecordingProvider {
            requests,
            turn: ProviderTurn::ToolCalls {
                calls: Vec::new(),
                response_items: Vec::new(),
            },
        };
        let telegram = RecordingTelegram::default();
        let sent_messages = Arc::clone(&telegram.sent_messages);

        let error = CeoDigestRuntime::new(pool, provider, telegram)
            .deliver_digest(&claimed_run(), "gpt-test".to_string())
            .await
            .expect_err("tool calls are rejected");

        assert_eq!(error.code, codes::AGENT_TOOL_NOT_ALLOWED);
        assert!(sent_messages.lock().expect("sent message lock").is_empty());
    }

    async fn seed_digest_business_room(pool: &Pool<Sqlite>) {
        let today = Local::now().date_naive();
        let yesterday = today - Duration::days(1);
        let tomorrow = today + Duration::days(1);
        let today = today.format("%Y-%m-%d").to_string();
        let yesterday = yesterday.format("%Y-%m-%d").to_string();
        let tomorrow = tomorrow.format("%Y-%m-%d").to_string();
        let created_at = format!("{today}T09:00:00+07:00");

        sqlx::query(
            "INSERT INTO rooms (
                id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status
             )
             VALUES (
                'DIGEST-SNAPSHOT-ROOM', 'Digest Snapshot Room', 'standard', 1, 0, 100000, 2, 0, 'vacant'
             )",
        )
        .execute(pool)
        .await
        .expect("seed digest business room");

        sqlx::query(
            "INSERT INTO guests (
                id, guest_type, full_name, doc_number, created_at
             )
             VALUES (
                'DIGEST-GUEST-1', 'domestic', 'Digest Visible Guest', 'DIGEST-DOC-1', ?
             )",
        )
        .bind(&created_at)
        .execute(pool)
        .await
        .expect("seed digest guest");

        sqlx::query(
            "INSERT INTO bookings (
                id, room_id, primary_guest_id, check_in_at, expected_checkout,
                actual_checkout, nights, total_price, paid_amount, status,
                source, notes, scheduled_checkin, scheduled_checkout, created_at
             )
             VALUES (
                'DIGEST-ARRIVAL-BOOKING', 'DIGEST-SNAPSHOT-ROOM', 'DIGEST-GUEST-1', ?, ?,
                NULL, 1, 200000, 50000, 'booked',
                'walk-in', 'digest arrival fixture', ?, ?, ?
             )",
        )
        .bind(&today)
        .bind(&tomorrow)
        .bind(&today)
        .bind(&tomorrow)
        .bind(&created_at)
        .execute(pool)
        .await
        .expect("seed digest arrival booking");

        sqlx::query(
            "INSERT INTO bookings (
                id, room_id, primary_guest_id, check_in_at, expected_checkout,
                actual_checkout, nights, total_price, paid_amount, status,
                source, notes, scheduled_checkin, scheduled_checkout, created_at
             )
             VALUES (
                'DIGEST-CHECKOUT-BOOKING', 'DIGEST-SNAPSHOT-ROOM', 'DIGEST-GUEST-1', ?, ?,
                NULL, 1, 300000, 100000, 'active',
                'walk-in', 'digest checkout fixture', ?, ?, ?
             )",
        )
        .bind(&yesterday)
        .bind(&today)
        .bind(&yesterday)
        .bind(&today)
        .bind(&created_at)
        .execute(pool)
        .await
        .expect("seed digest checkout booking");

        sqlx::query(
            "INSERT INTO transactions (
                id, booking_id, amount, type, created_at
             )
             VALUES (
                'DIGEST-TRANSACTION-1', 'DIGEST-CHECKOUT-BOOKING', 25000, 'cancellation_fee', ?
             )",
        )
        .bind(&created_at)
        .execute(pool)
        .await
        .expect("seed digest transaction");

        sqlx::query(
            "INSERT INTO folio_lines (
                id, booking_id, category, description, amount, created_at
             )
             VALUES (
                'DIGEST-FOLIO-1', 'DIGEST-CHECKOUT-BOOKING', 'service', 'Digest service line', 35000, ?
             )",
        )
        .bind(&created_at)
        .execute(pool)
        .await
        .expect("seed digest folio line");

        sqlx::query(
            "INSERT INTO expenses (
                id, category, amount, expense_date, created_at
             )
             VALUES (
                'DIGEST-EXPENSE-1', 'operations', 15000, ?, ?
             )",
        )
        .bind(&today)
        .bind(&created_at)
        .execute(pool)
        .await
        .expect("seed digest expense");
    }
}
