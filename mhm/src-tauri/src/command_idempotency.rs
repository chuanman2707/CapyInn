use crate::{
    app_error::{codes, CommandError, CommandResult},
    services::settings_store,
};
use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{sqlite::SqliteRow, Pool, Row, Sqlite, Transaction};
use std::{collections::BTreeMap, future::Future, pin::Pin};

pub const SET_CRASH_REPORTING_PREFERENCE_COMMAND: &str = "settings.set_crash_reporting_preference";
const CLAIM_LEASE_SECONDS: i64 = 30;
#[cfg(test)]
const CLAIM_LEASE_REFRESH_SECONDS: u64 = 1;
#[cfg(not(test))]
const CLAIM_LEASE_REFRESH_SECONDS: u64 = 15;
#[cfg(test)]
const CLAIM_LEASE_REFRESH_TIMEOUT_SECONDS: u64 = 1;
#[cfg(not(test))]
const CLAIM_LEASE_REFRESH_TIMEOUT_SECONDS: u64 = 2;

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
    label: String,
    aggregate_refs: Vec<LedgerAggregateRef>,
    business_dates: Vec<String>,
    safe_fields: BTreeMap<String, String>,
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
    fn to_json_string(&self) -> CommandResult<String> {
        let summary = Self::success(self.label.clone())?;
        stable_json_string(&serde_json::to_value(summary).map_err(system_error)?)
    }
}

impl CommandLedgerErrorSummary {
    #[allow(dead_code)]
    fn from_error(error: &CommandError) -> Self {
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
    fn to_json_string(&self) -> CommandResult<String> {
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
    hash_payload: serde_json::Value,
    ledger_intent: SanitizedLedgerIntent,
    summary: CommandLedgerSummary,
    success_summary: CommandLedgerResultSummary,
    primary_aggregate_key: Option<String>,
    lock_key_deriver: LockKeyDeriver,
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
    fn as_str(self) -> &'static str {
        match self {
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::FailedRetryable => "failed_retryable",
            Self::FailedTerminal => "failed_terminal",
        }
    }

    fn from_str(value: &str) -> CommandResult<Self> {
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
}

pub fn default_lock_key_deriver(_intent: &serde_json::Value) -> CommandResult<Vec<String>> {
    Ok(Vec::new())
}

fn actor_type_as_str(actor_type: ActorType) -> &'static str {
    match actor_type {
        ActorType::Human => "human",
        ActorType::AiAgent => "ai_agent",
        ActorType::System => "system",
        ActorType::Integration => "integration",
    }
}

fn sanitized_intent_from_value(value: serde_json::Value) -> CommandResult<SanitizedLedgerIntent> {
    match value {
        serde_json::Value::Object(object) => SanitizedLedgerIntent::from_pairs(object),
        value => SanitizedLedgerIntent::from_pairs([("value", value)]),
    }
}

pub struct WriteCommandExecutor {
    pool: Pool<Sqlite>,
}

enum ClaimOutcome {
    Claimed { claim_token: String },
    Replayed(IdempotentCommandResult<serde_json::Value>),
}

struct PreparedWriteCommandRequest {
    request_hash: String,
    intent_json: String,
    summary_json: String,
    result_summary_json: String,
    lock_keys_json: String,
    primary_aggregate_key: Option<String>,
}

impl WriteCommandExecutor {
    pub fn new(pool: Pool<Sqlite>) -> Self {
        Self { pool }
    }

    pub async fn execute<F>(
        &self,
        ctx: &WriteCommandContext,
        request: WriteCommandRequest,
        service: F,
    ) -> CommandResult<IdempotentCommandResult<serde_json::Value>>
    where
        F: for<'tx> FnOnce(&'tx mut Transaction<'_, Sqlite>) -> WriteCommandServiceFuture<'tx>,
    {
        let prepared = prepare_write_command_request(request)?;
        let claim = self.claim_or_reclaim(ctx, &prepared).await?;

        match claim {
            ClaimOutcome::Claimed { claim_token } => {
                self.run_claimed(ctx, &claim_token, &prepared, service)
                    .await
            }
            ClaimOutcome::Replayed(result) => Ok(result),
        }
    }

    pub async fn execute_with_pre_transaction_guard<F, G, GFut, Guard>(
        &self,
        ctx: &WriteCommandContext,
        request: WriteCommandRequest,
        before_transaction: G,
        service: F,
    ) -> CommandResult<IdempotentCommandResult<serde_json::Value>>
    where
        F: for<'tx> FnOnce(&'tx mut Transaction<'_, Sqlite>) -> WriteCommandServiceFuture<'tx>,
        G: FnOnce() -> GFut,
        GFut: Future<Output = CommandResult<Guard>> + Send,
        Guard: Send,
    {
        let prepared = prepare_write_command_request(request)?;
        let claim = self.claim_or_reclaim(ctx, &prepared).await?;

        match claim {
            ClaimOutcome::Claimed { claim_token } => {
                let _guard = match self
                    .await_pre_transaction_guard(ctx, &claim_token, before_transaction())
                    .await
                {
                    Ok(guard) => guard,
                    Err(mut error) => {
                        error.request_id = Some(ctx.request_id.clone());
                        self.finalize_failure(ctx, &claim_token, &error).await?;
                        return Err(error);
                    }
                };
                self.run_claimed(ctx, &claim_token, &prepared, service)
                    .await
            }
            ClaimOutcome::Replayed(result) => Ok(result),
        }
    }

    pub async fn execute_with_resolved_guard<F, G, GFut, Guard>(
        &self,
        ctx: &WriteCommandContext,
        request: WriteCommandRequest,
        before_transaction: G,
        service: F,
    ) -> CommandResult<IdempotentCommandResult<serde_json::Value>>
    where
        F: for<'tx> FnOnce(
            &'tx mut Transaction<'_, Sqlite>,
            Guard,
        ) -> WriteCommandServiceFuture<'tx>,
        G: FnOnce() -> GFut,
        GFut: Future<Output = CommandResult<ResolvedWriteCommandGuard<Guard>>> + Send,
        Guard: Send,
    {
        let prepared = prepare_write_command_request(request)?;
        let claim = self.claim_or_reclaim(ctx, &prepared).await?;

        match claim {
            ClaimOutcome::Claimed { claim_token } => {
                let resolved = match self
                    .await_pre_transaction_guard(ctx, &claim_token, before_transaction())
                    .await
                {
                    Ok(guard) => guard,
                    Err(mut error) => {
                        error.request_id = Some(ctx.request_id.clone());
                        self.finalize_failure(ctx, &claim_token, &error).await?;
                        return Err(error);
                    }
                };

                if let Err(mut error) = self
                    .refresh_claim_lock_keys(ctx, &claim_token, resolved.lock_keys)
                    .await
                {
                    error.request_id = Some(ctx.request_id.clone());
                    self.finalize_failure(ctx, &claim_token, &error).await?;
                    return Err(error);
                }

                self.run_claimed(ctx, &claim_token, &prepared, move |tx| {
                    service(tx, resolved.guard)
                })
                .await
            }
            ClaimOutcome::Replayed(result) => Ok(result),
        }
    }

    async fn await_pre_transaction_guard<GFut, Guard>(
        &self,
        ctx: &WriteCommandContext,
        claim_token: &str,
        guard_future: GFut,
    ) -> CommandResult<Guard>
    where
        GFut: Future<Output = CommandResult<Guard>> + Send,
        Guard: Send,
    {
        tokio::pin!(guard_future);
        let mut refresh_interval =
            tokio::time::interval(std::time::Duration::from_secs(CLAIM_LEASE_REFRESH_SECONDS));
        refresh_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        refresh_interval.tick().await;

        loop {
            tokio::select! {
                result = &mut guard_future => return result,
                _ = refresh_interval.tick() => {
                    self.refresh_claim_lease(ctx, claim_token).await?;
                }
            }
        }
    }

    async fn refresh_claim_lock_keys<I, S>(
        &self,
        ctx: &WriteCommandContext,
        claim_token: &str,
        lock_keys: I,
    ) -> CommandResult<()>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut lock_keys = lock_keys
            .into_iter()
            .map(Into::into)
            .collect::<Vec<String>>();
        lock_keys.sort();
        lock_keys.dedup();
        if lock_keys.is_empty() {
            return Err(system_error("Resolved idempotency lock keys are required"));
        }
        let lock_keys_json = stable_json_string(&serde_json::json!(lock_keys))?;
        let now = Utc::now().to_rfc3339();
        let result = sqlx::query(
            "UPDATE command_idempotency
             SET lock_keys_json = ?,
                 updated_at = ?
             WHERE command_name = ?
               AND idempotency_key = ?
               AND status = ?
               AND claim_token = ?",
        )
        .bind(&lock_keys_json)
        .bind(&now)
        .bind(&ctx.command_name)
        .bind(&ctx.idempotency_key)
        .bind(CommandStatus::InProgress.as_str())
        .bind(claim_token)
        .execute(&self.pool)
        .await
        .map_err(|error| {
            let message = error.to_string();
            if crate::db_error_monitoring::classify_db_error_code(&message)
                == Some(codes::DB_LOCKED_RETRYABLE)
            {
                return CommandError::system(codes::DB_LOCKED_RETRYABLE, message).retryable(true);
            }
            system_error(message)
        })?;

