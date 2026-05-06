use crate::{
    agent::{
        config::CeoTelegramConfig,
        model::{AgentChannel, ChannelActor},
        secrets::{redact_agent_secret_markers, AgentSecretKind, AgentSecretStore},
    },
    app_error::{codes, CommandError, CommandResult},
};
use serde::Deserialize;
use serde_json::json;
use std::{future::Future, pin::Pin, time::Duration};

const TELEGRAM_API_BASE_URL: &str = "https://api.telegram.org";
const TELEGRAM_LONG_POLL_TIMEOUT_SECONDS: u64 = 25;
const TELEGRAM_HTTP_TIMEOUT_SECONDS: u64 = 35;
const TELEGRAM_MAX_RESPONSE_BYTES: u64 = 1024 * 1024;
const OWNER_DENIAL_PREFIX: &str = "Telegram ID ";
const OWNER_DENIAL_SUFFIX: &str =
    " is not paired with CapyInn CEO Chat. Ask an admin to bind this numeric ID.";
const MISSING_SENDER_DENIAL: &str = "Telegram sender is not paired with CapyInn CEO Chat. Ask an admin to bind this numeric ID once Telegram provides it.";

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramUpdate {
    pub update_id: i64,
    pub message: Option<TelegramMessage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramMessage {
    pub message_id: i64,
    pub from: Option<TelegramUser>,
    pub chat: TelegramChat,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramUser {
    pub id: i64,
    pub username: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TelegramChat {
    pub id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramRuntimeMessage {
    pub actor: ChannelActor,
    pub chat_id: i64,
    pub message_id: i64,
    pub text: String,
}

pub trait TelegramTransport: Send + Sync {
    fn get_updates<'a>(
        &'a self,
        offset: Option<i64>,
    ) -> Pin<Box<dyn Future<Output = CommandResult<Vec<TelegramUpdate>>> + Send + 'a>>;

    fn send_message<'a>(
        &'a self,
        chat_id: i64,
        text: String,
    ) -> Pin<Box<dyn Future<Output = CommandResult<()>> + Send + 'a>>;
}

pub trait TelegramMessageRuntime: Send + Sync {
    fn handle_message<'a>(
        &'a self,
        message: TelegramRuntimeMessage,
    ) -> Pin<Box<dyn Future<Output = CommandResult<String>> + Send + 'a>>;
}

pub struct HttpTelegramTransport<S: AgentSecretStore> {
    secrets: S,
    client: reqwest::Client,
    base_url: String,
}

impl<S: AgentSecretStore> HttpTelegramTransport<S> {
    pub fn new(secrets: S) -> CommandResult<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(TELEGRAM_HTTP_TIMEOUT_SECONDS))
            .build()
            .map_err(|error| scrub_telegram_error(&error.to_string(), None))?;

        Ok(Self {
            secrets,
            client,
            base_url: TELEGRAM_API_BASE_URL.to_string(),
        })
    }

    pub fn with_client(secrets: S, client: reqwest::Client, base_url: impl Into<String>) -> Self {
        Self {
            secrets,
            client,
            base_url: base_url.into(),
        }
    }

    fn bot_token(&self) -> CommandResult<String> {
        self.secrets
            .get_secret(AgentSecretKind::TelegramBotToken)?
            .ok_or_else(|| {
                CommandError::user(
                    codes::AGENT_SECRET_MISSING,
                    "Missing Telegram bot token for CEO Telegram Chat.",
                )
            })
    }

    fn method_url(&self, token: &str, method: &str) -> String {
        format!(
            "{}/bot{}/{}",
            self.base_url.trim_end_matches('/'),
            token,
            method
        )
    }

    async fn get_updates_inner(&self, offset: Option<i64>) -> CommandResult<Vec<TelegramUpdate>> {
        let token = self.bot_token()?;
        let body = match offset {
            Some(offset) => json!({
                "offset": offset,
                "timeout": TELEGRAM_LONG_POLL_TIMEOUT_SECONDS,
                "allowed_updates": ["message"],
            }),
            None => json!({
                "timeout": TELEGRAM_LONG_POLL_TIMEOUT_SECONDS,
                "allowed_updates": ["message"],
            }),
        };

        let response = self
            .client
            .post(self.method_url(&token, "getUpdates"))
            .json(&body)
            .send()
            .await
            .map_err(|error| scrub_telegram_error(&error.to_string(), Some(&token)))?;

        let status = response.status();
        if !status.is_success() {
            return Err(scrub_telegram_error(
                &format!("telegram getUpdates returned HTTP {status}"),
                Some(&token),
            ));
        }

        let bytes = limited_response_bytes(response, Some(&token)).await?;
        let response: TelegramApiResponse<Vec<TelegramUpdate>> = serde_json::from_slice(&bytes)
            .map_err(|error| scrub_telegram_error(&error.to_string(), Some(&token)))?;
        if response.ok {
            Ok(response.result.unwrap_or_default())
        } else {
            Err(scrub_telegram_error(
                response
                    .description
                    .as_deref()
                    .unwrap_or("telegram getUpdates failed"),
                Some(&token),
            ))
        }
    }

    async fn send_message_inner(&self, chat_id: i64, text: String) -> CommandResult<()> {
        let token = self.bot_token()?;
        let response = self
            .client
            .post(self.method_url(&token, "sendMessage"))
            .json(&json!({
                "chat_id": chat_id,
                "text": text,
            }))
            .send()
            .await
            .map_err(|error| scrub_telegram_error(&error.to_string(), Some(&token)))?;

        let status = response.status();
        if !status.is_success() {
            return Err(scrub_telegram_error(
                &format!("telegram sendMessage returned HTTP {status}"),
                Some(&token),
            ));
        }

        let bytes = limited_response_bytes(response, Some(&token)).await?;
        let response: TelegramApiResponse<serde_json::Value> = serde_json::from_slice(&bytes)
            .map_err(|error| scrub_telegram_error(&error.to_string(), Some(&token)))?;
        if response.ok {
            Ok(())
        } else {
            Err(scrub_telegram_error(
                response
                    .description
                    .as_deref()
                    .unwrap_or("telegram sendMessage failed"),
                Some(&token),
            ))
        }
    }
}

