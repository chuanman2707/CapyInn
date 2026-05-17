use super::{default_lock_key_deriver, stable_json_string, system_error};
use crate::{
    app_error::{codes, CommandError, CommandResult},
    outbox::OutboxEventSpec,
};
use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, future::Future, pin::Pin};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorType {
    Human,
    AiAgent,
    System,
    Integration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteCommandContext {
    pub request_id: String,
    pub idempotency_key: String,
    pub command_name: String,
    pub actor_id: Option<String>,
    pub actor_type: ActorType,
    pub client_id: Option<String>,
    pub session_id: Option<String>,
    pub channel_id: Option<String>,
    pub issued_at: DateTime<FixedOffset>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdempotentCommandResult<T> {
    pub response: T,
    pub replayed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LedgerAggregateRef {
    #[serde(rename = "type")]
    ref_type: String,
    id: String,
    label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandLedgerSummary {
    pub(super) label: String,
    pub(super) aggregate_refs: Vec<LedgerAggregateRef>,
    pub(super) business_dates: Vec<String>,
    pub(super) safe_fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SanitizedLedgerIntent {
    fields: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandLedgerResultSummary {
    label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandLedgerErrorSummary {
    code: String,
    kind: String,
    retryable: bool,
    message: String,
    support_id: Option<String>,
}

const SAFE_TEXT_MAX_CHARS: usize = 160;
const SENSITIVE_NUMERIC_MIN_DIGITS: usize = 10;
const FORBIDDEN_SAFE_FIELD_PARTS: &[&str] = &[
    "phone",
    "email",
    "payment",
    "card",
    "token",
    "secret",
    "password",
    "guest_note",
    "prompt",
    "raw",
    "payload",
];

impl CommandLedgerSummary {
    pub fn new(label: impl Into<String>) -> CommandResult<Self> {
        Ok(Self {
            label: validate_safe_text(label.into())?,
            aggregate_refs: Vec::new(),
            business_dates: Vec::new(),
            safe_fields: BTreeMap::new(),
        })
    }

    pub fn with_aggregate_ref(
        mut self,
        ref_type: impl Into<String>,
        id: impl Into<String>,
        label: Option<impl Into<String>>,
    ) -> CommandResult<Self> {
        self.aggregate_refs.push(LedgerAggregateRef {
            ref_type: validate_safe_key(ref_type.into())?,
            id: validate_safe_text(id.into())?,
            label: label.map(Into::into).map(validate_safe_text).transpose()?,
        });
        Ok(self)
    }

    pub fn with_business_date(mut self, business_date: impl Into<String>) -> CommandResult<Self> {
        self.business_dates
            .push(validate_safe_text(business_date.into())?);
        Ok(self)
    }

    pub fn with_safe_field(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> CommandResult<Self> {
        self.safe_fields.insert(
            validate_safe_key(key.into())?,
            validate_safe_text(value.into())?,
        );
        Ok(self)
    }

    pub fn to_value(&self) -> CommandResult<serde_json::Value> {
        serde_json::to_value(self.validated()?).map_err(system_error)
    }

    fn validated(&self) -> CommandResult<Self> {
        let mut summary = Self::new(self.label.clone())?;

        for aggregate_ref in &self.aggregate_refs {
            summary = summary.with_aggregate_ref(
                aggregate_ref.ref_type.clone(),
                aggregate_ref.id.clone(),
                aggregate_ref.label.clone(),
            )?;
        }

        for business_date in &self.business_dates {
            summary = summary.with_business_date(business_date.clone())?;
        }

        for (key, value) in &self.safe_fields {
            summary = summary.with_safe_field(key.clone(), value.clone())?;
        }

        Ok(summary)
    }
}

impl SanitizedLedgerIntent {
    pub fn from_pairs<K, V, I>(pairs: I) -> CommandResult<Self>
    where
        K: Into<String>,
        V: Into<serde_json::Value>,
        I: IntoIterator<Item = (K, V)>,
    {
        let mut fields = BTreeMap::new();
        for (key, value) in pairs {
            fields.insert(
                validate_safe_key(key.into())?,
                validate_safe_value(value.into())?,
            );
        }
        Ok(Self { fields })
    }

    pub fn to_value(&self) -> CommandResult<serde_json::Value> {
        serde_json::to_value(self.validated()?).map_err(system_error)
    }

    fn validated(&self) -> CommandResult<Self> {
        Self::from_pairs(
            self.fields
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        )
    }
}

impl CommandLedgerResultSummary {
    pub fn success(label: impl Into<String>) -> CommandResult<Self> {
        Ok(Self {
            label: validate_safe_text(label.into())?,
        })
    }

    #[allow(dead_code)]
    pub(super) fn to_json_string(&self) -> CommandResult<String> {
        let summary = Self::success(self.label.clone())?;
        stable_json_string(&serde_json::to_value(summary).map_err(system_error)?)
    }
}

impl CommandLedgerErrorSummary {
    #[allow(dead_code)]
    pub(super) fn from_error(error: &CommandError) -> Self {
        let message = validate_safe_text(error.message.clone())
            .unwrap_or_else(|_| "Command failed".to_string());
        let support_id = error
            .support_id
            .clone()
            .and_then(|support_id| validate_safe_text(support_id).ok());
        let code = validate_safe_key(error.code.clone())
            .unwrap_or_else(|_| codes::SYSTEM_INTERNAL_ERROR.to_string());
        let kind = serde_json::to_value(error.kind)
            .ok()
            .and_then(|value| value.as_str().map(ToString::to_string))
            .and_then(|kind| validate_safe_key(kind).ok())
            .unwrap_or_else(|| "system".to_string());

        Self {
            code,
            kind,
            retryable: error.retryable,
            message,
            support_id,
        }
    }

    #[allow(dead_code)]
    pub(super) fn to_json_string(&self) -> CommandResult<String> {
        let summary = Self {
            code: validate_safe_key(self.code.clone())?,
            kind: validate_safe_key(self.kind.clone())?,
            retryable: self.retryable,
            message: validate_safe_text(self.message.clone())?,
            support_id: self
                .support_id
                .clone()
                .map(validate_safe_text)
                .transpose()?,
        };
        stable_json_string(&serde_json::to_value(summary).map_err(system_error)?)
    }
}

fn validate_safe_key(value: String) -> CommandResult<String> {
    if value.is_empty() || contains_forbidden_safe_term(&value) {
        return Err(system_error(format!("unsafe ledger key: {value}")));
    }
    Ok(value)
}

fn validate_safe_text(value: String) -> CommandResult<String> {
    let trimmed = value.trim().to_string();
    if trimmed.len() > SAFE_TEXT_MAX_CHARS
        || trimmed.contains('@')
        || contains_sensitive_numeric_sequence(&trimmed)
        || contains_forbidden_safe_term(&trimmed)
    {
        return Err(system_error("unsafe ledger text"));
    }
    Ok(trimmed)
}

fn contains_forbidden_safe_term(value: &str) -> bool {
    let with_case_boundaries = split_case_boundaries(value);
    let lower = with_case_boundaries.to_ascii_lowercase();
    let parts = lower
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    let normalized = parts.join("_");
    let compact = parts.join("");

    parts
        .iter()
        .any(|part| FORBIDDEN_SAFE_FIELD_PARTS.contains(part))
        || FORBIDDEN_SAFE_FIELD_PARTS
            .iter()
            .filter(|part| part.contains('_'))
            .any(|part| normalized.contains(part))
        || FORBIDDEN_SAFE_FIELD_PARTS
            .iter()
            .filter(|part| !part.contains('_'))
            .any(|part| compact.contains(part))
}

fn split_case_boundaries(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut previous_was_lower_or_digit = false;

    for ch in value.chars() {
        if ch.is_ascii_uppercase() && previous_was_lower_or_digit {
            normalized.push('_');
        }
        normalized.push(ch);
        previous_was_lower_or_digit = ch.is_ascii_lowercase() || ch.is_ascii_digit();
    }

    normalized
}

fn contains_sensitive_numeric_sequence(value: &str) -> bool {
    value.chars().filter(|ch| ch.is_ascii_digit()).count() >= SENSITIVE_NUMERIC_MIN_DIGITS
}

fn validate_safe_value(value: serde_json::Value) -> CommandResult<serde_json::Value> {
    match value {
        serde_json::Value::String(text) => Ok(serde_json::Value::String(validate_safe_text(text)?)),
        serde_json::Value::Number(number) => {
            if contains_sensitive_numeric_sequence(&number.to_string()) {
                return Err(system_error("unsafe ledger value"));
            }
            Ok(serde_json::Value::Number(number))
        }
        serde_json::Value::Bool(_) | serde_json::Value::Null => Ok(value),
        serde_json::Value::Array(values) => values
            .into_iter()
            .map(validate_safe_value)
            .collect::<CommandResult<Vec<_>>>()
            .map(serde_json::Value::Array),
        serde_json::Value::Object(object) => {
            let mut safe = serde_json::Map::new();
            for (key, value) in object {
                safe.insert(validate_safe_key(key)?, validate_safe_value(value)?);
            }
            Ok(serde_json::Value::Object(safe))
        }
    }
}

pub type LockKeyDeriver = fn(&serde_json::Value) -> CommandResult<Vec<String>>;

pub type WriteCommandServiceFuture<'tx> =
    Pin<Box<dyn Future<Output = CommandResult<serde_json::Value>> + Send + 'tx>>;

#[derive(Debug)]
pub struct ResolvedWriteCommandGuard<T> {
    pub guard: T,
    pub lock_keys: Vec<String>,
}

impl<T> ResolvedWriteCommandGuard<T> {
    pub fn new<I, S>(guard: T, lock_keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            guard,
            lock_keys: lock_keys.into_iter().map(Into::into).collect(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct WriteCommandRequest {
    pub(super) hash_payload: serde_json::Value,
    pub(super) ledger_intent: SanitizedLedgerIntent,
    pub(super) summary: CommandLedgerSummary,
    pub(super) success_summary: CommandLedgerResultSummary,
    pub(super) primary_aggregate_key: Option<String>,
    pub(super) lock_key_deriver: LockKeyDeriver,
    pub(super) outbox_event: Option<OutboxEventSpec>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandStatus {
    InProgress,
    Completed,
    FailedRetryable,
    FailedTerminal,
}

impl CommandStatus {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::FailedRetryable => "failed_retryable",
            Self::FailedTerminal => "failed_terminal",
        }
    }

    pub(super) fn from_str(value: &str) -> CommandResult<Self> {
        match value {
            "in_progress" => Ok(Self::InProgress),
            "completed" => Ok(Self::Completed),
            "failed_retryable" => Ok(Self::FailedRetryable),
            "failed_terminal" => Ok(Self::FailedTerminal),
            _ => Err(system_error(format!(
                "unknown idempotency command status: {value}"
            ))),
        }
    }
}

impl WriteCommandRequest {
    pub fn new_sanitized(
        hash_payload: serde_json::Value,
        ledger_intent: SanitizedLedgerIntent,
        summary: CommandLedgerSummary,
    ) -> CommandResult<Self> {
        Ok(Self {
            hash_payload,
            ledger_intent: SanitizedLedgerIntent::from_pairs(
                ledger_intent
                    .fields
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            )?,
            summary: summary.validated()?,
            success_summary: CommandLedgerResultSummary::success("Command completed")?,
            primary_aggregate_key: None,
            lock_key_deriver: default_lock_key_deriver,
            outbox_event: None,
        })
    }

    pub fn new_low_risk(
        intent: serde_json::Value,
        label: impl Into<String>,
    ) -> CommandResult<Self> {
        let ledger_intent = sanitized_intent_from_value(intent.clone())?;
        let summary = CommandLedgerSummary::new(label)?;
        Self::new_sanitized(intent, ledger_intent, summary)
    }

    pub fn with_primary_aggregate_key(mut self, primary_aggregate_key: impl Into<String>) -> Self {
        self.primary_aggregate_key = Some(primary_aggregate_key.into());
        self
    }

    pub fn with_lock_key_deriver(mut self, lock_key_deriver: LockKeyDeriver) -> Self {
        self.lock_key_deriver = lock_key_deriver;
        self
    }

    pub fn with_success_summary(mut self, success_summary: CommandLedgerResultSummary) -> Self {
        self.success_summary = success_summary;
        self
    }

    pub fn with_outbox_event(mut self, outbox_event: OutboxEventSpec) -> Self {
        self.outbox_event = Some(outbox_event);
        self
    }
}

fn sanitized_intent_from_value(value: serde_json::Value) -> CommandResult<SanitizedLedgerIntent> {
    match value {
        serde_json::Value::Object(object) => SanitizedLedgerIntent::from_pairs(object),
        value => SanitizedLedgerIntent::from_pairs([("value", value)]),
    }
}

impl WriteCommandContext {
    pub fn for_scoped_command(
        request_id: impl Into<String>,
        idempotency_key: impl Into<String>,
        command_name: impl Into<String>,
    ) -> CommandResult<Self> {
        let request_id = request_id.into();
        let idempotency_key = idempotency_key.into().trim().to_string();
        if idempotency_key.is_empty() {
            return Err(CommandError::user(
                codes::IDEMPOTENCY_KEY_REQUIRED,
                "Idempotency key is required",
            )
            .with_request_id(request_id.clone()));
        }

        Ok(Self {
            request_id,
            idempotency_key,
            command_name: command_name.into(),
            actor_id: None,
            actor_type: ActorType::Human,
            client_id: None,
            session_id: None,
            channel_id: None,
            issued_at: chrono::Local::now().fixed_offset(),
        })
    }

    pub fn new_internal(command_name: &str) -> Self {
        Self {
            request_id: uuid::Uuid::new_v4().to_string(),
            idempotency_key: uuid::Uuid::new_v4().to_string(),
            command_name: command_name.to_string(),
            actor_id: None,
            actor_type: ActorType::System,
            client_id: None,
            session_id: None,
            channel_id: None,
            issued_at: chrono::Local::now().fixed_offset(),
        }
    }

    #[cfg(test)]
    pub fn for_internal_test(request_id: &str, idempotency_key: &str, command_name: &str) -> Self {
        let issued_at = DateTime::parse_from_rfc3339("2026-04-24T10:00:00+07:00")
            .expect("fixed test timestamp parses");

        Self {
            request_id: request_id.to_string(),
            idempotency_key: idempotency_key.to_string(),
            command_name: command_name.to_string(),
            actor_id: Some("test".to_string()),
            actor_type: ActorType::System,
            client_id: None,
            session_id: None,
            channel_id: None,
            issued_at,
        }
    }
}