        if result.rows_affected() == 1 {
            Ok(())
        } else {
            Err(CommandError::system(
                codes::SYSTEM_INTERNAL_ERROR,
                "Failed to refresh claimed idempotency lock keys",
            )
            .with_request_id(ctx.request_id.clone()))
        }
    }

    async fn refresh_claim_lease(
        &self,
        ctx: &WriteCommandContext,
        claim_token: &str,
    ) -> CommandResult<()> {
        let now = Utc::now();
        let now_string = now.to_rfc3339();
        let lease_expires_at = (now + chrono::Duration::seconds(CLAIM_LEASE_SECONDS)).to_rfc3339();
        let refresh = sqlx::query(
            "UPDATE command_idempotency
             SET lease_expires_at = ?,
                 updated_at = ?
             WHERE command_name = ?
               AND idempotency_key = ?
               AND status = ?
               AND claim_token = ?",
        )
        .bind(&lease_expires_at)
        .bind(&now_string)
        .bind(&ctx.command_name)
        .bind(&ctx.idempotency_key)
        .bind(CommandStatus::InProgress.as_str())
        .bind(claim_token)
        .execute(&self.pool);

        let result = match tokio::time::timeout(
            std::time::Duration::from_secs(CLAIM_LEASE_REFRESH_TIMEOUT_SECONDS),
            refresh,
        )
        .await
        {
            Ok(Ok(result)) => result,
            Ok(Err(error)) if lease_refresh_error_is_transient(&error) => return Ok(()),
            Ok(Err(error)) => return Err(system_error(error)),
            Err(_) => {
                return Err(CommandError::system(
                    codes::SYSTEM_INTERNAL_ERROR,
                    "Timed out refreshing claimed idempotency lease",
                )
                .retryable(true)
                .with_request_id(ctx.request_id.clone()))
            }
        };

        if result.rows_affected() == 1 {
            Ok(())
        } else {
            Err(CommandError::system(
                codes::SYSTEM_INTERNAL_ERROR,
                "Failed to refresh claimed idempotency lease",
            )
            .with_request_id(ctx.request_id.clone()))
        }
    }

    pub async fn execute_atomic<F>(
        &self,
        ctx: &WriteCommandContext,
        request: WriteCommandRequest,
        service: F,
    ) -> CommandResult<IdempotentCommandResult<serde_json::Value>>
    where
        F: for<'tx> FnOnce(&'tx mut Transaction<'_, Sqlite>) -> WriteCommandServiceFuture<'tx>,
    {
        let prepared = prepare_write_command_request(request)?;
        let now = Utc::now();
        let now_string = now.to_rfc3339();
        let lease_expires_at = (now + chrono::Duration::seconds(CLAIM_LEASE_SECONDS)).to_rfc3339();
        let claim_token = uuid::Uuid::new_v4().to_string();

        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(system_error)?;

        if let Some(row) = fetch_existing_claim_tx(&mut tx, ctx).await? {
            let outcome = resolve_existing_claim_row(ctx, &prepared.request_hash, row);
            tx.rollback().await.map_err(system_error)?;
            return outcome;
        }

        sqlx::query(
            "INSERT INTO command_idempotency (
                idempotency_key,
                command_name,
                request_id,
                actor_type,
                actor_id,
                client_id,
                session_id,
                channel_id,
                issued_at,
                request_hash,
                intent_json,
                summary_json,
                primary_aggregate_key,
                lock_keys_json,
                status,
                claim_token,
                response_json,
                result_summary_json,
                error_code,
                error_json,
                error_summary_json,
                retryable,
                lease_expires_at,
                created_at,
                updated_at,
                completed_at,
                last_attempt_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&ctx.idempotency_key)
        .bind(&ctx.command_name)
        .bind(&ctx.request_id)
        .bind(actor_type_as_str(ctx.actor_type))
        .bind(&ctx.actor_id)
        .bind(&ctx.client_id)
        .bind(&ctx.session_id)
        .bind(&ctx.channel_id)
        .bind(ctx.issued_at.to_rfc3339())
        .bind(&prepared.request_hash)
        .bind(&prepared.intent_json)
        .bind(&prepared.summary_json)
        .bind(&prepared.primary_aggregate_key)
        .bind(&prepared.lock_keys_json)
        .bind(CommandStatus::InProgress.as_str())
        .bind(&claim_token)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(0_i64)
        .bind(&lease_expires_at)
        .bind(&now_string)
        .bind(&now_string)
        .bind(Option::<String>::None)
        .bind(&now_string)
        .execute(&mut *tx)
        .await
        .map_err(system_error)?;

        let response = match service(&mut tx).await {
            Ok(response) => response,
            Err(mut error) => {
                tx.rollback().await.map_err(system_error)?;
                error.request_id = Some(ctx.request_id.clone());
                return Err(error);
            }
        };

        let response_json = stable_json_string(&response)?;
        let completed_at = chrono::Utc::now().to_rfc3339();
        let completion_result = sqlx::query(
            "UPDATE command_idempotency
             SET status = ?,
                 response_json = ?,
                 result_summary_json = ?,
                 error_code = NULL,
                 error_json = NULL,
                 error_summary_json = NULL,
                 retryable = 0,
                 lease_expires_at = NULL,
                 updated_at = ?,
                 completed_at = ?
             WHERE command_name = ?
               AND idempotency_key = ?
               AND status = ?
               AND claim_token = ?",
        )
        .bind(CommandStatus::Completed.as_str())
        .bind(&response_json)
        .bind(&prepared.result_summary_json)
        .bind(&completed_at)
        .bind(&completed_at)
        .bind(&ctx.command_name)
        .bind(&ctx.idempotency_key)
        .bind(CommandStatus::InProgress.as_str())
        .bind(&claim_token)
        .execute(&mut *tx)
        .await;

        let completion_result = match completion_result {
            Ok(result) => result,
            Err(error) => {
                let _ = tx.rollback().await;
                return Err(system_error(error).with_request_id(ctx.request_id.clone()));
            }
        };

        if completion_result.rows_affected() != 1 {
            tx.rollback().await.map_err(system_error)?;
            return Err(CommandError::system(
                codes::SYSTEM_INTERNAL_ERROR,
                "Failed to complete claimed idempotency row",
            )
            .with_request_id(ctx.request_id.clone()));
        }

        tx.commit().await.map_err(system_error)?;

        Ok(IdempotentCommandResult {
            response,
            replayed: false,
        })
    }

    async fn claim_or_reclaim(
        &self,
        ctx: &WriteCommandContext,
        prepared: &PreparedWriteCommandRequest,
    ) -> CommandResult<ClaimOutcome> {
        let now = Utc::now();
        let now_string = now.to_rfc3339();
        let lease_expires_at = (now + chrono::Duration::seconds(CLAIM_LEASE_SECONDS)).to_rfc3339();
        let claim_token = uuid::Uuid::new_v4().to_string();

        if let Some(row) = fetch_existing_claim(&self.pool, ctx).await? {
            if existing_claim_is_reclaimable(&row, &prepared.request_hash, now)? {
                if reclaim_claim(
                    &self.pool,
                    ctx,
                    prepared,
                    &claim_token,
                    &now_string,
                    &lease_expires_at,
                )
                .await?
                {
                    return Ok(ClaimOutcome::Claimed { claim_token });
                }

                return resolve_existing_claim(&self.pool, ctx, &prepared.request_hash).await;
            }

            return resolve_existing_claim_row(ctx, &prepared.request_hash, row)
                .map(ClaimOutcome::Replayed);
        }

        let mut claim_tx = self.pool.begin().await.map_err(system_error)?;
        let claim_result = sqlx::query(
            "INSERT OR IGNORE INTO command_idempotency (
                idempotency_key,
                command_name,
                request_id,
                actor_type,
                actor_id,
                client_id,
                session_id,
                channel_id,
                issued_at,
                request_hash,
                intent_json,
                summary_json,
                primary_aggregate_key,
                lock_keys_json,
                status,
                claim_token,
                response_json,
                result_summary_json,
                error_code,
                error_json,
                error_summary_json,
                retryable,
                lease_expires_at,
                created_at,
                updated_at,
                completed_at,
                last_attempt_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&ctx.idempotency_key)
        .bind(&ctx.command_name)
        .bind(&ctx.request_id)
        .bind(actor_type_as_str(ctx.actor_type))
        .bind(&ctx.actor_id)
        .bind(&ctx.client_id)
        .bind(&ctx.session_id)
        .bind(&ctx.channel_id)
        .bind(ctx.issued_at.to_rfc3339())
        .bind(&prepared.request_hash)
        .bind(&prepared.intent_json)
        .bind(&prepared.summary_json)
        .bind(&prepared.primary_aggregate_key)
        .bind(&prepared.lock_keys_json)
        .bind(CommandStatus::InProgress.as_str())
        .bind(&claim_token)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(Option::<String>::None)
        .bind(0_i64)
        .bind(&lease_expires_at)
        .bind(&now_string)
        .bind(&now_string)
        .bind(Option::<String>::None)
        .bind(&now_string)
        .execute(&mut *claim_tx)
        .await
        .map_err(system_error)?;

        claim_tx.commit().await.map_err(system_error)?;

        if claim_result.rows_affected() != 1 {
            return resolve_existing_claim(&self.pool, ctx, &prepared.request_hash).await;
        }

        Ok(ClaimOutcome::Claimed { claim_token })
    }

    async fn run_claimed<F>(
        &self,
        ctx: &WriteCommandContext,
        claim_token: &str,
        prepared: &PreparedWriteCommandRequest,
        service: F,
    ) -> CommandResult<IdempotentCommandResult<serde_json::Value>>
    where
        F: for<'tx> FnOnce(&'tx mut Transaction<'_, Sqlite>) -> WriteCommandServiceFuture<'tx>,
    {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(system_error)?;

        let response = match service(&mut tx).await {
            Ok(response) => response,
            Err(mut error) => {
                tx.rollback().await.map_err(system_error)?;
                error.request_id = Some(ctx.request_id.clone());
                self.finalize_failure(ctx, claim_token, &error).await?;
                return Err(error);
            }
        };

        let response_json = stable_json_string(&response)?;
        let completed_at = chrono::Utc::now().to_rfc3339();
        let completion_result = sqlx::query(
            "UPDATE command_idempotency
             SET status = ?,
                 response_json = ?,
                 result_summary_json = ?,
                 error_code = NULL,
                 error_json = NULL,
                 error_summary_json = NULL,
                 retryable = 0,
                 lease_expires_at = NULL,
                 updated_at = ?,
                 completed_at = ?
             WHERE command_name = ?
               AND idempotency_key = ?
               AND status = ?
               AND claim_token = ?",
        )
        .bind(CommandStatus::Completed.as_str())
        .bind(&response_json)
        .bind(&prepared.result_summary_json)
        .bind(&completed_at)
        .bind(&completed_at)
        .bind(&ctx.command_name)
        .bind(&ctx.idempotency_key)
        .bind(CommandStatus::InProgress.as_str())
        .bind(claim_token)
        .execute(&mut *tx)
        .await;

        let completion_result = match completion_result {
            Ok(result) => result,
            Err(error) => {
                let _ = tx.rollback().await;
                return Err(system_error(error).with_request_id(ctx.request_id.clone()));
            }
        };

        if completion_result.rows_affected() != 1 {
            tx.rollback().await.map_err(system_error)?;
            return Err(CommandError::system(
                codes::SYSTEM_INTERNAL_ERROR,
                "Failed to complete claimed idempotency row",
            )
            .with_request_id(ctx.request_id.clone()));
        }

        tx.commit().await.map_err(system_error)?;

        Ok(IdempotentCommandResult {
            response,
            replayed: false,
        })
    }

    async fn finalize_failure(
        &self,
        ctx: &WriteCommandContext,
        claim_token: &str,
        error: &CommandError,
    ) -> CommandResult<()> {
        let status = if error.retryable {
            CommandStatus::FailedRetryable
        } else {
            CommandStatus::FailedTerminal
        };
        let retryable = if error.retryable { 1_i64 } else { 0_i64 };
        let error_json = serde_json::to_string(error).map_err(system_error)?;
        let error_summary_json = CommandLedgerErrorSummary::from_error(error).to_json_string()?;
        let now = chrono::Utc::now().to_rfc3339();
        let completed_at = if status == CommandStatus::FailedTerminal {
            Some(now.clone())
        } else {
            None
        };

        let mut tx = self.pool.begin().await.map_err(system_error)?;
        let result = sqlx::query(
            "UPDATE command_idempotency
             SET status = ?,
                 response_json = NULL,
                 result_summary_json = NULL,
                 error_code = ?,
                 error_json = ?,
                 error_summary_json = ?,
                 retryable = ?,
                 lease_expires_at = NULL,
                 updated_at = ?,
                 completed_at = ?
             WHERE command_name = ?
               AND idempotency_key = ?
               AND status = ?
               AND claim_token = ?",
        )
        .bind(status.as_str())
        .bind(&error.code)
        .bind(&error_json)
        .bind(&error_summary_json)
        .bind(retryable)
        .bind(&now)
        .bind(&completed_at)
        .bind(&ctx.command_name)
        .bind(&ctx.idempotency_key)
        .bind(CommandStatus::InProgress.as_str())
        .bind(claim_token)
        .execute(&mut *tx)
        .await
        .map_err(system_error)?;

        if result.rows_affected() != 1 {
            tx.rollback().await.map_err(system_error)?;
            return Err(CommandError::system(
                codes::SYSTEM_INTERNAL_ERROR,
                "Failed to record failed idempotency row",
            )
            .with_request_id(ctx.request_id.clone()));
        }

        tx.commit().await.map_err(system_error)
    }
}