impl<S: AgentSecretStore> TelegramTransport for HttpTelegramTransport<S> {
    fn get_updates<'a>(
        &'a self,
        offset: Option<i64>,
    ) -> Pin<Box<dyn Future<Output = CommandResult<Vec<TelegramUpdate>>> + Send + 'a>> {
        Box::pin(async move { self.get_updates_inner(offset).await })
    }

    fn send_message<'a>(
        &'a self,
        chat_id: i64,
        text: String,
    ) -> Pin<Box<dyn Future<Output = CommandResult<()>> + Send + 'a>> {
        Box::pin(async move { self.send_message_inner(chat_id, text).await })
    }
}

pub async fn poll_once<T, R>(
    transport: &T,
    runtime: &R,
    config: &CeoTelegramConfig,
) -> CommandResult<Option<i64>>
where
    T: TelegramTransport,
    R: TelegramMessageRuntime,
{
    let offset = config.last_update_id.map(|id| id + 1);
    let updates = transport.get_updates(offset).await?;
    let mut highest_update_id: Option<i64> = None;

    for update in updates {
        highest_update_id = Some(
            highest_update_id.map_or(update.update_id, |highest| highest.max(update.update_id)),
        );

        let Some(message) = update.message else {
            continue;
        };
        let Some(text) = message.text else {
            continue;
        };
        let Some(sender) = message.from else {
            transport
                .send_message(message.chat.id, MISSING_SENDER_DENIAL.to_string())
                .await?;
            continue;
        };

        let sender_id = sender.id.to_string();
        if config.telegram_user_id.as_deref().map(str::trim) != Some(sender_id.as_str()) {
            transport
                .send_message(message.chat.id, owner_denial_message(sender.id))
                .await?;
            continue;
        }

        let reply = runtime
            .handle_message(TelegramRuntimeMessage {
                actor: actor_from_sender(&sender),
                chat_id: message.chat.id,
                message_id: message.message_id,
                text,
            })
            .await?;
        transport.send_message(message.chat.id, reply).await?;
    }

    Ok(highest_update_id)
}

