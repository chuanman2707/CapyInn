use crate::app_error::{codes, CommandError, CommandResult};
use regex::Regex;
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
    fn keychain_account(self) -> &'static str {
        match self {
            Self::TelegramBotToken => "ceo_telegram_bot_token",
            Self::OpenAiApiKey => "ceo_openai_api_key",
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
        keyring::Entry::new(KEYCHAIN_SERVICE, kind.keychain_account()).map_err(map_keyring_error)
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
    let bearer = Regex::new(r"(?i)Bearer\s+\S+").expect("valid bearer redaction regex");
    let openai = Regex::new(r"(?i)\bsk-[A-Za-z0-9_-]+").expect("valid openai redaction regex");
    let assignment = Regex::new(
        r"(?i)\b(telegram_bot_token|openai_api_key|api_key|bot_token|token|secret|password)\s*=\s*[^\s,&;]+",
    )
    .expect("valid assignment redaction regex");

    let redacted = bearer.replace_all(value, "Bearer [redacted]");
    let redacted = openai.replace_all(&redacted, "[redacted]");
    assignment
        .replace_all(&redacted, "$1=[redacted]")
        .to_string()
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
}