fn lease_refresh_error_is_transient(error: &sqlx::Error) -> bool {
    crate::db_error_monitoring::classify_db_error_code(&error.to_string())
        == Some(codes::DB_LOCKED_RETRYABLE)
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

pub async fn set_crash_reporting_preference_idempotent(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    enabled: bool,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let intent = serde_json::json!({ "enabled": enabled });
    let request = WriteCommandRequest::new_low_risk(intent, "Set crash reporting preference")?
        .with_success_summary(CommandLedgerResultSummary::success(
            "Crash reporting preference saved",
        )?);

    WriteCommandExecutor::new(pool.clone())
        .execute(ctx, request, move |tx| {
            Box::pin(async move {
                settings_store::save_setting_tx(
                    tx,
                    "send_crash_reports",
                    if enabled { "true" } else { "false" },
                )
                .await
                .map_err(system_error)?;

                Ok(serde_json::json!({ "ok": true }))
            })
        })
        .await
}

fn canonicalize_json_value(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries = map.into_iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));

            let mut canonical = serde_json::Map::new();
            for (key, value) in entries {
                canonical.insert(key, canonicalize_json_value(value));
            }

            serde_json::Value::Object(canonical)
        }
        serde_json::Value::Array(values) => serde_json::Value::Array(
            values
                .into_iter()
                .map(canonicalize_json_value)
                .collect::<Vec<_>>(),
        ),
        primitive => primitive,
    }
}

fn stable_json_string(value: &serde_json::Value) -> CommandResult<String> {
    serde_json::to_string(&canonicalize_json_value(value.clone())).map_err(system_error)
}

pub(crate) fn stable_request_hash(intent: &serde_json::Value) -> CommandResult<String> {
    let intent_json = stable_json_string(intent)?;
    stable_request_hash_from_json(&intent_json)
}