#[derive(Debug, Deserialize)]
struct TelegramApiResponse<T> {
    ok: bool,
    result: Option<T>,
    description: Option<String>,
}

async fn limited_response_bytes(
    response: reqwest::Response,
    token: Option<&str>,
) -> CommandResult<Vec<u8>> {
    if let Some(length) = response.content_length() {
        if length > TELEGRAM_MAX_RESPONSE_BYTES {
            return Err(scrub_telegram_error(
                "telegram response was too large",
                token,
            ));
        }
    }

    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|error| scrub_telegram_error(&error.to_string(), token))
}

fn scrub_telegram_error(message: &str, token: Option<&str>) -> CommandError {
    let mut scrubbed = redact_agent_secret_markers(message);
    if let Some(token) = token {
        scrubbed = scrubbed.replace(token, "[redacted]");
    }
    CommandError::system(
        codes::SYSTEM_INTERNAL_ERROR,
        format!("Telegram channel request failed: {scrubbed}"),
    )
}

fn owner_denial_message(sender_id: i64) -> String {
    format!("{OWNER_DENIAL_PREFIX}{sender_id}{OWNER_DENIAL_SUFFIX}")
}

fn actor_from_sender(sender: &TelegramUser) -> ChannelActor {
    ChannelActor {
        channel: AgentChannel::Telegram,
        stable_actor_id: Some(sender.id.to_string()),
        display_name: display_name(sender),
        username: sender.username.clone(),
    }
}

fn display_name(sender: &TelegramUser) -> Option<String> {
    let name = [sender.first_name.as_deref(), sender.last_name.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    if name.trim().is_empty() {
        None
    } else {
        Some(name)
    }
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentTelegramMessage {
    pub chat_id: i64,
    pub text: String,
}

#[cfg(test)]
pub struct FakeTelegramTransport {
    updates: std::sync::Mutex<Vec<TelegramUpdate>>,
    sent_messages: std::sync::Mutex<Vec<SentTelegramMessage>>,
    requested_offsets: std::sync::Mutex<Vec<Option<i64>>>,
}

#[cfg(test)]
impl FakeTelegramTransport {
    pub fn with_updates(updates: Vec<TelegramUpdate>) -> Self {
        Self {
            updates: std::sync::Mutex::new(updates),
            sent_messages: std::sync::Mutex::new(Vec::new()),
            requested_offsets: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn sent_messages(&self) -> Vec<SentTelegramMessage> {
        self.sent_messages
            .lock()
            .expect("sent message lock")
            .clone()
    }

    pub fn requested_offsets(&self) -> Vec<Option<i64>> {
        self.requested_offsets
            .lock()
            .expect("requested offset lock")
            .clone()
    }
}

#[cfg(test)]
impl TelegramTransport for FakeTelegramTransport {
    fn get_updates<'a>(
        &'a self,
        offset: Option<i64>,
    ) -> Pin<Box<dyn Future<Output = CommandResult<Vec<TelegramUpdate>>> + Send + 'a>> {
        Box::pin(async move {
            self.requested_offsets
                .lock()
                .expect("requested offset lock")
                .push(offset);
            let minimum_update_id = offset.unwrap_or(i64::MIN);
            Ok(self
                .updates
                .lock()
                .expect("update lock")
                .iter()
                .filter(|update| update.update_id >= minimum_update_id)
                .cloned()
                .collect())
        })
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
                .push(SentTelegramMessage { chat_id, text });
            Ok(())
        })
    }
}

