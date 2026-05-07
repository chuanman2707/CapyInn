use crate::{
    agent::{
        channel::telegram::TelegramTransport,
        digest::store::ClaimedDigestRun,
        provider::openai::{AiProvider, ProviderRequest, ProviderTurn},
        tools::ceo_read::dispatch_ceo_read_tool,
    },
    app_error::{codes, CommandError, CommandResult},
};
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

        let mut tool_results = Vec::with_capacity(CEO_DIGEST_TOOL_NAMES.len());
        let mut unavailable_tools = Vec::new();

        for tool_name in CEO_DIGEST_TOOL_NAMES {
            match dispatch_ceo_read_tool(&self.pool, tool_name, json!({})).await {
                Ok(envelope) => {
                    tool_results.push(serde_json::to_value(envelope).map_err(|_| {
                        CommandError::system(
                            codes::SYSTEM_INTERNAL_ERROR,
                            "Cannot serialize digest tool envelope.",
                        )
                    })?)
                }
                Err(_) => {
                    unavailable_tools.push((*tool_name).to_string());
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
            "unavailable_tools": unavailable_tools,
        });

        let reply = if unavailable_tools.len() == CEO_DIGEST_TOOL_NAMES.len() {
            "Không có đủ dữ liệu PMS được phép để gửi digest hiện tại.".to_string()
        } else {
            match self
                .provider
                .create_turn(ProviderRequest::new(
                    model,
                    CEO_DIGEST_SYSTEM_PROMPT,
                    payload.to_string(),
                    Vec::new(),
                ))
                .await?
            {
                ProviderTurn::FinalText(text) => text,
                ProviderTurn::ToolCalls { .. } => {
                    return Err(CommandError::user(
                        codes::AGENT_TOOL_NOT_ALLOWED,
                        "CEO digest summarizer cannot request tools.",
                    ));
                }
            }
        };

        self.telegram.send_message(chat_id, reply.clone()).await?;

        Ok(DigestDeliveryResult {
            reply_chars: reply.chars().count(),
            unavailable_tools,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{
        channel::telegram::{FakeTelegramTransport, TelegramUpdate},
        provider::openai::{ProviderRequest, ProviderTurn},
    };
    use serde_json::{Map, Number, Value};
    use sqlx::{
        sqlite::{SqlitePoolOptions, SqliteRow},
        Pool, Row, Sqlite, TypeInfo as _, ValueRef as _,
    };
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

    #[tokio::test]
    async fn digest_runtime_sends_fixed_tool_payload_to_provider_and_telegram() {
        let pool = test_pool().await;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let provider = RecordingProvider {
            requests: Arc::clone(&requests),
            turn: ProviderTurn::FinalText("Digest hiện tại ổn.".to_string()),
        };
        let telegram = FakeTelegramTransport::with_updates(Vec::<TelegramUpdate>::new());

        let result = CeoDigestRuntime::new(pool, provider, telegram)
            .deliver_digest(&claimed_run(), "gpt-test".to_string())
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
            .deliver_digest(&claimed_run(), "gpt-test".to_string())
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

        let payload: serde_json::Value =
            serde_json::from_str(&request.user).expect("provider payload is json");
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
        pool.close().await;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let provider = RecordingProvider {
            requests: Arc::clone(&requests),
            turn: ProviderTurn::FinalText("provider should not run".to_string()),
        };
        let telegram = RecordingTelegram::default();
        let sent_messages = Arc::clone(&telegram.sent_messages);

        let result = CeoDigestRuntime::new(pool, provider, telegram)
            .deliver_digest(&claimed_run(), "gpt-test".to_string())
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
        seed_digest_business_room(&pool).await;
        let before = business_table_snapshots(&pool).await;
        let provider = RecordingProvider {
            requests: Arc::new(Mutex::new(Vec::new())),
            turn: ProviderTurn::FinalText("Không có thay đổi dữ liệu PMS.".to_string()),
        };
        let telegram = RecordingTelegram::default();

        CeoDigestRuntime::new(pool.clone(), provider, telegram)
            .deliver_digest(&claimed_run(), "gpt-test".to_string())
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
        let requests = Arc::new(Mutex::new(Vec::new()));
        let provider = RecordingProvider {
            requests,
            turn: ProviderTurn::FinalText("Báo cáo đã sẵn sàng.".to_string()),
        };
        let telegram = RecordingTelegram::default();
        let sent_messages = Arc::clone(&telegram.sent_messages);

        CeoDigestRuntime::new(pool, provider, telegram)
            .deliver_digest(&claimed_run(), "gpt-test".to_string())
            .await
            .expect("deliver digest");

        assert_eq!(
            *sent_messages.lock().expect("sent message lock"),
            vec![(55, "Báo cáo đã sẵn sàng.".to_string())]
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
    }

    async fn business_table_snapshots(pool: &Pool<Sqlite>) -> Vec<(String, Vec<Value>)> {
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
                 'agent_digest_runs',
                 'command_idempotency',
                 'command_recovery_actions',
                 'outbox_events'
               )
             ORDER BY name ASC",
        )
        .fetch_all(pool)
        .await
        .expect("list tables");

        let mut snapshots = Vec::new();
        for row in rows {
            let table: String = row.get("name");
            let columns = table_column_names(pool, &table).await;
            let select_sql = format!("SELECT * FROM {}", quote_identifier(&table));
            let row_values = sqlx::query(&select_sql)
                .fetch_all(pool)
                .await
                .expect("snapshot table rows")
                .into_iter()
                .map(|row| sqlite_row_json(&row, &columns))
                .collect::<Vec<_>>();
            let mut serialized_rows = row_values
                .into_iter()
                .map(|value| serde_json::to_string(&value).expect("serialize snapshot row"))
                .collect::<Vec<_>>();
            serialized_rows.sort();
            let sorted_rows = serialized_rows
                .into_iter()
                .map(|value| serde_json::from_str(&value).expect("deserialize snapshot row"))
                .collect::<Vec<_>>();
            snapshots.push((table, sorted_rows));
        }
        snapshots
    }

    async fn table_column_names(pool: &Pool<Sqlite>, table: &str) -> Vec<String> {
        let pragma_sql = format!("PRAGMA table_info({})", quote_identifier(table));
        let mut columns = sqlx::query(&pragma_sql)
            .fetch_all(pool)
            .await
            .expect("list table columns")
            .into_iter()
            .map(|row| (row.get::<i64, _>("cid"), row.get::<String, _>("name")))
            .collect::<Vec<_>>();
        columns.sort_by_key(|(cid, _)| *cid);
        columns.into_iter().map(|(_, name)| name).collect()
    }

    fn sqlite_row_json(row: &SqliteRow, columns: &[String]) -> Value {
        let mut object = Map::new();
        for column in columns {
            object.insert(column.clone(), sqlite_cell_json(row, column));
        }
        Value::Object(object)
    }

    fn sqlite_cell_json(row: &SqliteRow, column: &str) -> Value {
        let raw = row.try_get_raw(column).expect("read raw sqlite value");
        if raw.is_null() {
            return Value::Null;
        }

        match raw.type_info().name() {
            "INTEGER" | "BOOLEAN" => Value::from(row.get::<i64, _>(column)),
            "REAL" => Number::from_f64(row.get::<f64, _>(column))
                .map(Value::Number)
                .unwrap_or(Value::Null),
            "TEXT" | "DATE" | "TIME" | "DATETIME" => Value::from(row.get::<String, _>(column)),
            "BLOB" => Value::Array(
                row.get::<Vec<u8>, _>(column)
                    .into_iter()
                    .map(Value::from)
                    .collect(),
            ),
            other => Value::from(format!("[unsupported sqlite type: {other}]")),
        }
    }

    fn quote_identifier(identifier: &str) -> String {
        format!("\"{}\"", identifier.replace('"', "\"\""))
    }
}