fn stable_request_hash_from_json(intent_json: &str) -> CommandResult<String> {
    let digest = Sha256::digest(intent_json.as_bytes());
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn prepare_write_command_request(
    request: WriteCommandRequest,
) -> CommandResult<PreparedWriteCommandRequest> {
    let mut lock_keys = (request.lock_key_deriver)(&request.hash_payload)?;
    lock_keys.sort();
    lock_keys.dedup();

    let lock_keys_json = stable_json_string(&serde_json::json!(lock_keys))?;
    let hash_payload_json = stable_json_string(&request.hash_payload)?;
    let request_hash = stable_request_hash_from_json(&hash_payload_json)?;
    let intent_json = stable_json_string(&request.ledger_intent.to_value()?)?;
    let summary_json = stable_json_string(&request.summary.to_value()?)?;
    let result_summary_json = request.success_summary.to_json_string()?;

    Ok(PreparedWriteCommandRequest {
        request_hash,
        intent_json,
        summary_json,
        result_summary_json,
        lock_keys_json,
        primary_aggregate_key: request.primary_aggregate_key,
    })
}

async fn resolve_existing_claim(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    request_hash: &str,
) -> CommandResult<ClaimOutcome> {
    let row = fetch_existing_claim(pool, ctx)
        .await?
        .ok_or_else(|| system_error("Idempotency claim was not inserted and no row exists"))?;

    resolve_existing_claim_row(ctx, request_hash, row).map(ClaimOutcome::Replayed)
}

async fn reclaim_claim(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    prepared: &PreparedWriteCommandRequest,
    claim_token: &str,
    now: &str,
    lease_expires_at: &str,
) -> CommandResult<bool> {
    let mut tx = pool.begin().await.map_err(system_error)?;
    let result = sqlx::query(
        "UPDATE command_idempotency
         SET status = 'in_progress',
             claim_token = ?,
             response_json = NULL,
             result_summary_json = NULL,
             error_code = NULL,
             error_json = NULL,
             error_summary_json = NULL,
             retryable = 0,
             lease_expires_at = ?,
             updated_at = ?,
             completed_at = NULL,
             last_attempt_at = ?
         WHERE command_name = ?
           AND idempotency_key = ?
           AND request_hash = ?
           AND (
                status = 'failed_retryable'
                OR (
                    status = 'in_progress'
                    AND (lease_expires_at IS NULL OR lease_expires_at <= ?)
                )
           )",
    )
    .bind(claim_token)
    .bind(lease_expires_at)
    .bind(now)
    .bind(now)
    .bind(&ctx.command_name)
    .bind(&ctx.idempotency_key)
    .bind(&prepared.request_hash)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(system_error)?;

    tx.commit().await.map_err(system_error)?;

    Ok(result.rows_affected() == 1)
}

async fn fetch_existing_claim(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
) -> CommandResult<Option<SqliteRow>> {
    sqlx::query(
        "SELECT request_hash, status, response_json, error_json, lease_expires_at
         FROM command_idempotency
         WHERE command_name = ? AND idempotency_key = ?",
    )
    .bind(&ctx.command_name)
    .bind(&ctx.idempotency_key)
    .fetch_optional(pool)
    .await
    .map_err(system_error)
}

async fn fetch_existing_claim_tx(
    tx: &mut Transaction<'_, Sqlite>,
    ctx: &WriteCommandContext,
) -> CommandResult<Option<SqliteRow>> {
    sqlx::query(
        "SELECT request_hash, status, response_json, error_json, lease_expires_at
         FROM command_idempotency
         WHERE command_name = ? AND idempotency_key = ?",
    )
    .bind(&ctx.command_name)
    .bind(&ctx.idempotency_key)
    .fetch_optional(&mut **tx)
    .await
    .map_err(system_error)
}

fn existing_claim_is_reclaimable(
    row: &SqliteRow,
    request_hash: &str,
    now: DateTime<Utc>,
) -> CommandResult<bool> {
    let stored_hash: String = row.get("request_hash");
    if stored_hash != request_hash {
        return Ok(false);
    }

    let status = CommandStatus::from_str(row.get::<String, _>("status").as_str())?;
    match status {
        CommandStatus::FailedRetryable => return Ok(true),
        CommandStatus::InProgress => {}
        CommandStatus::Completed | CommandStatus::FailedTerminal => return Ok(false),
    }

    let lease_expires_at: Option<String> = row.try_get("lease_expires_at").map_err(system_error)?;
    let Some(lease_expires_at) = lease_expires_at else {
        return Ok(true);
    };

    let lease_expires_at = DateTime::parse_from_rfc3339(&lease_expires_at)
        .map_err(system_error)?
        .with_timezone(&Utc);

    Ok(lease_expires_at <= now)
}

fn resolve_existing_claim_row(
    ctx: &WriteCommandContext,
    request_hash: &str,
    row: SqliteRow,
) -> CommandResult<IdempotentCommandResult<serde_json::Value>> {
    let stored_hash: String = row.get("request_hash");
    if stored_hash != request_hash {
        return Err(CommandError::user(
            codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH,
            "Idempotency key was reused with a different request payload",
        )
        .with_request_id(ctx.request_id.clone()));
    }

    let status = CommandStatus::from_str(row.get::<String, _>("status").as_str())?;
    match status {
        CommandStatus::Completed => {
            let raw_response = row
                .try_get::<Option<String>, _>("response_json")
                .map_err(system_error)?
                .ok_or_else(|| {
                    system_error("completed idempotency row is missing response_json")
                })?;
            let response = serde_json::from_str(&raw_response).map_err(system_error)?;
            Ok(IdempotentCommandResult {
                response,
                replayed: true,
            })
        }
        CommandStatus::FailedTerminal => {
            let raw_error = row
                .try_get::<Option<String>, _>("error_json")
                .map_err(system_error)?
                .ok_or_else(|| system_error("terminal idempotency row is missing error_json"))?;
            let mut error: CommandError = serde_json::from_str(&raw_error).map_err(system_error)?;
            error.request_id = Some(ctx.request_id.clone());
            Err(error)
        }
        CommandStatus::InProgress | CommandStatus::FailedRetryable => Err(CommandError::user(
            codes::CONFLICT_DUPLICATE_IN_FLIGHT,
            "A command with this idempotency key is already in progress",
        )
        .with_request_id(ctx.request_id.clone())),
    }
}

pub(crate) fn system_error(error: impl std::fmt::Display) -> CommandError {
    CommandError::system(codes::SYSTEM_INTERNAL_ERROR, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_error::codes;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool_with_max_connections(max_connections: u32) -> Pool<Sqlite> {
        let database_url = format!(
            "sqlite://file:{}?mode=memory&cache=shared",
            uuid::Uuid::new_v4()
        );

        let pool = SqlitePoolOptions::new()
            .max_connections(max_connections)
            .connect(&database_url)
            .await
            .expect("failed to open sqlite test pool");

        crate::db::run_migrations(&pool)
            .await
            .expect("failed to run migrations");

        pool
    }

    async fn test_pool() -> Pool<Sqlite> {
        test_pool_with_max_connections(5).await
    }

    #[test]
    fn write_command_context_rejects_blank_idempotency_key() {
        let error =
            WriteCommandContext::for_scoped_command("req-blank", "   ", "create_reservation")
                .expect_err("blank key should be rejected");

        assert_eq!(error.code, codes::IDEMPOTENCY_KEY_REQUIRED);
    }

    #[test]
    fn canonical_request_hash_sorts_object_keys_recursively() {
        let first = serde_json::json!({
            "schema": "reservation.create.v1",
            "payload": {
                "room_id": "R1",
                "dates": {
                    "check_out": "2026-04-28",
                    "check_in": "2026-04-27"
                }
            }
        });
        let second = serde_json::json!({
            "payload": {
                "dates": {
                    "check_in": "2026-04-27",
                    "check_out": "2026-04-28"
                },
                "room_id": "R1"
            },
            "schema": "reservation.create.v1"
        });

        assert_eq!(
            stable_request_hash(&first).expect("hashes first payload"),
            stable_request_hash(&second).expect("hashes second payload")
        );
    }

    #[test]
    fn canonical_request_hash_keeps_array_order_meaningful() {
        let first = serde_json::json!({
            "schema": "example.v1",
            "room_ids": ["R1", "R2"]
        });
        let second = serde_json::json!({
            "schema": "example.v1",
            "room_ids": ["R2", "R1"]
        });

        assert_ne!(
            stable_request_hash(&first).expect("hashes first payload"),
            stable_request_hash(&second).expect("hashes second payload")
        );
    }

    #[tokio::test]
    async fn execute_atomic_rolls_back_claim_when_service_fails() {
        let pool = test_pool().await;
        let ctx = WriteCommandContext::for_internal_test(
            "req-atomic-fail",
            "idem-atomic-fail",
            "test.atomic_fail",
        );
        let request = WriteCommandRequest::new_low_risk(
            serde_json::json!({ "schema": "test.atomic.v1", "value": "fail" }),
            "Atomic failure test",
        )
        .expect("builds request");

        let error = WriteCommandExecutor::new(pool.clone())
            .execute_atomic(&ctx, request, |_tx| {
                Box::pin(async move {
                    Err(CommandError::system(
                        codes::SYSTEM_INTERNAL_ERROR,
                        "forced failure",
                    ))
                })
            })
            .await
            .expect_err("service failure should return error");

        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM command_idempotency
             WHERE command_name = ? AND idempotency_key = ?",
        )
        .bind(&ctx.command_name)
        .bind(&ctx.idempotency_key)
        .fetch_one(&pool)
        .await
        .expect("counts command rows");

        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn execute_atomic_replay_does_not_run_service_twice() {
        let pool = test_pool().await;
        sqlx::query("CREATE TABLE atomic_counter (id INTEGER PRIMARY KEY)")
            .execute(&pool)
            .await
            .expect("creates counter table");
        let ctx = WriteCommandContext::for_internal_test(
            "req-atomic-replay",
            "idem-atomic-replay",
            "test.atomic_replay",
        );
        let request = WriteCommandRequest::new_low_risk(
            serde_json::json!({ "schema": "test.atomic.v1", "value": "ok" }),
            "Atomic replay test",
        )
        .expect("builds request");

        let first = WriteCommandExecutor::new(pool.clone())
            .execute_atomic(&ctx, request.clone(), |tx| {
                Box::pin(async move {
                    sqlx::query("INSERT INTO atomic_counter (id) VALUES (1)")
                        .execute(&mut **tx)
                        .await
                        .map_err(system_error)?;
                    Ok(serde_json::json!({ "ok": true }))
                })
            })
            .await
            .expect("first command completes");

        let second = WriteCommandExecutor::new(pool.clone())
            .execute_atomic(&ctx, request, |_tx| {
                Box::pin(async move {
                    panic!("business logic must not run on replay");
                })
            })
            .await
            .expect("second command replays");

        assert!(!first.replayed);
        assert!(second.replayed);
        assert_eq!(first.response, second.response);
    }

    #[test]
    fn sanitized_ledger_intent_rejects_sensitive_keys() {
        for key in [
            "phone",
            "email",
            "payment_token",
            "card_data",
            "access_token",
            "secret",
            "password",
            "guest_note",
            "prompt",
            "raw_external_payload",
            "paymentToken",
            "cardNumber",
            "phoneNumber",
            "rawPayload",
            "guestEmail",
        ] {
            let error = SanitizedLedgerIntent::from_pairs([(key, serde_json::json!("value"))])
                .expect_err("sensitive key must be rejected");
            assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
        }
    }

    #[test]
    fn command_ledger_summary_rejects_sensitive_safe_fields() {
        let error = CommandLedgerSummary::new("Safe label")
            .and_then(|summary| summary.with_safe_field("email", "guest@example.com"))
            .expect_err("email safe field must be rejected");

        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
    }

    #[test]
    fn command_ledger_summary_rejects_email_like_values() {
        let error = CommandLedgerSummary::new("Safe label")
            .and_then(|summary| summary.with_safe_field("room_label", "guest@example.com"))
            .expect_err("email-like value must be rejected");

        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
    }

    #[test]
    fn sanitized_ledger_intent_rejects_sensitive_values() {
        for value in [
            "phone number",
            "email address",
            "payment token captured",
            "card_data",
            "access_token",
            "secret",
            "password",
            "guest_note",
            "prompt",
            "raw",
            "payload",
            "paymentToken",
            "cardNumber",
            "phoneNumber",
            "rawPayload",
            "guestEmail",
        ] {
            let error =
                SanitizedLedgerIntent::from_pairs([("safe_label", serde_json::json!(value))])
                    .expect_err("sensitive value must be rejected");
            assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
        }
    }

    #[test]
    fn sanitized_ledger_intent_rejects_numeric_pii_values() {
        for value in [
            serde_json::json!("0900000000"),
            serde_json::json!("4111 1111 1111 1111"),
            serde_json::json!(4111111111111111_u64),
        ] {
            let error = SanitizedLedgerIntent::from_pairs([("safe_label", value)])
                .expect_err("numeric PII value must be rejected");
            assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
        }
    }

    #[test]
    fn command_ledger_summary_to_value_revalidates_direct_construction() {
        let mut safe_fields = BTreeMap::new();
        safe_fields.insert(
            "room_label".to_string(),
            "payment token captured".to_string(),
        );
        let summary = CommandLedgerSummary {
            label: "Safe label".to_string(),
            aggregate_refs: Vec::new(),
            business_dates: Vec::new(),
            safe_fields,
        };

        let error = summary
            .to_value()
            .expect_err("directly constructed unsafe summary must be rejected");
        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
    }

    #[test]
    fn command_ledger_summary_rejects_phone_like_values() {
        let error = CommandLedgerSummary::new("Safe label")
            .and_then(|summary| summary.with_safe_field("room_label", "0900000000"))
            .expect_err("phone-like value must be rejected");

        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
    }

    #[test]
    fn command_ledger_summary_serializes_safe_fields() {
        let summary = CommandLedgerSummary::new("Check-in booking #123")
            .expect("summary builds")
            .with_aggregate_ref("booking", "123", Some("Booking #123"))
            .expect("booking ref is safe")
            .with_aggregate_ref("room", "205", Some("Room 205"))
            .expect("room ref is safe")
            .with_business_date("2026-04-26")
            .expect("date is safe")
            .with_safe_field("room_label", "205")
            .expect("safe field is accepted");

        let value = summary.to_value().expect("summary serializes");
        assert_eq!(value["label"], "Check-in booking #123");
        assert_eq!(value["aggregate_refs"][0]["type"], "booking");
        assert_eq!(value["aggregate_refs"][1]["id"], "205");
        assert_eq!(value["business_dates"][0], "2026-04-26");
        assert_eq!(value["safe_fields"]["room_label"], "205");
    }

    fn test_summary(label: &str) -> CommandLedgerSummary {
        CommandLedgerSummary::new(label)
            .expect("summary builds")
            .with_aggregate_ref("booking", "123", Some("Booking #123"))
            .expect("aggregate ref builds")
            .with_safe_field("room_label", "205")
            .expect("safe field builds")
    }

    fn test_intent() -> SanitizedLedgerIntent {
        SanitizedLedgerIntent::from_pairs([
            ("booking_id", serde_json::json!("123")),
            ("room_id", serde_json::json!("205")),
        ])
        .expect("intent builds")
    }

    fn original_test_lock_keys(_intent: &serde_json::Value) -> CommandResult<Vec<String>> {
        Ok(vec!["booking:123".to_string()])
    }

    fn retry_test_lock_keys(_intent: &serde_json::Value) -> CommandResult<Vec<String>> {
        Ok(vec!["booking:999".to_string()])
    }

    #[tokio::test]
    async fn write_command_executor_persists_operator_safe_metadata() {
        let pool = test_pool().await;
        let issued_at =
            DateTime::parse_from_rfc3339("2026-04-26T09:30:00+07:00").expect("issued_at parses");
        let ctx = WriteCommandContext {
            request_id: "req-ledger-metadata".to_string(),
            idempotency_key: "idem-ledger-metadata".to_string(),
            command_name: "test.ledger_metadata".to_string(),
            actor_id: Some("admin-1".to_string()),
            actor_type: ActorType::Human,
            client_id: Some("client-opaque".to_string()),
            session_id: Some("session-opaque".to_string()),
            channel_id: Some("tauri".to_string()),
            issued_at,
        };
        let request = WriteCommandRequest::new_sanitized(
            serde_json::json!({
                "booking_id": "123",
                "room_id": "205",
                "guest_phone": "0900000000"
            }),
            test_intent(),
            test_summary("Check-in booking #123"),
        )
        .expect("request builds")
        .with_primary_aggregate_key("booking:123")
        .with_success_summary(CommandLedgerResultSummary::success("Command completed").unwrap());

        let result = WriteCommandExecutor::new(pool.clone())
            .execute(&ctx, request, |_tx| {
                Box::pin(
                    async move { Ok(serde_json::json!({ "ok": true, "internal": "raw replay" })) },
                )
            })
            .await
            .expect("command succeeds");

        assert!(!result.replayed);

        let row = sqlx::query(
            "SELECT request_id, actor_type, actor_id, client_id, session_id, channel_id,
                    issued_at, intent_json, summary_json, result_summary_json, response_json
             FROM command_idempotency
             WHERE command_name = ? AND idempotency_key = ?",
        )
        .bind(&ctx.command_name)
        .bind(&ctx.idempotency_key)
        .fetch_one(&pool)
        .await
        .expect("reads command row");

        assert_eq!(
            row.get::<Option<String>, _>("request_id"),
            Some(ctx.request_id)
        );
        assert_eq!(row.get::<String, _>("actor_type"), "human");
        assert_eq!(
            row.get::<Option<String>, _>("actor_id"),
            Some("admin-1".to_string())
        );
        assert_eq!(
            row.get::<Option<String>, _>("client_id"),
            Some("client-opaque".to_string())
        );
        assert_eq!(
            row.get::<Option<String>, _>("session_id"),
            Some("session-opaque".to_string())
        );
        assert_eq!(
            row.get::<Option<String>, _>("channel_id"),
            Some("tauri".to_string())
        );
        assert_eq!(
            row.get::<Option<String>, _>("issued_at"),
            Some(issued_at.to_rfc3339())
        );

        let intent_json = row.get::<String, _>("intent_json");
        assert!(intent_json.contains("booking_id"));
        assert!(!intent_json.contains("guest_phone"));

        let summary_json = row.get::<String, _>("summary_json");
        assert!(summary_json.contains("Check-in booking #123"));
        assert!(!summary_json.contains("0900000000"));

        let result_summary_json = row
            .get::<Option<String>, _>("result_summary_json")
            .expect("result summary is stored");
        assert!(result_summary_json.contains("Command completed"));

        assert!(row
            .get::<Option<String>, _>("response_json")
            .expect("raw replay response is stored")
            .contains("raw replay"));
    }

    #[tokio::test]
    async fn write_command_executor_hashes_full_payload_but_stores_sanitized_intent() {
        let pool = test_pool().await;
        let ctx = WriteCommandContext::for_internal_test(
            "req-hash-split",
            "idem-hash-split",
            "test.hash_split",
        );
        let first = WriteCommandRequest::new_sanitized(
            serde_json::json!({
                "booking_id": "123",
                "room_id": "205",
                "guest_phone": "0900000000"
            }),
            test_intent(),
            test_summary("Hash split"),
        )
        .expect("first request builds");
        let second = WriteCommandRequest::new_sanitized(
            serde_json::json!({
                "booking_id": "123",
                "room_id": "205",
                "guest_phone": "0911111111"
            }),
            test_intent(),
            test_summary("Hash split"),
        )
        .expect("second request builds");

        WriteCommandExecutor::new(pool.clone())
            .execute(&ctx, first, |_tx| {
                Box::pin(async move { Ok(serde_json::json!({ "ok": true })) })
            })
            .await
            .expect("first request succeeds");

        let error = WriteCommandExecutor::new(pool.clone())
            .execute(&ctx, second, |_tx| {
                Box::pin(async move { Ok(serde_json::json!({ "unexpected": true })) })
            })
            .await
            .expect_err("different hash payload conflicts");

        assert_eq!(error.code, codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH);

        let intent_json: String = sqlx::query_scalar(
            "SELECT intent_json FROM command_idempotency
             WHERE command_name = ? AND idempotency_key = ?",
        )
        .bind(&ctx.command_name)
        .bind(&ctx.idempotency_key)
        .fetch_one(&pool)
        .await
        .expect("reads intent");
        assert!(!intent_json.contains("guest_phone"));
        assert!(!intent_json.contains("0900000000"));
    }

    #[tokio::test]
    async fn write_command_executor_retry_reclaim_preserves_original_operator_metadata() {
        let pool = test_pool().await;
        let mut original_ctx = WriteCommandContext::for_internal_test(
            "req-original",
            "idem-reclaim-preserve",
            "test.reclaim_preserve",
        );
        original_ctx.actor_id = Some("original-actor".to_string());
        let mut retry_ctx = original_ctx.clone();
        retry_ctx.request_id = "req-retry".to_string();
        retry_ctx.actor_id = Some("retry-actor".to_string());

        let hash_payload = serde_json::json!({ "booking_id": "123" });
        let original_request = WriteCommandRequest::new_sanitized(
            hash_payload.clone(),
            test_intent(),
            test_summary("Original summary"),
        )
        .expect("original request builds")
        .with_primary_aggregate_key("booking:123")
        .with_lock_key_deriver(original_test_lock_keys);
        let retry_request = WriteCommandRequest::new_sanitized(
            hash_payload,
            SanitizedLedgerIntent::from_pairs([
                ("booking_id", serde_json::json!("999")),
                ("room_id", serde_json::json!("999")),
            ])
            .expect("retry intent builds"),
            test_summary("Retry summary"),
        )
        .expect("retry request builds")
        .with_primary_aggregate_key("booking:999")
        .with_lock_key_deriver(retry_test_lock_keys);

        WriteCommandExecutor::new(pool.clone())
            .execute(&original_ctx, original_request, |_tx| {
                Box::pin(async move {
                    Err(CommandError::system(codes::DB_LOCKED_RETRYABLE, "locked").retryable(true))
                })
            })
            .await
            .expect_err("first attempt is retryable");

        WriteCommandExecutor::new(pool.clone())
            .execute(&retry_ctx, retry_request, |_tx| {
                Box::pin(async move { Ok(serde_json::json!({ "ok": true })) })
            })
            .await
            .expect("retry succeeds");

        let row = sqlx::query(
            "SELECT request_id, actor_id, intent_json, summary_json, primary_aggregate_key,
                    lock_keys_json, result_summary_json, error_summary_json
             FROM command_idempotency
             WHERE command_name = ? AND idempotency_key = ?",
        )
        .bind(&original_ctx.command_name)
        .bind(&original_ctx.idempotency_key)
        .fetch_one(&pool)
        .await
        .expect("reads row");

        assert_eq!(
            row.get::<Option<String>, _>("request_id"),
            Some("req-original".to_string())
        );
        assert_eq!(
            row.get::<Option<String>, _>("actor_id"),
            Some("original-actor".to_string())
        );

        let intent_json = row.get::<String, _>("intent_json");
        assert!(intent_json.contains("\"booking_id\":\"123\""));
        assert!(intent_json.contains("\"room_id\":\"205\""));
        assert!(!intent_json.contains("\"booking_id\":\"999\""));
        assert!(!intent_json.contains("\"room_id\":\"999\""));

        assert!(row
            .get::<String, _>("summary_json")
            .contains("Original summary"));
        assert_eq!(
            row.get::<Option<String>, _>("primary_aggregate_key"),
            Some("booking:123".to_string())
        );
        assert_eq!(row.get::<String, _>("lock_keys_json"), "[\"booking:123\"]");
        assert!(row
            .get::<Option<String>, _>("result_summary_json")
            .is_some());
        assert_eq!(row.get::<Option<String>, _>("error_summary_json"), None);
    }

    #[tokio::test]
    async fn write_command_executor_failure_summary_falls_back_for_unsafe_error_message() {
        let pool = test_pool().await;
        let ctx = WriteCommandContext::for_internal_test(
            "req-unsafe-error-summary",
            "idem-unsafe-error-summary",
            "test.unsafe_error_summary",
        );
        let request = WriteCommandRequest::new_sanitized(
            serde_json::json!({ "case": "unsafe_error" }),
            SanitizedLedgerIntent::from_pairs([("case", serde_json::json!("safe"))]).unwrap(),
            CommandLedgerSummary::new("Unsafe error summary test").unwrap(),
        )
        .expect("request builds");

        let error = WriteCommandExecutor::new(pool.clone())
            .execute(&ctx, request, |_tx| {
                Box::pin(async move {
                    Err(CommandError::system(
                        codes::SYSTEM_INTERNAL_ERROR,
                        "payment token leaked in raw error",
                    ))
                })
            })
            .await
            .expect_err("command fails");
        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);

        let error_summary_json: Option<String> = sqlx::query_scalar(
            "SELECT error_summary_json FROM command_idempotency
             WHERE command_name = ? AND idempotency_key = ?",
        )
        .bind(&ctx.command_name)
        .bind(&ctx.idempotency_key)
        .fetch_one(&pool)
        .await
        .expect("reads error summary");
        let error_summary_json = error_summary_json.expect("safe error summary is stored");
        assert!(!error_summary_json.contains("payment"));
        assert!(!error_summary_json.contains("token"));
        assert!(error_summary_json.contains("Command failed"));
    }

    #[tokio::test]
    async fn execute_with_pre_transaction_guard_keeps_claim_fresh_while_waiting() {
        let pool = test_pool().await;
        let ctx = WriteCommandContext::for_internal_test(
            "test-request-pre-guard-lease",
            "idem-pre-guard-lease",
            "test.pre_guard_lease",
        );
        let request = WriteCommandRequest::new_low_risk(
            serde_json::json!({ "case": "pre-guard-lease" }),
            "Pre-guard lease",
        )
        .expect("request builds");
        let wait_started = std::sync::Arc::new(tokio::sync::Notify::new());
        let release_guard = std::sync::Arc::new(tokio::sync::Notify::new());

        let runner_pool = pool.clone();
        let runner_ctx = ctx.clone();
        let runner_request = request.clone();
        let runner_wait_started = wait_started.clone();
        let runner_release_guard = release_guard.clone();
        let runner = tokio::spawn(async move {
            WriteCommandExecutor::new(runner_pool)
                .execute_with_pre_transaction_guard(
                    &runner_ctx,
                    runner_request,
                    move || {
                        let wait_started = runner_wait_started.clone();
                        let release_guard = runner_release_guard.clone();
                        async move {
                            wait_started.notify_one();
                            release_guard.notified().await;
                            Ok(())
                        }
                    },
                    |_tx| Box::pin(async move { Ok(serde_json::json!({ "ok": true })) }),
                )
                .await
        });

        wait_started.notified().await;

        let expired_lease = (Utc::now() - chrono::Duration::seconds(1)).to_rfc3339();
        sqlx::query(
            "UPDATE command_idempotency
             SET lease_expires_at = ?
             WHERE command_name = ? AND idempotency_key = ?",
        )
        .bind(&expired_lease)
        .bind(&ctx.command_name)
        .bind(&ctx.idempotency_key)
        .execute(&pool)
        .await
        .expect("expires active claim lease");

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let duplicate_error = WriteCommandExecutor::new(pool.clone())
            .execute_with_pre_transaction_guard(
                &ctx,
                request,
                || async { Ok(()) },
                |_tx| Box::pin(async move { Ok(serde_json::json!({ "unexpected": true })) }),
            )
            .await
            .expect_err("duplicate must remain in-flight while original waits for guard");

        assert_eq!(duplicate_error.code, codes::CONFLICT_DUPLICATE_IN_FLIGHT);

        release_guard.notify_one();
        let result = runner
            .await
            .expect("original task joins")
            .expect("original guarded command completes");

        assert_eq!(result.response, serde_json::json!({ "ok": true }));
        assert!(!result.replayed);
    }

    #[tokio::test]
    async fn execute_with_resolved_guard_finalizes_guard_error_as_terminal() {
        let pool = test_pool().await;
        let ctx = WriteCommandContext::for_internal_test(
            "test-request-resolved-guard-error",
            "idem-resolved-guard-error",
            "test.resolved_guard_error",
        );
        let request = WriteCommandRequest::new_low_risk(
            serde_json::json!({ "case": "resolved-guard-error" }),
            "Resolved guard error",
        )
        .expect("request builds");
        let guard_attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let first_attempts = guard_attempts.clone();
        let first_error = WriteCommandExecutor::new(pool.clone())
            .execute_with_resolved_guard(
                &ctx,
                request.clone(),
                move || {
                    let first_attempts = first_attempts.clone();
                    async move {
                        first_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Err(CommandError::user(codes::AUTH_FORBIDDEN, "guard denied"))
                    }
                },
                |_tx, _guard: ()| {
                    Box::pin(async move { Ok(serde_json::json!({ "unexpected": true })) })
                },
            )
            .await
            .expect_err("guard error should fail command");

        let second_attempts = guard_attempts.clone();
        let replay_error = WriteCommandExecutor::new(pool.clone())
            .execute_with_resolved_guard(
                &ctx,
                request,
                move || {
                    let second_attempts = second_attempts.clone();
                    async move {
                        second_attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok(ResolvedWriteCommandGuard::new((), vec!["lock:unexpected"]))
                    }
                },
                |_tx, _guard: ()| {
                    Box::pin(async move { Ok(serde_json::json!({ "unexpected": true })) })
                },
            )
            .await
            .expect_err("terminal guard error should replay");

        assert_eq!(first_error.code, codes::AUTH_FORBIDDEN);
        assert_eq!(replay_error.code, codes::AUTH_FORBIDDEN);
        assert_eq!(guard_attempts.load(std::sync::atomic::Ordering::SeqCst), 1);

        let status: String = sqlx::query_scalar(
            "SELECT status FROM command_idempotency
             WHERE command_name = ? AND idempotency_key = ?",
        )
        .bind(&ctx.command_name)
        .bind(&ctx.idempotency_key)
        .fetch_one(&pool)
        .await
        .expect("reads command status");
        assert_eq!(status, "failed_terminal");
    }

    #[tokio::test]
    async fn execute_with_resolved_guard_finalizes_retryable_guard_error_as_reclaimable() {
        let pool = test_pool().await;
        let ctx = WriteCommandContext::for_internal_test(
            "test-request-resolved-guard-retryable",
            "idem-resolved-guard-retryable",
            "test.resolved_guard_retryable",
        );
        let request = WriteCommandRequest::new_low_risk(
            serde_json::json!({ "case": "resolved-guard-retryable" }),
            "Resolved guard retryable",
        )
        .expect("request builds");

        let error = WriteCommandExecutor::new(pool.clone())
            .execute_with_resolved_guard(
                &ctx,
                request.clone(),
                || async {
                    Err::<ResolvedWriteCommandGuard<()>, _>(
                        CommandError::system(codes::DB_LOCKED_RETRYABLE, "database is locked")
                            .retryable(true),
                    )
                },
                |_tx, _guard: ()| {
                    Box::pin(async move { Ok(serde_json::json!({ "unexpected": true })) })
                },
            )
            .await
            .expect_err("retryable guard error should fail first attempt");

        assert_eq!(error.code, codes::DB_LOCKED_RETRYABLE);
        assert!(error.retryable);

        let row = sqlx::query(
            "SELECT status, retryable FROM command_idempotency
             WHERE command_name = ? AND idempotency_key = ?",
        )
        .bind(&ctx.command_name)
        .bind(&ctx.idempotency_key)
        .fetch_one(&pool)
        .await
        .expect("reads command row");

        assert_eq!(row.get::<String, _>("status"), "failed_retryable");
        assert_eq!(row.get::<i64, _>("retryable"), 1);

        let result = WriteCommandExecutor::new(pool.clone())
            .execute_with_resolved_guard(
                &ctx,
                request,
                || async { Ok(ResolvedWriteCommandGuard::new((), vec!["booking:B1"])) },
                |_tx, _guard: ()| Box::pin(async move { Ok(serde_json::json!({ "ok": true })) }),
            )
            .await
            .expect("retryable guard error can be reclaimed");

        assert!(!result.replayed);
        assert_eq!(result.response, serde_json::json!({ "ok": true }));
    }

    #[tokio::test]
    async fn execute_with_resolved_guard_lock_key_refresh_failure_finalizes_claim() {
        let pool = test_pool().await;
        let ctx = WriteCommandContext::for_internal_test(
            "test-request-resolved-lock-refresh-fail",
            "idem-resolved-lock-refresh-fail",
            "test.resolved_lock_refresh_fail",
        );
        let request = WriteCommandRequest::new_low_risk(
            serde_json::json!({ "case": "resolved-lock-refresh-fail" }),
            "Resolved lock refresh fail",
        )
        .expect("request builds");

        let error = WriteCommandExecutor::new(pool.clone())
            .execute_with_resolved_guard(
                &ctx,
                request,
                || async { Ok(ResolvedWriteCommandGuard::new((), Vec::<String>::new())) },
                |_tx, _guard: ()| {
                    Box::pin(async move { Ok(serde_json::json!({ "unexpected": true })) })
                },
            )
            .await
            .expect_err("empty resolved lock keys should fail after claim");

        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);

        let status: String = sqlx::query_scalar(
            "SELECT status FROM command_idempotency
             WHERE command_name = ? AND idempotency_key = ?",
        )
        .bind(&ctx.command_name)
        .bind(&ctx.idempotency_key)
        .fetch_one(&pool)
        .await
        .expect("reads command status");

        assert_ne!(status, "in_progress");
    }

    #[tokio::test]
    async fn execute_with_resolved_guard_refreshes_lock_keys_before_transaction() {
        let pool = test_pool().await;
        let ctx = WriteCommandContext::for_internal_test(
            "test-request-resolved-locks",
            "idem-resolved-locks",
            "test.resolved_locks",
        );
        let request = WriteCommandRequest::new_low_risk(
            serde_json::json!({ "case": "resolved-locks" }),
            "Resolved locks",
        )
        .expect("request builds");
        let command_name = ctx.command_name.clone();
        let idempotency_key = ctx.idempotency_key.clone();

        let result = WriteCommandExecutor::new(pool.clone())
            .execute_with_resolved_guard(
                &ctx,
                request,
                || async {
                    Ok(ResolvedWriteCommandGuard::new(
                        "guard-payload".to_string(),
                        vec!["room:R1", "booking:B1", "room:R1"],
                    ))
                },
                move |tx, guard| {
                    Box::pin(async move {
                        let lock_keys_json: String = sqlx::query_scalar(
                            "SELECT lock_keys_json FROM command_idempotency
                             WHERE command_name = ? AND idempotency_key = ?",
                        )
                        .bind(&command_name)
                        .bind(&idempotency_key)
                        .fetch_one(&mut **tx)
                        .await
                        .map_err(system_error)?;

                        Ok(serde_json::json!({
                            "guard": guard,
                            "lock_keys_json": lock_keys_json,
                        }))
                    })
                },
            )
            .await
            .expect("command succeeds");

        assert_eq!(result.response["guard"], "guard-payload");
        assert_eq!(
            result.response["lock_keys_json"],
            "[\"booking:B1\",\"room:R1\"]"
        );
        assert!(!result.replayed);
    }

    #[test]
    fn stable_request_hash_matches_executor_request_hash() {
        let payload = serde_json::json!({
            "schema": "test.hash.v1",
            "nested": {
                "b": 2,
                "a": 1
            }
        });
        let request = WriteCommandRequest::new_low_risk(payload.clone(), "Hash match")
            .expect("request builds");
        let prepared = prepare_write_command_request(request).expect("request prepares");

        assert_eq!(
            stable_request_hash(&payload).expect("payload hashes"),
            prepared.request_hash
        );
    }

    #[tokio::test]
    async fn refresh_claim_lease_write_lock_timeout_fails_claim() {
        let pool = test_pool().await;
        let ctx = WriteCommandContext::for_internal_test(
            "test-request-refresh-lock",
            "idem-refresh-lock",
            "test.refresh_lock",
        );
        let request = WriteCommandRequest::new_low_risk(
            serde_json::json!({ "case": "refresh-lock" }),
            "Refresh lock",
        )
        .expect("request builds");
        let prepared = prepare_write_command_request(request).expect("prepares request");
        let now = Utc::now().to_rfc3339();
        let lease_expires_at =
            (Utc::now() + chrono::Duration::seconds(CLAIM_LEASE_SECONDS)).to_rfc3339();
        let claim_token = "refresh-lock-claim-token";

        sqlx::query(
            "INSERT INTO command_idempotency (
                idempotency_key,
                command_name,
                request_hash,
                intent_json,
                lock_keys_json,
                status,
                claim_token,
                retryable,
                lease_expires_at,
                created_at,
                updated_at,
                last_attempt_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&ctx.idempotency_key)
        .bind(&ctx.command_name)
        .bind(&prepared.request_hash)
        .bind(&prepared.intent_json)
        .bind(&prepared.lock_keys_json)
        .bind(CommandStatus::InProgress.as_str())
        .bind(claim_token)
        .bind(0_i64)
        .bind(&lease_expires_at)
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("seeds in-progress claim");

        let _blocking_tx = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .expect("holds a write lock");

        let error = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            WriteCommandExecutor::new(pool.clone()).refresh_claim_lease(&ctx, claim_token),
        )
        .await
        .expect("refresh returns without waiting past the refresh timeout")
        .expect_err("refresh timeout must fail the claim");

        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
        assert!(error.retryable);
        assert_eq!(error.request_id.as_deref(), Some(ctx.request_id.as_str()));
        assert_eq!(
            error.message,
            "Timed out refreshing claimed idempotency lease"
        );
    }

    #[tokio::test]
    async fn refresh_claim_lease_timeout_fails_claim() {
        let pool = test_pool_with_max_connections(1).await;
        let ctx = WriteCommandContext::for_internal_test(
            "test-request-refresh-timeout",
            "idem-refresh-timeout",
            "test.refresh_timeout",
        );
        let request = WriteCommandRequest::new_low_risk(
            serde_json::json!({ "case": "refresh-timeout" }),
            "Refresh timeout",
        )
        .expect("request builds");
        let prepared = prepare_write_command_request(request).expect("prepares request");
        let now = Utc::now().to_rfc3339();
        let lease_expires_at =
            (Utc::now() + chrono::Duration::seconds(CLAIM_LEASE_SECONDS)).to_rfc3339();
        let claim_token = "refresh-timeout-claim-token";

        sqlx::query(
            "INSERT INTO command_idempotency (
                idempotency_key,
                command_name,
                request_hash,
                intent_json,
                lock_keys_json,
                status,
                claim_token,
                retryable,
                lease_expires_at,
                created_at,
                updated_at,
                last_attempt_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&ctx.idempotency_key)
        .bind(&ctx.command_name)
        .bind(&prepared.request_hash)
        .bind(&prepared.intent_json)
        .bind(&prepared.lock_keys_json)
        .bind(CommandStatus::InProgress.as_str())
        .bind(claim_token)
        .bind(0_i64)
        .bind(&lease_expires_at)
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("seeds in-progress claim");

        let _held_connection = pool.acquire().await.expect("holds the only connection");

        let error = WriteCommandExecutor::new(pool.clone())
            .refresh_claim_lease(&ctx, claim_token)
            .await
            .expect_err("refresh timeout must fail the claim");

        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
        assert!(error.retryable);
        assert_eq!(error.request_id.as_deref(), Some(ctx.request_id.as_str()));
        assert_eq!(
            error.message,
            "Timed out refreshing claimed idempotency lease"
        );
    }

    #[tokio::test]
    async fn set_crash_reporting_preference_exact_retry_returns_cached_response() {
        let pool = test_pool().await;
        let ctx = WriteCommandContext::for_internal_test(
            "test-request-retry",
            "idem-crash-pref-true",
            SET_CRASH_REPORTING_PREFERENCE_COMMAND,
        );

        let first = set_crash_reporting_preference_idempotent(&pool, &ctx, true)
            .await
            .expect("first write succeeds");
        let second = set_crash_reporting_preference_idempotent(&pool, &ctx, true)
            .await
            .expect("exact retry succeeds");

        assert_eq!(first.response, serde_json::json!({ "ok": true }));
        assert!(!first.replayed);
        assert_eq!(second.response, serde_json::json!({ "ok": true }));
        assert!(second.replayed);
    }

    #[tokio::test]
    async fn set_crash_reporting_preference_same_key_different_payload_conflicts() {
        let pool = test_pool().await;
        let ctx = WriteCommandContext::for_internal_test(
            "test-request-conflict",
            "idem-crash-pref-conflict",
            SET_CRASH_REPORTING_PREFERENCE_COMMAND,
        );

        set_crash_reporting_preference_idempotent(&pool, &ctx, true)
            .await
            .expect("first write succeeds");

        let error = set_crash_reporting_preference_idempotent(&pool, &ctx, false)
            .await
            .expect_err("same key with different payload conflicts");

        assert_eq!(error.code, codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH);
        assert_eq!(error.request_id.as_deref(), Some(ctx.request_id.as_str()));
    }

    #[tokio::test]
    async fn set_crash_reporting_preference_same_key_in_progress_conflicts() {
        let pool = test_pool().await;
        let ctx = WriteCommandContext::for_internal_test(
            "test-request-in-flight",
            "idem-crash-pref-in-flight",
            SET_CRASH_REPORTING_PREFERENCE_COMMAND,
        );
        let intent = serde_json::json!({ "enabled": true });
        let now = Utc::now();
        let now_string = now.to_rfc3339();
        let lease_expires_at = (now + chrono::Duration::seconds(CLAIM_LEASE_SECONDS)).to_rfc3339();

        sqlx::query(
            "INSERT INTO command_idempotency (
                idempotency_key,
                command_name,
                request_hash,
                intent_json,
                lock_keys_json,
                status,
                claim_token,
                retryable,
                lease_expires_at,
                created_at,
                updated_at,
                last_attempt_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&ctx.idempotency_key)
        .bind(&ctx.command_name)
        .bind(stable_request_hash(&intent).expect("intent hashes"))
        .bind(stable_json_string(&intent).expect("intent serializes"))
        .bind("[]")
        .bind("in_progress")
        .bind("other-claim-token")
        .bind(0_i64)
        .bind(&lease_expires_at)
        .bind(&now_string)
        .bind(&now_string)
        .bind(&now_string)
        .execute(&pool)
        .await
        .expect("seeds committed in-progress idempotency row");

        let error = set_crash_reporting_preference_idempotent(&pool, &ctx, true)
            .await
            .expect_err("matching in-progress claim conflicts");

        assert_eq!(error.code, codes::CONFLICT_DUPLICATE_IN_FLIGHT);
        assert_eq!(error.request_id.as_deref(), Some(ctx.request_id.as_str()));
    }

    #[tokio::test]
    async fn set_crash_reporting_preference_reclaims_expired_in_progress_claim() {
        let pool = test_pool().await;
        let ctx = WriteCommandContext::for_internal_test(
            "test-request-expired",
            "idem-crash-pref-expired",
            SET_CRASH_REPORTING_PREFERENCE_COMMAND,
        );
        let intent = serde_json::json!({ "enabled": true });
        let now = ctx.issued_at.to_rfc3339();
        let expired_lease = (Utc::now() - chrono::Duration::seconds(1)).to_rfc3339();

        sqlx::query(
            "INSERT INTO command_idempotency (
                idempotency_key,
                command_name,
                request_hash,
                intent_json,
                lock_keys_json,
                status,
                claim_token,
                retryable,
                lease_expires_at,
                created_at,
                updated_at,
                last_attempt_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&ctx.idempotency_key)
        .bind(&ctx.command_name)
        .bind(stable_request_hash(&intent).expect("intent hashes"))
        .bind(stable_json_string(&intent).expect("intent serializes"))
        .bind("[]")
        .bind("in_progress")
        .bind("expired-claim-token")
        .bind(0_i64)
        .bind(&expired_lease)
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .execute(&pool)
        .await
        .expect("seeds expired in-progress idempotency row");

        let result = set_crash_reporting_preference_idempotent(&pool, &ctx, true)
            .await
            .expect("expired claim is reclaimed");

        assert!(!result.replayed);
        assert_eq!(result.response, serde_json::json!({ "ok": true }));

        let row = sqlx::query(
            "SELECT status, response_json, lease_expires_at
             FROM command_idempotency
             WHERE command_name = ? AND idempotency_key = ?",
        )
        .bind(&ctx.command_name)
        .bind(&ctx.idempotency_key)
        .fetch_one(&pool)
        .await
        .expect("reads reclaimed row");

        assert_eq!(row.get::<String, _>("status"), "completed");
        assert_eq!(
            row.get::<Option<String>, _>("response_json"),
            Some(serde_json::json!({ "ok": true }).to_string())
        );
        assert_eq!(row.get::<Option<String>, _>("lease_expires_at"), None);

        let setting: String =
            sqlx::query_scalar("SELECT value FROM settings WHERE key = 'send_crash_reports'")
                .fetch_one(&pool)
                .await
                .expect("reads crash reporting setting");
        assert_eq!(setting, "true");
    }

    #[tokio::test]
    async fn write_command_executor_terminal_failure_returns_cached_error_without_rerun() {
        let pool = test_pool().await;
        let ctx = WriteCommandContext::for_internal_test(
            "test-request-terminal-failure",
            "idem-terminal-failure",
            "test.terminal_failure",
        );
        let request = WriteCommandRequest::new_low_risk(
            serde_json::json!({ "case": "terminal" }),
            "Terminal failure",
        )
        .expect("request builds");
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let first_attempts = attempts.clone();
        let first_error = WriteCommandExecutor::new(pool.clone())
            .execute(&ctx, request.clone(), move |_tx| {
                let attempts = first_attempts.clone();
                Box::pin(async move {
                    attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Err(CommandError::user(
                        codes::AUTH_FORBIDDEN,
                        "terminal failure",
                    ))
                })
            })
            .await
            .expect_err("first terminal failure is returned");

        let second_attempts = attempts.clone();
        let second_error = WriteCommandExecutor::new(pool.clone())
            .execute(&ctx, request, move |_tx| {
                let attempts = second_attempts.clone();
                Box::pin(async move {
                    attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(serde_json::json!({ "unexpected": true }))
                })
            })
            .await
            .expect_err("terminal failure is replayed");

        assert_eq!(first_error.code, codes::AUTH_FORBIDDEN);
        assert_eq!(
            first_error.request_id.as_deref(),
            Some(ctx.request_id.as_str())
        );
        assert_eq!(second_error.code, codes::AUTH_FORBIDDEN);
        assert_eq!(second_error.message, "terminal failure");
        assert_eq!(
            second_error.request_id.as_deref(),
            Some(ctx.request_id.as_str())
        );
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 1);

        let row = sqlx::query(
            "SELECT status, error_code, error_json, retryable, lease_expires_at, completed_at
             FROM command_idempotency
             WHERE command_name = ? AND idempotency_key = ?",
        )
        .bind(&ctx.command_name)
        .bind(&ctx.idempotency_key)
        .fetch_one(&pool)
        .await
        .expect("reads failed row");

        assert_eq!(row.get::<String, _>("status"), "failed_terminal");
        assert_eq!(
            row.get::<Option<String>, _>("error_code"),
            Some(codes::AUTH_FORBIDDEN.to_string())
        );
        assert_eq!(row.get::<i64, _>("retryable"), 0);
        assert_eq!(row.get::<Option<String>, _>("lease_expires_at"), None);
        assert!(row.get::<Option<String>, _>("completed_at").is_some());
        assert!(row
            .get::<Option<String>, _>("error_json")
            .expect("error_json is stored")
            .contains("terminal failure"));
    }

    #[tokio::test]
    async fn write_command_executor_retryable_failure_reclaims_and_reruns() {
        let pool = test_pool().await;
        let ctx = WriteCommandContext::for_internal_test(
            "test-request-retryable-failure",
            "idem-retryable-failure",
            "test.retryable_failure",
        );
        let request = WriteCommandRequest::new_low_risk(
            serde_json::json!({ "case": "retryable" }),
            "Retryable failure",
        )
        .expect("request builds");
        let attempts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let first_attempts = attempts.clone();
        let first_error = WriteCommandExecutor::new(pool.clone())
            .execute(&ctx, request.clone(), move |_tx| {
                let attempts = first_attempts.clone();
                Box::pin(async move {
                    attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Err(
                        CommandError::system(codes::DB_LOCKED_RETRYABLE, "database locked")
                            .retryable(true),
                    )
                })
            })
            .await
            .expect_err("first retryable failure is returned");

        assert_eq!(first_error.code, codes::DB_LOCKED_RETRYABLE);
        assert!(first_error.retryable);

        let failed_row = sqlx::query(
            "SELECT status, completed_at
             FROM command_idempotency
             WHERE command_name = ? AND idempotency_key = ?",
        )
        .bind(&ctx.command_name)
        .bind(&ctx.idempotency_key)
        .fetch_one(&pool)
        .await
        .expect("reads failed retryable row");

        assert_eq!(failed_row.get::<String, _>("status"), "failed_retryable");
        assert_eq!(failed_row.get::<Option<String>, _>("completed_at"), None);

        let second_attempts = attempts.clone();
        let second = WriteCommandExecutor::new(pool.clone())
            .execute(&ctx, request, move |_tx| {
                let attempts = second_attempts.clone();
                Box::pin(async move {
                    attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Ok(serde_json::json!({ "ok": true }))
                })
            })
            .await
            .expect("retryable failure is reclaimed and rerun");

        assert_eq!(second.response, serde_json::json!({ "ok": true }));
        assert!(!second.replayed);
        assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 2);

        let row = sqlx::query(
            "SELECT status, retryable, error_json, lease_expires_at
             FROM command_idempotency
             WHERE command_name = ? AND idempotency_key = ?",
        )
        .bind(&ctx.command_name)
        .bind(&ctx.idempotency_key)
        .fetch_one(&pool)
        .await
        .expect("reads completed row");

        assert_eq!(row.get::<String, _>("status"), "completed");
        assert_eq!(row.get::<i64, _>("retryable"), 0);
        assert_eq!(row.get::<Option<String>, _>("error_json"), None);
        assert_eq!(row.get::<Option<String>, _>("lease_expires_at"), None);
    }

    #[tokio::test]
    async fn write_command_executor_stale_claimant_finalize_rolls_back_business_tx() {
        let pool = test_pool().await;
        let ctx = WriteCommandContext::for_internal_test(
            "test-request-stale-claimant",
            "idem-stale-claimant",
            "test.stale_claimant",
        );
        let request = WriteCommandRequest::new_low_risk(
            serde_json::json!({ "case": "stale" }),
            "Stale claim",
        )
        .expect("request builds");
        let command_name = ctx.command_name.clone();
        let idempotency_key = ctx.idempotency_key.clone();

        let error = WriteCommandExecutor::new(pool.clone())
            .execute(&ctx, request, |tx| {
                Box::pin(async move {
                    settings_store::save_setting_tx(tx, "send_crash_reports", "true")
                        .await
                        .map_err(system_error)?;

                    sqlx::query(
                        "UPDATE command_idempotency
                         SET claim_token = 'stale-claim-token'
                         WHERE command_name = ? AND idempotency_key = ?",
                    )
                    .bind(&command_name)
                    .bind(&idempotency_key)
                    .execute(&mut **tx)
                    .await
                    .map_err(system_error)?;

                    Ok(serde_json::json!({ "ok": true }))
                })
            })
            .await
            .expect_err("stale claimant cannot finalize");

        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
        assert_eq!(error.request_id.as_deref(), Some(ctx.request_id.as_str()));

        let setting: Option<String> =
            sqlx::query_scalar("SELECT value FROM settings WHERE key = 'send_crash_reports'")
                .fetch_optional(&pool)
                .await
                .expect("reads crash reporting setting");
        assert_eq!(setting, None);

        let row = sqlx::query(
            "SELECT status, response_json
             FROM command_idempotency
             WHERE command_name = ? AND idempotency_key = ?",
        )
        .bind(&ctx.command_name)
        .bind(&ctx.idempotency_key)
        .fetch_one(&pool)
        .await
        .expect("reads idempotency row after rollback");

        assert_eq!(row.get::<String, _>("status"), "in_progress");
        assert_eq!(row.get::<Option<String>, _>("response_json"), None);
    }
}
