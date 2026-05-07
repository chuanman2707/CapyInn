use crate::app_error::{codes, CommandError, CommandResult};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

const KEYCHAIN_SERVICE: &str = "CapyInn CEO Agent";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AgentSecretKind {
    TelegramBotToken,
    OpenAiApiKey,
}

impl AgentSecretKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::TelegramBotToken => "ceo_telegram_bot_token",
            Self::OpenAiApiKey => "ceo_openai_api_key",
        }
    }

    pub fn idempotency_label(self) -> &'static str {
        match self {
            Self::TelegramBotToken => "telegram",
            Self::OpenAiApiKey => "openai",
        }
    }
}

pub trait AgentSecretStore: Send + Sync {
    fn get_secret(&self, kind: AgentSecretKind) -> CommandResult<Option<String>>;
    fn set_secret(&self, kind: AgentSecretKind, value: &str) -> CommandResult<()>;
    fn clear_secret(&self, kind: AgentSecretKind) -> CommandResult<()>;
}

#[derive(Default)]
pub struct FakeSecretStore {
    values: Arc<Mutex<HashMap<AgentSecretKind, String>>>,
}

impl AgentSecretStore for FakeSecretStore {
    fn get_secret(&self, kind: AgentSecretKind) -> CommandResult<Option<String>> {
        Ok(self
            .values
            .lock()
            .map_err(secret_lock_error)?
            .get(&kind)
            .cloned())
    }

    fn set_secret(&self, kind: AgentSecretKind, value: &str) -> CommandResult<()> {
        self.values
            .lock()
            .map_err(secret_lock_error)?
            .insert(kind, value.to_string());
        Ok(())
    }

    fn clear_secret(&self, kind: AgentSecretKind) -> CommandResult<()> {
        self.values.lock().map_err(secret_lock_error)?.remove(&kind);
        Ok(())
    }
}

pub struct KeychainSecretStore;

impl KeychainSecretStore {
    fn entry(kind: AgentSecretKind) -> CommandResult<keyring::Entry> {
        keyring::Entry::new(KEYCHAIN_SERVICE, kind.label()).map_err(map_keyring_error)
    }
}

impl AgentSecretStore for KeychainSecretStore {
    fn get_secret(&self, kind: AgentSecretKind) -> CommandResult<Option<String>> {
        match Self::entry(kind)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(map_keyring_error(error)),
        }
    }

    fn set_secret(&self, kind: AgentSecretKind, value: &str) -> CommandResult<()> {
        Self::entry(kind)?
            .set_password(value)
            .map_err(map_keyring_error)
    }

    fn clear_secret(&self, kind: AgentSecretKind) -> CommandResult<()> {
        match Self::entry(kind)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(map_keyring_error(error)),
        }
    }
}

fn secret_lock_error<T>(error: PoisonError<T>) -> CommandError {
    CommandError::system(
        codes::SYSTEM_INTERNAL_ERROR,
        format!("Agent secret store lock failed: {error}"),
    )
}

fn map_keyring_error(error: keyring::Error) -> CommandError {
    CommandError::system(
        codes::SYSTEM_INTERNAL_ERROR,
        format!(
            "Agent secret store operation failed: {}",
            redact_agent_secret_markers(&error.to_string())
        ),
    )
}

pub fn redact_agent_secret_markers(value: &str) -> String {
    let telegram_bot_url = Regex::new(r"(?i)\b(https?://api\.telegram\.org)/bot\d+:[^/\s]+/")
        .expect("valid telegram bot URL redaction regex");
    let bearer = Regex::new(r"(?i)Bearer\s+\S+").expect("valid bearer redaction regex");
    let openai = Regex::new(r"(?i)\bsk-[A-Za-z0-9_-]+").expect("valid openai redaction regex");
    let assignment = Regex::new(
        r"(?i)\b(telegram_bot_token|openai_api_key|api_key|bot_token|token|secret|password)\s*=\s*[^\s,&;]+",
    )
    .expect("valid assignment redaction regex");

    let redacted = telegram_bot_url.replace_all(value, "$1/bot[redacted]/");
    let redacted = bearer.replace_all(&redacted, "Bearer [redacted]");
    let redacted = openai.replace_all(&redacted, "[redacted]");
    assignment
        .replace_all(&redacted, "$1=[redacted]")
        .to_string()
}

pub fn agent_secret_fingerprint(kind: AgentSecretKind, value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(kind.label().as_bytes());
    hasher.update(b":");
    hasher.update(value.as_bytes());
    let digest = hasher.finalize();

    let mut fingerprint = String::with_capacity(digest.len() * 2);
    for byte in digest {
        fingerprint.push(char::from(b'a' + (byte >> 4)));
        fingerprint.push(char::from(b'a' + (byte & 0x0f)));
    }
    fingerprint
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_secret_store_round_trips_and_clears() {
        let store = FakeSecretStore::default();

        store
            .set_secret(AgentSecretKind::TelegramBotToken, "telegram-token")
            .expect("set secret");
        assert_eq!(
            store
                .get_secret(AgentSecretKind::TelegramBotToken)
                .expect("get secret")
                .as_deref(),
            Some("telegram-token")
        );

        store
            .clear_secret(AgentSecretKind::TelegramBotToken)
            .expect("clear secret");
        assert_eq!(
            store
                .get_secret(AgentSecretKind::TelegramBotToken)
                .expect("get after clear"),
            None
        );
    }

    #[test]
    fn redaction_removes_token_like_strings() {
        let redacted = redact_agent_secret_markers("Bearer sk-test and telegram_bot_token=abc");

        assert!(!redacted.contains("sk-test"));
        assert!(!redacted.contains("abc"));
        assert!(redacted.contains("[redacted]"));
    }

    #[test]
    fn redaction_removes_telegram_bot_url_tokens() {
        let redacted = redact_agent_secret_markers(
            "request failed for https://api.telegram.org/bot123456:ABC-secret/sendMessage and Bearer sk-test-secret",
        );

        assert!(!redacted.contains("123456:ABC-secret"));
        assert!(!redacted.contains("sk-test-secret"));
        assert!(redacted.contains("[redacted]"));
    }

    #[test]
    fn redaction_preserves_benign_botanical_urls() {
        let benign_url = "https://example.com/botanical/index.html";

        assert_eq!(redact_agent_secret_markers(benign_url), benign_url);
    }

    #[test]
    fn redaction_preserves_telegram_api_botanical_urls() {
        let benign_url = "https://api.telegram.org/botanical/index.html";

        assert_eq!(redact_agent_secret_markers(benign_url), benign_url);
    }

    #[test]
    fn secret_fingerprint_is_kind_scoped_and_one_way() {
        let token_fingerprint =
            agent_secret_fingerprint(AgentSecretKind::TelegramBotToken, "same-secret");
        let key_fingerprint =
            agent_secret_fingerprint(AgentSecretKind::OpenAiApiKey, "same-secret");

        assert_ne!(token_fingerprint, key_fingerprint);
        assert_eq!(token_fingerprint.len(), 64);
        assert!(!token_fingerprint.contains("same-secret"));
    }
}