#[cfg(test)]
#[derive(Default)]
pub struct FakeTelegramMessageRuntime {
    replies: std::sync::Mutex<Vec<String>>,
    calls: std::sync::Mutex<Vec<TelegramRuntimeMessage>>,
}

#[cfg(test)]
impl FakeTelegramMessageRuntime {
    pub fn with_replies(replies: Vec<String>) -> Self {
        Self {
            replies: std::sync::Mutex::new(replies),
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn call_count(&self) -> usize {
        self.calls.lock().expect("runtime call lock").len()
    }

    pub fn calls(&self) -> Vec<TelegramRuntimeMessage> {
        self.calls.lock().expect("runtime call lock").clone()
    }
}

#[cfg(test)]
impl TelegramMessageRuntime for FakeTelegramMessageRuntime {
    fn handle_message<'a>(
        &'a self,
        message: TelegramRuntimeMessage,
    ) -> Pin<Box<dyn Future<Output = CommandResult<String>> + Send + 'a>> {
        Box::pin(async move {
            self.calls.lock().expect("runtime call lock").push(message);
            Ok(self
                .replies
                .lock()
                .expect("runtime reply lock")
                .pop()
                .unwrap_or_default())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{config::CeoTelegramConfig, secrets::FakeSecretStore};

    fn ready_config(owner_id: &str, last_update_id: Option<i64>) -> CeoTelegramConfig {
        CeoTelegramConfig {
            runtime_enabled: true,
            telegram_user_id: Some(owner_id.to_string()),
            telegram_bot_token_present: true,
            openai_api_key_present: true,
            openai_model: "gpt-5".to_string(),
            last_update_id,
        }
    }

    fn telegram_text_update(
        update_id: i64,
        sender_id: i64,
        chat_id: i64,
        text: &str,
    ) -> TelegramUpdate {
        TelegramUpdate {
            update_id,
            message: Some(TelegramMessage {
                message_id: update_id * 10,
                from: Some(TelegramUser {
                    id: sender_id,
                    username: Some(format!("user{sender_id}")),
                    first_name: Some("First".to_string()),
                    last_name: Some("Last".to_string()),
                }),
                chat: TelegramChat { id: chat_id },
                text: Some(text.to_string()),
            }),
        }
    }

    fn telegram_non_text_update(update_id: i64, sender_id: i64, chat_id: i64) -> TelegramUpdate {
        TelegramUpdate {
            update_id,
            message: Some(TelegramMessage {
                message_id: update_id * 10,
                from: Some(TelegramUser {
                    id: sender_id,
                    username: None,
                    first_name: None,
                    last_name: None,
                }),
                chat: TelegramChat { id: chat_id },
                text: None,
            }),
        }
    }

    fn telegram_text_update_without_sender(
        update_id: i64,
        chat_id: i64,
        text: &str,
    ) -> TelegramUpdate {
        TelegramUpdate {
            update_id,
            message: Some(TelegramMessage {
                message_id: update_id * 10,
                from: None,
                chat: TelegramChat { id: chat_id },
                text: Some(text.to_string()),
            }),
        }
    }

    #[tokio::test]
    async fn unknown_sender_gets_denial_without_runtime_call() {
        let transport = FakeTelegramTransport::with_updates(vec![telegram_text_update(
            10, 777, 55, "xin chao",
        )]);
        let runtime = FakeTelegramMessageRuntime::default();
        let config = ready_config("123", None);

        let result = poll_once(&transport, &runtime, &config)
            .await
            .expect("poll succeeds");

        assert_eq!(result, Some(10));
        assert_eq!(runtime.call_count(), 0);
        let sent_messages = transport.sent_messages();
        assert_eq!(sent_messages.len(), 1);
        assert_eq!(sent_messages[0].chat_id, 55);
        assert_eq!(
            sent_messages[0].text,
            "Telegram ID 777 is not paired with CapyInn CEO Chat. Ask an admin to bind this numeric ID."
        );
    }

    #[tokio::test]
    async fn missing_sender_gets_denial_without_runtime_call() {
        let transport =
            FakeTelegramTransport::with_updates(vec![telegram_text_update_without_sender(
                13, 55, "xin chao",
            )]);
        let runtime = FakeTelegramMessageRuntime::default();
        let config = ready_config("123", None);

        let result = poll_once(&transport, &runtime, &config)
            .await
            .expect("poll succeeds");

        assert_eq!(result, Some(13));
        assert_eq!(runtime.call_count(), 0);
        let sent_messages = transport.sent_messages();
        assert_eq!(sent_messages.len(), 1);
        assert_eq!(sent_messages[0].chat_id, 55);
        assert!(!sent_messages[0].text.contains("Telegram ID "));
        assert!(sent_messages[0].text.contains("not paired"));
        assert!(sent_messages[0].text.contains("bind this numeric ID"));
    }

    #[tokio::test]
    async fn paired_sender_calls_runtime_and_sends_reply() {
        let transport =
            FakeTelegramTransport::with_updates(vec![telegram_text_update(11, 123, 55, "status?")]);
        let runtime = FakeTelegramMessageRuntime::with_replies(vec!["All clear".to_string()]);
        let config = ready_config("123", None);

        let result = poll_once(&transport, &runtime, &config)
            .await
            .expect("poll succeeds");

        assert_eq!(result, Some(11));
        assert_eq!(runtime.call_count(), 1);
        let calls = runtime.calls();
        assert_eq!(calls[0].actor.stable_actor_id.as_deref(), Some("123"));
        assert_eq!(calls[0].actor.username.as_deref(), Some("user123"));
        assert_eq!(calls[0].actor.display_name.as_deref(), Some("First Last"));
        assert_eq!(calls[0].chat_id, 55);
        assert_eq!(calls[0].message_id, 110);
        assert_eq!(calls[0].text, "status?");
        assert_eq!(transport.sent_messages()[0].text, "All clear");
    }

    #[tokio::test]
    async fn ignores_non_text_updates_without_runtime_call_or_reply() {
        let transport =
            FakeTelegramTransport::with_updates(vec![telegram_non_text_update(12, 123, 55)]);
        let runtime = FakeTelegramMessageRuntime::default();
        let config = ready_config("123", None);

        let result = poll_once(&transport, &runtime, &config)
            .await
            .expect("poll succeeds");

        assert_eq!(result, Some(12));
        assert_eq!(runtime.call_count(), 0);
        assert!(transport.sent_messages().is_empty());
    }

    #[tokio::test]
    async fn returns_highest_processed_update_id_and_uses_offset() {
        let transport = FakeTelegramTransport::with_updates(vec![
            telegram_non_text_update(20, 123, 55),
            telegram_text_update(21, 123, 55, "hello"),
        ]);
        let runtime = FakeTelegramMessageRuntime::with_replies(vec!["reply".to_string()]);
        let config = ready_config("123", Some(19));

        let result = poll_once(&transport, &runtime, &config)
            .await
            .expect("poll succeeds");

        assert_eq!(result, Some(21));
        assert_eq!(transport.requested_offsets(), vec![Some(20)]);
        assert_eq!(runtime.call_count(), 1);
    }

    #[tokio::test]
    async fn http_transport_missing_token_returns_secret_missing() {
        let transport = HttpTelegramTransport::new(FakeSecretStore::default()).expect("transport");

        let error = transport
            .get_updates(None)
            .await
            .expect_err("missing bot token must fail closed");

        assert_eq!(error.code, crate::app_error::codes::AGENT_SECRET_MISSING);
    }

    #[test]
    fn telegram_errors_do_not_leak_bot_token() {
        let token = "123456:secret-token-value";

        let error = scrub_telegram_error(
            "request failed for https://api.telegram.org/bot123456:secret-token-value/getUpdates",
            Some(token),
        );

        assert!(!error.message.contains(token));
        assert!(error.message.contains("[redacted]"));
    }
}
