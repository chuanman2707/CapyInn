use crate::{
    app_error::{codes, CommandError, CommandResult},
    command_idempotency::{system_error, WriteCommandContext},
};
use chrono::{Duration, Utc};
use log::{error, info};
use serde_json::{json, Value};
use sqlx::{sqlite::SqliteRow, Pool, Row, Sqlite, Transaction};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration as StdDuration,
};
use tokio::sync::oneshot;

const OUTBOX_STATUS_PENDING: &str = "pending";
const OUTBOX_STATUS_PROCESSING: &str = "processing";
const OUTBOX_STATUS_DISPATCHED: &str = "dispatched";
const OUTBOX_STATUS_FAILED: &str = "failed";
const OUTBOX_SAFE_TEXT_MAX_CHARS: usize = 160;
const OUTBOX_ERROR_MAX_CHARS: usize = 512;
const OUTBOX_RETRY_LIMIT_ERROR: &str = "outbox retry limit reached";
const CLAIM_NEXT_OUTBOX_EVENT_CANDIDATE_SQL: &str = "
SELECT id
FROM (
    SELECT candidate.id AS id
    FROM outbox_events candidate INDEXED BY outbox_events_pending_idx
    WHERE candidate.status = 'pending'
      AND candidate.attempts < ?
      AND candidate.next_attempt_at IS NULL
      AND NOT EXISTS (
          SELECT 1
          FROM outbox_events older
          WHERE older.aggregate_key = candidate.aggregate_key
            AND older.id < candidate.id
            AND older.status IN ('pending', 'processing')
      )
    UNION ALL
    SELECT candidate.id AS id
    FROM outbox_events candidate INDEXED BY outbox_events_pending_idx
    WHERE candidate.status = 'pending'
      AND candidate.attempts < ?
      AND candidate.next_attempt_at <= ?
      AND NOT EXISTS (
          SELECT 1
          FROM outbox_events older
          WHERE older.aggregate_key = candidate.aggregate_key
            AND older.id < candidate.id
            AND older.status IN ('pending', 'processing')
      )
    UNION ALL
    SELECT candidate.id AS id
    FROM outbox_events candidate INDEXED BY outbox_events_processing_idx
    WHERE candidate.status = 'processing'
      AND candidate.attempts < ?
      AND candidate.processing_expires_at <= ?
      AND NOT EXISTS (
          SELECT 1
          FROM outbox_events older
          WHERE older.aggregate_key = candidate.aggregate_key
            AND older.id < candidate.id
            AND older.status IN ('pending', 'processing')
      )
)
ORDER BY id
LIMIT 1";
const FAIL_RETRY_LIMIT_PENDING_NULL_SQL: &str = "
UPDATE outbox_events INDEXED BY outbox_events_pending_idx
SET status = ?,
    worker_token = NULL,
    next_attempt_at = NULL,
    processing_started_at = NULL,
    processing_expires_at = NULL,
    last_error = ?
WHERE status = 'pending'
  AND attempts >= ?
  AND next_attempt_at IS NULL";
const FAIL_RETRY_LIMIT_PENDING_DUE_SQL: &str = "
UPDATE outbox_events INDEXED BY outbox_events_pending_idx
SET status = ?,
    worker_token = NULL,
    next_attempt_at = NULL,
    processing_started_at = NULL,
    processing_expires_at = NULL,
    last_error = ?
WHERE status = 'pending'
  AND attempts >= ?
  AND next_attempt_at <= ?";
const FAIL_RETRY_LIMIT_PROCESSING_SQL: &str = "
UPDATE outbox_events INDEXED BY outbox_events_processing_idx
SET status = ?,
    worker_token = NULL,
    next_attempt_at = NULL,
    processing_started_at = NULL,
    processing_expires_at = NULL,
    last_error = ?
WHERE status = 'processing'
  AND attempts >= ?
  AND processing_expires_at <= ?";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutboxDispatchConfig {
    pub max_attempts: i64,
    pub processing_lease_seconds: i64,
    pub batch_size: usize,
}

impl Default for OutboxDispatchConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            processing_lease_seconds: 30,
            batch_size: 25,
        }
    }
}

impl OutboxDispatchConfig {
    #[cfg(test)]
    fn test() -> Self {
        Self {
            max_attempts: 5,
            processing_lease_seconds: 30,
            batch_size: 10,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxEvent {
    pub id: i64,
    pub event_type: String,
    pub aggregate_key: String,
    pub payload_json: String,
    pub origin_request_id: String,
    pub origin_idempotency_key: String,
    pub origin_command_name: String,
    pub origin_request_hash: String,
    pub created_at: String,
    pub attempts: i64,
    pub worker_token: String,
}

pub type OutboxSubscriberFuture<'a> = Pin<Box<dyn Future<Output = CommandResult<()>> + Send + 'a>>;

pub trait OutboxSubscriber: Send + Sync {
    fn name(&self) -> &'static str;
    fn handle<'a>(&'a self, event: &'a OutboxEvent) -> OutboxSubscriberFuture<'a>;
}

pub struct OutboxDispatcherHandle {
    task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
}

impl OutboxDispatcherHandle {
    pub fn inactive() -> Self {
        Self {
            task: Mutex::new(None),
            shutdown_tx: Mutex::new(None),
        }
    }

    fn new(task: tauri::async_runtime::JoinHandle<()>, shutdown_tx: oneshot::Sender<()>) -> Self {
        Self {
            task: Mutex::new(Some(task)),
            shutdown_tx: Mutex::new(Some(shutdown_tx)),
        }
    }

    pub fn is_active(&self) -> bool {
        self.task
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
    }

    pub fn shutdown(&self) {
        let shutdown_tx = self
            .shutdown_tx
            .lock()
            .ok()
            .and_then(|mut guard| guard.take());

        if let Some(shutdown_tx) = shutdown_tx {
            let _ = shutdown_tx.send(());
        }

        let _task = self.task.lock().ok().and_then(|mut guard| guard.take());
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OutboxDispatchSummary {
    pub claimed: u64,
    pub dispatched: u64,
    pub retried: u64,
    pub failed: u64,
    pub retry_limit_failed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxAggregateKeySource {
    PrimaryAggregateKey,
    ResponseField {
        aggregate_type: &'static str,
        field: &'static str,
    },
}

impl OutboxAggregateKeySource {
    pub const fn primary() -> Self {
        Self::PrimaryAggregateKey
    }

    pub const fn response_field(aggregate_type: &'static str, field: &'static str) -> Self {
        Self::ResponseField {
            aggregate_type,
            field,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboxEventSpec {
    event_type: &'static str,
    aggregate_key_source: OutboxAggregateKeySource,
    refresh: Vec<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedOutboxEvent {
    pub event_type: String,
    pub aggregate_key: String,
    pub payload_json: String,
    pub origin_request_id: String,
    pub origin_idempotency_key: String,
    pub origin_command_name: String,
    pub origin_request_hash: String,
    pub created_at: String,
}

pub async fn insert_outbox_event_tx(
    tx: &mut Transaction<'_, Sqlite>,
    event: &PreparedOutboxEvent,
) -> CommandResult<()> {
    validate_prepared_event(event)?;

    sqlx::query(
        "INSERT INTO outbox_events (
            event_type,
            aggregate_key,
            payload_json,
            origin_request_id,
            origin_idempotency_key,
            origin_command_name,
            origin_request_hash,
            status,
            attempts,
            created_at
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&event.event_type)
    .bind(&event.aggregate_key)
    .bind(&event.payload_json)
    .bind(&event.origin_request_id)
    .bind(&event.origin_idempotency_key)
    .bind(&event.origin_command_name)
    .bind(&event.origin_request_hash)
    .bind(OUTBOX_STATUS_PENDING)
    .bind(0_i64)
    .bind(&event.created_at)
    .execute(&mut **tx)
    .await
    .map_err(system_error)?;

    Ok(())
}

pub async fn fail_retry_limit_rows(
    pool: &Pool<Sqlite>,
    config: OutboxDispatchConfig,
) -> CommandResult<u64> {
    let now = Utc::now().to_rfc3339();
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(system_error)?;

    let pending_null_result = sqlx::query(FAIL_RETRY_LIMIT_PENDING_NULL_SQL)
        .bind(OUTBOX_STATUS_FAILED)
        .bind(OUTBOX_RETRY_LIMIT_ERROR)
        .bind(config.max_attempts)
        .execute(&mut *tx)
        .await
        .map_err(system_error)?;
    let pending_due_result = sqlx::query(FAIL_RETRY_LIMIT_PENDING_DUE_SQL)
        .bind(OUTBOX_STATUS_FAILED)
        .bind(OUTBOX_RETRY_LIMIT_ERROR)
        .bind(config.max_attempts)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(system_error)?;
    let processing_result = sqlx::query(FAIL_RETRY_LIMIT_PROCESSING_SQL)
        .bind(OUTBOX_STATUS_FAILED)
        .bind(OUTBOX_RETRY_LIMIT_ERROR)
        .bind(config.max_attempts)
        .bind(&now)
        .execute(&mut *tx)
        .await
        .map_err(system_error)?;

    tx.commit().await.map_err(system_error)?;

    Ok(pending_null_result.rows_affected()
        + pending_due_result.rows_affected()
        + processing_result.rows_affected())
}

pub async fn claim_next_outbox_event(
    pool: &Pool<Sqlite>,
    config: OutboxDispatchConfig,
) -> CommandResult<Option<OutboxEvent>> {
    let now = Utc::now();
    let now_string = now.to_rfc3339();
    let processing_expires_at =
        (now + Duration::seconds(config.processing_lease_seconds)).to_rfc3339();
    let worker_token = uuid::Uuid::new_v4().to_string();
    let mut tx = pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(system_error)?;

    let result = async {
        let claimable_id = sqlx::query_scalar::<_, i64>(CLAIM_NEXT_OUTBOX_EVENT_CANDIDATE_SQL)
            .bind(config.max_attempts)
            .bind(config.max_attempts)
            .bind(&now_string)
            .bind(config.max_attempts)
            .bind(&now_string)
            .fetch_optional(&mut *tx)
            .await
            .map_err(system_error)?;

        let Some(claimable_id) = claimable_id else {
            return Ok(None);
        };

        sqlx::query(
            "UPDATE outbox_events
             SET status = ?,
                 worker_token = ?,
                 attempts = attempts + 1,
                 processing_started_at = ?,
                 processing_expires_at = ?,
                 last_error = NULL
             WHERE id = ?",
        )
        .bind(OUTBOX_STATUS_PROCESSING)
        .bind(&worker_token)
        .bind(&now_string)
        .bind(&processing_expires_at)
        .bind(claimable_id)
        .execute(&mut *tx)
        .await
        .map_err(system_error)?;

        let row = sqlx::query(
            "SELECT
                id,
                event_type,
                aggregate_key,
                payload_json,
                origin_request_id,
                origin_idempotency_key,
                origin_command_name,
                origin_request_hash,
                created_at,
                attempts,
                worker_token
             FROM outbox_events
             WHERE id = ?",
        )
        .bind(claimable_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(system_error)?;

        Ok(Some(outbox_event_from_row(row)?))
    }
    .await;

    match result {
        Ok(event) => {
            tx.commit().await.map_err(system_error)?;
            Ok(event)
        }
        Err(error) => {
            tx.rollback().await.map_err(system_error)?;
            Err(error)
        }
    }
}

pub async fn mark_outbox_dispatched(
    pool: &Pool<Sqlite>,
    id: i64,
    worker_token: &str,
) -> CommandResult<bool> {
    let now = Utc::now().to_rfc3339();
    let result = sqlx::query(
        "UPDATE outbox_events
         SET status = ?,
             worker_token = NULL,
             next_attempt_at = NULL,
             processing_started_at = NULL,
             processing_expires_at = NULL,
             last_error = NULL,
             dispatched_at = ?
         WHERE id = ?
           AND status = ?
           AND worker_token = ?",
    )
    .bind(OUTBOX_STATUS_DISPATCHED)
    .bind(&now)
    .bind(id)
    .bind(OUTBOX_STATUS_PROCESSING)
    .bind(worker_token)
    .execute(pool)
    .await
    .map_err(system_error)?;

    Ok(result.rows_affected() == 1)
}

pub async fn record_outbox_failure(
    pool: &Pool<Sqlite>,
    event: &OutboxEvent,
    error: &CommandError,
    config: OutboxDispatchConfig,
) -> CommandResult<bool> {
    let terminal_failure = event.attempts >= config.max_attempts;
    let status = if terminal_failure {
        OUTBOX_STATUS_FAILED
    } else {
        OUTBOX_STATUS_PENDING
    };
    let next_attempt_at = if terminal_failure {
        None
    } else {
        Some(next_attempt_at_for_attempt(event.attempts))
    };
    let last_error = sanitize_outbox_error(error);

    let result = sqlx::query(
        "UPDATE outbox_events
         SET status = ?,
             worker_token = NULL,
             next_attempt_at = ?,
             processing_started_at = NULL,
             processing_expires_at = NULL,
             last_error = ?
         WHERE id = ?
           AND status = ?
           AND worker_token = ?",
    )
    .bind(status)
    .bind(next_attempt_at)
    .bind(last_error)
    .bind(event.id)
    .bind(OUTBOX_STATUS_PROCESSING)
    .bind(&event.worker_token)
    .execute(pool)
    .await
    .map_err(system_error)?;

    Ok(result.rows_affected() == 1)
}

pub async fn run_outbox_dispatch_batch(
    pool: &Pool<Sqlite>,
    subscribers: &[&dyn OutboxSubscriber],
    config: OutboxDispatchConfig,
) -> CommandResult<OutboxDispatchSummary> {
    if subscribers.is_empty() {
        return Ok(OutboxDispatchSummary::default());
    }

    let mut summary = OutboxDispatchSummary {
        retry_limit_failed: fail_retry_limit_rows(pool, config).await?,
        ..OutboxDispatchSummary::default()
    };

    for _ in 0..config.batch_size {
        let Some(event) = claim_next_outbox_event(pool, config).await? else {
            break;
        };

        summary.claimed += 1;
        match dispatch_claimed_event(pool, event, subscribers, config).await? {
            ClaimedDispatchOutcome::Dispatched => summary.dispatched += 1,
            ClaimedDispatchOutcome::Retried => summary.retried += 1,
            ClaimedDispatchOutcome::Failed => summary.failed += 1,
            ClaimedDispatchOutcome::Stale => {}
        }
    }

    Ok(summary)
}

pub fn start_outbox_dispatcher(
    pool: Pool<Sqlite>,
    subscribers: Vec<Arc<dyn OutboxSubscriber>>,
) -> OutboxDispatcherHandle {
    if subscribers.is_empty() {
        info!("Outbox dispatcher inactive: no subscribers registered");
        return OutboxDispatcherHandle::inactive();
    }

    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let task = tauri::async_runtime::spawn(async move {
        run_outbox_dispatcher_loop(pool, subscribers, shutdown_rx).await;
    });

    OutboxDispatcherHandle::new(task, shutdown_tx)
}

pub async fn run_outbox_startup_recovery_once(
    pool: &Pool<Sqlite>,
    subscribers: &[Arc<dyn OutboxSubscriber>],
    config: OutboxDispatchConfig,
) -> CommandResult<OutboxDispatchSummary> {
    let subscriber_refs = subscribers
        .iter()
        .map(|subscriber| subscriber.as_ref())
        .collect::<Vec<&dyn OutboxSubscriber>>();
    run_outbox_dispatch_batch(pool, &subscriber_refs, config).await
}

async fn run_outbox_dispatcher_loop(
    pool: Pool<Sqlite>,
    subscribers: Vec<Arc<dyn OutboxSubscriber>>,
    mut shutdown_rx: oneshot::Receiver<()>,
) {
    run_default_outbox_dispatch_batch(&pool, &subscribers).await;

    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown_rx => {
                break;
            }
            _ = tokio::time::sleep(StdDuration::from_secs(5)) => {
                run_default_outbox_dispatch_batch(&pool, &subscribers).await;
            }
        }
    }
}

async fn run_default_outbox_dispatch_batch(
    pool: &Pool<Sqlite>,
    subscribers: &[Arc<dyn OutboxSubscriber>],
) {
    let subscriber_refs = subscribers
        .iter()
        .map(|subscriber| subscriber.as_ref())
        .collect::<Vec<&dyn OutboxSubscriber>>();

    if let Err(error) =
        run_outbox_dispatch_batch(pool, &subscriber_refs, OutboxDispatchConfig::default()).await
    {
        error!(
            "Outbox dispatcher batch failed: code={} message={}",
            error.code, error.message
        );
    }
}

impl OutboxEventSpec {
    pub fn new(
        event_type: &'static str,
        aggregate_key_source: OutboxAggregateKeySource,
        refresh: &'static [&'static str],
    ) -> CommandResult<Self> {
        validate_safe_outbox_text(event_type)?;
        validate_aggregate_key_source(&aggregate_key_source)?;
        if refresh.is_empty() {
            return Err(system_error("outbox refresh areas are required"));
        }
        for area in refresh {
            validate_safe_outbox_text(area)?;
        }
        Ok(Self {
            event_type,
            aggregate_key_source,
            refresh: refresh.to_vec(),
        })
    }

    pub fn prepare(
        &self,
        ctx: &WriteCommandContext,
        primary_aggregate_key: Option<&str>,
        origin_request_hash: &str,
        response: &Value,
    ) -> CommandResult<PreparedOutboxEvent> {
        let (aggregate_type, aggregate_id, aggregate_key) =
            resolve_aggregate(&self.aggregate_key_source, primary_aggregate_key, response)?;

        validate_safe_outbox_text(&ctx.command_name)?;
        validate_safe_outbox_identifier(origin_request_hash)?;

        let payload = json!({
            "schema_version": 1,
            "command_name": ctx.command_name,
            "aggregate": {
                "type": aggregate_type,
                "id": aggregate_id,
            },
            "refresh": self.refresh,
        });
        let payload_json = canonical_json_string(&payload)?;

        Ok(PreparedOutboxEvent {
            event_type: self.event_type.to_string(),
            aggregate_key,
            payload_json,
            origin_request_id: validate_non_empty(ctx.request_id.clone(), "origin_request_id")?,
            origin_idempotency_key: validate_non_empty(
                ctx.idempotency_key.clone(),
                "origin_idempotency_key",
            )?,
            origin_command_name: ctx.command_name.clone(),
            origin_request_hash: origin_request_hash.to_string(),
            created_at: Utc::now().to_rfc3339(),
        })
    }
}

fn resolve_aggregate(
    source: &OutboxAggregateKeySource,
    primary_aggregate_key: Option<&str>,
    response: &Value,
) -> CommandResult<(String, String, String)> {
    match source {
        OutboxAggregateKeySource::PrimaryAggregateKey => {
            let aggregate_key = primary_aggregate_key
                .ok_or_else(|| system_error("outbox event missing primary aggregate key"))?;
            let (aggregate_type, aggregate_id) = aggregate_key
                .split_once(':')
                .ok_or_else(|| system_error("outbox primary aggregate key must be type:id"))?;
            validate_safe_outbox_text(aggregate_type)?;
            validate_safe_outbox_identifier(aggregate_id)?;
            validate_safe_outbox_identifier(aggregate_key)?;
            Ok((
                aggregate_type.to_string(),
                aggregate_id.to_string(),
                aggregate_key.to_string(),
            ))
        }
        OutboxAggregateKeySource::ResponseField {
            aggregate_type,
            field,
        } => {
            let aggregate_id = response
                .get(*field)
                .and_then(Value::as_str)
                .ok_or_else(|| system_error(format!("outbox response missing field {field}")))?;
            validate_safe_outbox_text(aggregate_type)?;
            validate_safe_outbox_identifier(aggregate_id)?;
            Ok((
                aggregate_type.to_string(),
                aggregate_id.to_string(),
                format!("{aggregate_type}:{aggregate_id}"),
            ))
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClaimedDispatchOutcome {
    Dispatched,
    Retried,
    Failed,
    Stale,
}

async fn dispatch_claimed_event(
    pool: &Pool<Sqlite>,
    event: OutboxEvent,
    subscribers: &[&dyn OutboxSubscriber],
    config: OutboxDispatchConfig,
) -> CommandResult<ClaimedDispatchOutcome> {
    for subscriber in subscribers {
        if let Err(error) = subscriber.handle(&event).await {
            let terminal_failure = event.attempts >= config.max_attempts;
            let finalized = record_outbox_failure(pool, &event, &error, config).await?;
            if !finalized {
                return Ok(ClaimedDispatchOutcome::Stale);
            }
            return Ok(if terminal_failure {
                ClaimedDispatchOutcome::Failed
            } else {
                ClaimedDispatchOutcome::Retried
            });
        }
    }

    if mark_outbox_dispatched(pool, event.id, &event.worker_token).await? {
        Ok(ClaimedDispatchOutcome::Dispatched)
    } else {
        Ok(ClaimedDispatchOutcome::Stale)
    }
}

fn validate_aggregate_key_source(source: &OutboxAggregateKeySource) -> CommandResult<()> {
    match source {
        OutboxAggregateKeySource::PrimaryAggregateKey => Ok(()),
        OutboxAggregateKeySource::ResponseField {
            aggregate_type,
            field,
        } => {
            validate_safe_outbox_text(aggregate_type)?;
            validate_safe_outbox_text(field)
        }
    }
}

fn validate_prepared_event(event: &PreparedOutboxEvent) -> CommandResult<()> {
    validate_safe_outbox_text(&event.event_type)?;
    validate_safe_outbox_identifier(&event.aggregate_key)?;
    validate_non_empty(event.origin_request_id.clone(), "origin_request_id")?;
    validate_non_empty(
        event.origin_idempotency_key.clone(),
        "origin_idempotency_key",
    )?;
    validate_safe_outbox_text(&event.origin_command_name)?;
    validate_safe_outbox_identifier(&event.origin_request_hash)?;
    validate_non_empty(event.created_at.clone(), "created_at")?;
    serde_json::from_str::<Value>(&event.payload_json)
        .map_err(|error| CommandError::system(codes::SYSTEM_INTERNAL_ERROR, error.to_string()))?;
    Ok(())
}

fn validate_non_empty(value: String, field: &str) -> CommandResult<String> {
    let trimmed = value.trim().to_string();
    if trimmed.is_empty() {
        return Err(system_error(format!("{field} is required")));
    }
    Ok(trimmed)
}

fn validate_safe_outbox_text(value: &str) -> CommandResult<()> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > OUTBOX_SAFE_TEXT_MAX_CHARS
        || trimmed.contains('@')
        || contains_forbidden_payload_term(trimmed)
    {
        return Err(system_error("unsafe outbox text"));
    }
    Ok(())
}

fn validate_safe_outbox_identifier(value: &str) -> CommandResult<()> {
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > OUTBOX_SAFE_TEXT_MAX_CHARS
        || trimmed.contains('@')
    {
        return Err(system_error("unsafe outbox text"));
    }
    Ok(())
}

fn contains_forbidden_payload_term(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "phone", "email", "card", "token", "secret", "password", "passport", "cccd", "prompt",
        "raw",
    ]
    .iter()
    .any(|term| lower.contains(term))
}

fn sanitize_outbox_error(error: &CommandError) -> String {
    let safe_code = error.code.trim();
    let safe_code = if validate_safe_outbox_text(safe_code).is_ok() {
        safe_code
    } else {
        codes::SYSTEM_INTERNAL_ERROR
    };
    format!("{safe_code}: subscriber delivery failed")
        .chars()
        .take(OUTBOX_ERROR_MAX_CHARS)
        .collect()
}

fn next_attempt_at_for_attempt(attempts: i64) -> String {
    let delay_seconds = match attempts {
        i64::MIN..=1 => 1,
        2 => 5,
        3 => 15,
        _ => 30,
    };
    (Utc::now() + Duration::seconds(delay_seconds)).to_rfc3339()
}

fn canonicalize_json_value(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries = map.into_iter().collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            let mut canonical = serde_json::Map::new();
            for (key, value) in entries {
                canonical.insert(key, canonicalize_json_value(value));
            }
            Value::Object(canonical)
        }
        Value::Array(values) => Value::Array(
            values
                .into_iter()
                .map(canonicalize_json_value)
                .collect::<Vec<_>>(),
        ),
        primitive => primitive,
    }
}

fn canonical_json_string(value: &Value) -> CommandResult<String> {
    serde_json::to_string(&canonicalize_json_value(value.clone())).map_err(system_error)
}

fn outbox_event_from_row(row: SqliteRow) -> CommandResult<OutboxEvent> {
    let worker_token = row
        .try_get::<Option<String>, _>("worker_token")
        .map_err(system_error)?
        .ok_or_else(|| system_error("claimed outbox event missing worker token"))?;

    Ok(OutboxEvent {
        id: row.try_get("id").map_err(system_error)?,
        event_type: row.try_get("event_type").map_err(system_error)?,
        aggregate_key: row.try_get("aggregate_key").map_err(system_error)?,
        payload_json: row.try_get("payload_json").map_err(system_error)?,
        origin_request_id: row.try_get("origin_request_id").map_err(system_error)?,
        origin_idempotency_key: row
            .try_get("origin_idempotency_key")
            .map_err(system_error)?,
        origin_command_name: row.try_get("origin_command_name").map_err(system_error)?,
        origin_request_hash: row.try_get("origin_request_hash").map_err(system_error)?,
        created_at: row.try_get("created_at").map_err(system_error)?,
        attempts: row.try_get("attempts").map_err(system_error)?,
        worker_token,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_idempotency::WriteCommandContext;
    use chrono::DateTime;
    use serde_json::json;
    use sqlx::{sqlite::SqlitePoolOptions, Row, Sqlite};

    async fn test_pool() -> sqlx::Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite connects");
        crate::db::run_migrations(&pool)
            .await
            .expect("migrations run");
        pool
    }

    async fn seed_outbox_event(
        pool: &sqlx::Pool<Sqlite>,
        event_type: &str,
        aggregate_key: &str,
        status: &str,
        attempts: i64,
        next_attempt_at: Option<&str>,
        processing_expires_at: Option<&str>,
    ) -> i64 {
        let result = sqlx::query(
            "INSERT INTO outbox_events (
                event_type, aggregate_key, payload_json,
                origin_request_id, origin_idempotency_key,
                origin_command_name, origin_request_hash,
                status, attempts, next_attempt_at,
                processing_started_at, processing_expires_at,
                created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(event_type)
        .bind(aggregate_key)
        .bind(r#"{"schema_version":1}"#)
        .bind(format!("req-{event_type}-{aggregate_key}"))
        .bind(format!("idem-{event_type}-{aggregate_key}"))
        .bind("test.command")
        .bind("hash-test")
        .bind(status)
        .bind(attempts)
        .bind(next_attempt_at)
        .bind(if status == "processing" {
            Some("2026-05-03T08:59:00+00:00")
        } else {
            None
        })
        .bind(processing_expires_at)
        .bind("2026-05-03T09:00:00+00:00")
        .execute(pool)
        .await
        .expect("seeds outbox event");

        result.last_insert_rowid()
    }

    fn test_ctx(command_name: &str) -> WriteCommandContext {
        let issued_at = DateTime::parse_from_rfc3339("2026-05-03T09:00:00+07:00")
            .expect("fixed test timestamp parses");
        WriteCommandContext {
            request_id: "req-outbox".to_string(),
            idempotency_key: "idem-outbox".to_string(),
            command_name: command_name.to_string(),
            actor_id: Some("operator-1".to_string()),
            actor_type: crate::command_idempotency::ActorType::Human,
            client_id: None,
            session_id: None,
            channel_id: None,
            issued_at,
        }
    }

    #[derive(Default)]
    struct RecordingSubscriber {
        calls: std::sync::Arc<std::sync::Mutex<Vec<i64>>>,
        fail: bool,
        error_message: Option<String>,
    }

    impl OutboxSubscriber for RecordingSubscriber {
        fn name(&self) -> &'static str {
            "recording"
        }

        fn handle<'a>(&'a self, event: &'a OutboxEvent) -> OutboxSubscriberFuture<'a> {
            Box::pin(async move {
                self.calls.lock().expect("calls lock").push(event.id);
                if self.fail {
                    return Err(CommandError::system(
                        codes::SYSTEM_INTERNAL_ERROR,
                        self.error_message.as_deref().unwrap_or("subscriber failed"),
                    ));
                }
                Ok(())
            })
        }
    }

    #[test]
    fn dispatcher_handle_is_inactive_without_subscribers() {
        let handle = OutboxDispatcherHandle::inactive();
        assert!(!handle.is_active());
    }

    #[test]
    fn outbox_spec_builds_canonical_minimal_payload() {
        let spec = OutboxEventSpec::new(
            "booking.checked_out",
            OutboxAggregateKeySource::response_field("booking", "booking_id"),
            &["bookings", "rooms", "folio"],
        )
        .expect("spec builds");
        let ctx = test_ctx("check_out");
        let prepared = spec
            .prepare(
                &ctx,
                Some("booking:B1"),
                "hash-1",
                &json!({ "booking_id": "B1", "room_id": "R1", "ok": true }),
            )
            .expect("event prepares");

        assert_eq!(prepared.event_type, "booking.checked_out");
        assert_eq!(prepared.aggregate_key, "booking:B1");
        assert_eq!(prepared.origin_request_id, "req-outbox");
        assert_eq!(prepared.origin_idempotency_key, "idem-outbox");
        assert_eq!(prepared.origin_command_name, "check_out");
        assert_eq!(prepared.origin_request_hash, "hash-1");
        assert_eq!(
            prepared.payload_json,
            r#"{"aggregate":{"id":"B1","type":"booking"},"command_name":"check_out","refresh":["bookings","rooms","folio"],"schema_version":1}"#
        );
    }

    #[test]
    fn outbox_spec_rejects_sensitive_refresh_area() {
        let error = OutboxEventSpec::new(
            "folio.payment_recorded",
            OutboxAggregateKeySource::response_field("folio", "booking_id"),
            &["folio", "payment_token"],
        )
        .expect_err("sensitive refresh area rejected");

        assert!(error.message.contains("unsafe outbox text"));
    }

    #[test]
    fn outbox_spec_allows_payment_event_name() {
        OutboxEventSpec::new(
            "folio.payment_recorded",
            OutboxAggregateKeySource::response_field("folio", "booking_id"),
            &["folio", "bookings"],
        )
        .expect("payment event name is business-safe");
    }

    #[test]
    fn outbox_spec_allows_forbidden_substrings_in_machine_identifiers() {
        let spec = OutboxEventSpec::new(
            "booking.checked_out",
            OutboxAggregateKeySource::response_field("booking", "booking_id"),
            &["bookings", "rooms"],
        )
        .expect("spec builds");
        let ctx = test_ctx("check_out");
        let prepared = spec
            .prepare(
                &ctx,
                Some("booking:B-cccd-001"),
                "sha256-cccd001122334455",
                &json!({ "booking_id": "B-cccd-001" }),
            )
            .expect("machine identifiers with forbidden substrings prepare");

        assert_eq!(prepared.aggregate_key, "booking:B-cccd-001");
        assert_eq!(prepared.origin_request_hash, "sha256-cccd001122334455");
        validate_prepared_event(&prepared).expect("prepared event remains storable");
    }

    #[tokio::test]
    async fn insert_outbox_event_tx_persists_pending_event_contract() {
        let pool = test_pool().await;

        let spec = OutboxEventSpec::new(
            "booking.checked_out",
            OutboxAggregateKeySource::response_field("booking", "booking_id"),
            &["bookings", "rooms", "folio"],
        )
        .expect("spec builds");
        let ctx = test_ctx("check_out");
        let prepared = spec
            .prepare(
                &ctx,
                Some("booking:B1"),
                "hash-1",
                &json!({ "booking_id": "B1", "room_id": "R1", "ok": true }),
            )
            .expect("event prepares");

        let mut tx = pool.begin().await.expect("transaction begins");
        insert_outbox_event_tx(&mut tx, &prepared)
            .await
            .expect("outbox event inserts");
        tx.commit().await.expect("transaction commits");

        let row = sqlx::query(
            "SELECT
                event_type,
                aggregate_key,
                payload_json,
                origin_request_id,
                origin_idempotency_key,
                origin_command_name,
                origin_request_hash,
                status,
                attempts,
                worker_token,
                next_attempt_at,
                processing_started_at,
                processing_expires_at,
                last_error,
                dispatched_at
             FROM outbox_events",
        )
        .fetch_one(&pool)
        .await
        .expect("outbox row exists");

        assert_eq!(row.get::<String, _>("event_type"), "booking.checked_out");
        assert_eq!(row.get::<String, _>("aggregate_key"), "booking:B1");
        assert_eq!(
            row.get::<String, _>("payload_json"),
            r#"{"aggregate":{"id":"B1","type":"booking"},"command_name":"check_out","refresh":["bookings","rooms","folio"],"schema_version":1}"#
        );
        assert_eq!(row.get::<String, _>("origin_request_id"), "req-outbox");
        assert_eq!(
            row.get::<String, _>("origin_idempotency_key"),
            "idem-outbox"
        );
        assert_eq!(row.get::<String, _>("origin_command_name"), "check_out");
        assert_eq!(row.get::<String, _>("origin_request_hash"), "hash-1");
        assert_eq!(row.get::<String, _>("status"), "pending");
        assert_eq!(row.get::<i64, _>("attempts"), 0);
        assert!(row.get::<Option<String>, _>("worker_token").is_none());
        assert!(row.get::<Option<String>, _>("next_attempt_at").is_none());
        assert!(row
            .get::<Option<String>, _>("processing_started_at")
            .is_none());
        assert!(row
            .get::<Option<String>, _>("processing_expires_at")
            .is_none());
        assert!(row.get::<Option<String>, _>("last_error").is_none());
        assert!(row.get::<Option<String>, _>("dispatched_at").is_none());
    }

    #[tokio::test]
    async fn claim_pending_event_sets_processing_and_increments_attempts() {
        let pool = test_pool().await;
        let id = seed_outbox_event(
            &pool,
            "booking.checked_out",
            "booking:B1",
            "pending",
            0,
            None,
            None,
        )
        .await;

        let claimed = claim_next_outbox_event(&pool, OutboxDispatchConfig::test())
            .await
            .expect("claim succeeds")
            .expect("event is claimed");

        assert_eq!(claimed.id, id);
        assert_eq!(claimed.event_type, "booking.checked_out");
        assert_eq!(claimed.aggregate_key, "booking:B1");
        assert_eq!(claimed.payload_json, r#"{"schema_version":1}"#);
        assert_eq!(
            claimed.origin_request_id,
            "req-booking.checked_out-booking:B1"
        );
        assert_eq!(
            claimed.origin_idempotency_key,
            "idem-booking.checked_out-booking:B1"
        );
        assert_eq!(claimed.origin_command_name, "test.command");
        assert_eq!(claimed.origin_request_hash, "hash-test");
        assert_eq!(claimed.attempts, 1);
        assert!(!claimed.worker_token.is_empty());

        let row = sqlx::query(
            "SELECT status, attempts, worker_token, next_attempt_at,
                    processing_started_at, processing_expires_at, last_error
             FROM outbox_events WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .expect("claimed row exists");

        assert_eq!(row.get::<String, _>("status"), "processing");
        assert_eq!(row.get::<i64, _>("attempts"), 1);
        assert_eq!(
            row.get::<Option<String>, _>("worker_token"),
            Some(claimed.worker_token)
        );
        assert!(row.get::<Option<String>, _>("next_attempt_at").is_none());
        assert!(row
            .get::<Option<String>, _>("processing_started_at")
            .is_some());
        assert!(row
            .get::<Option<String>, _>("processing_expires_at")
            .is_some());
        assert!(row.get::<Option<String>, _>("last_error").is_none());
    }

    #[tokio::test]
    async fn claim_preserves_fifo_for_same_aggregate() {
        let pool = test_pool().await;
        let first_id = seed_outbox_event(
            &pool,
            "booking.first",
            "booking:B1",
            "pending",
            0,
            None,
            None,
        )
        .await;
        let second_id = seed_outbox_event(
            &pool,
            "booking.second",
            "booking:B1",
            "pending",
            0,
            None,
            None,
        )
        .await;

        let first_claim = claim_next_outbox_event(&pool, OutboxDispatchConfig::test())
            .await
            .expect("claim succeeds")
            .expect("first event is claimed");
        let second_claim = claim_next_outbox_event(&pool, OutboxDispatchConfig::test())
            .await
            .expect("second claim succeeds");

        assert_eq!(first_claim.id, first_id);
        assert_eq!(second_claim, None);

        let second_status: String =
            sqlx::query_scalar("SELECT status FROM outbox_events WHERE id = ?")
                .bind(second_id)
                .fetch_one(&pool)
                .await
                .expect("second row exists");
        assert_eq!(second_status, "pending");
    }

    #[tokio::test]
    async fn claim_reclaims_expired_processing_event() {
        let pool = test_pool().await;
        let id = seed_outbox_event(
            &pool,
            "booking.checked_out",
            "booking:B1",
            "processing",
            2,
            None,
            Some("2000-01-01T00:00:00+00:00"),
        )
        .await;

        let claimed = claim_next_outbox_event(&pool, OutboxDispatchConfig::test())
            .await
            .expect("claim succeeds")
            .expect("expired processing event is reclaimed");

        assert_eq!(claimed.id, id);
        assert_eq!(claimed.attempts, 3);
        assert!(!claimed.worker_token.is_empty());

        let row = sqlx::query(
            "SELECT status, attempts, worker_token, processing_expires_at
             FROM outbox_events WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .expect("claimed row exists");

        assert_eq!(row.get::<String, _>("status"), "processing");
        assert_eq!(row.get::<i64, _>("attempts"), 3);
        assert_eq!(
            row.get::<Option<String>, _>("worker_token"),
            Some(claimed.worker_token)
        );
        let expires_at = row
            .get::<Option<String>, _>("processing_expires_at")
            .expect("processing expiry set");
        assert!(expires_at.as_str() > "2000-01-01T00:00:00+00:00");
    }

    #[tokio::test]
    async fn claim_does_not_reclaim_active_processing_event() {
        let pool = test_pool().await;
        let id = seed_outbox_event(
            &pool,
            "booking.checked_out",
            "booking:B1",
            "processing",
            2,
            None,
            Some("2999-05-03T09:00:30+00:00"),
        )
        .await;

        let claimed = claim_next_outbox_event(&pool, OutboxDispatchConfig::test())
            .await
            .expect("claim succeeds");

        assert_eq!(claimed, None);

        let row = sqlx::query(
            "SELECT status, attempts, worker_token, processing_expires_at
             FROM outbox_events WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .expect("processing row exists");

        assert_eq!(row.get::<String, _>("status"), "processing");
        assert_eq!(row.get::<i64, _>("attempts"), 2);
        assert!(row.get::<Option<String>, _>("worker_token").is_none());
        assert_eq!(
            row.get::<Option<String>, _>("processing_expires_at"),
            Some("2999-05-03T09:00:30+00:00".to_string())
        );
    }

    #[tokio::test]
    async fn retry_limit_rows_fail_before_subscriber_claim() {
        let pool = test_pool().await;
        let pending_retry_limited_id = seed_outbox_event(
            &pool,
            "booking.pending_limited",
            "booking:B1",
            "pending",
            5,
            None,
            None,
        )
        .await;
        let expired_processing_retry_limited_id = seed_outbox_event(
            &pool,
            "booking.processing_limited",
            "booking:B2",
            "processing",
            5,
            None,
            Some("2000-01-01T00:00:00+00:00"),
        )
        .await;
        let claimable_id = seed_outbox_event(
            &pool,
            "booking.claimable",
            "booking:B3",
            "pending",
            4,
            None,
            None,
        )
        .await;

        let failed = fail_retry_limit_rows(&pool, OutboxDispatchConfig::test())
            .await
            .expect("retry-limit rows fail");
        let claimed = claim_next_outbox_event(&pool, OutboxDispatchConfig::test())
            .await
            .expect("claim succeeds")
            .expect("remaining event is claimable");

        assert_eq!(failed, 2);
        assert_eq!(claimed.id, claimable_id);

        for id in [
            pending_retry_limited_id,
            expired_processing_retry_limited_id,
        ] {
            let row = sqlx::query(
                "SELECT status, worker_token, next_attempt_at,
                        processing_started_at, processing_expires_at, last_error
                 FROM outbox_events WHERE id = ?",
            )
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("retry-limited row exists");

            assert_eq!(row.get::<String, _>("status"), "failed");
            assert!(row.get::<Option<String>, _>("worker_token").is_none());
            assert!(row.get::<Option<String>, _>("next_attempt_at").is_none());
            assert!(row
                .get::<Option<String>, _>("processing_started_at")
                .is_none());
            assert!(row
                .get::<Option<String>, _>("processing_expires_at")
                .is_none());
            assert_eq!(
                row.get::<Option<String>, _>("last_error"),
                Some("outbox retry limit reached".to_string())
            );
        }
    }

    #[tokio::test]
    async fn batch_with_empty_subscribers_leaves_pending_rows_unchanged() {
        let pool = test_pool().await;
        let id = seed_outbox_event(
            &pool,
            "booking.checked_out",
            "booking:B1",
            "pending",
            0,
            None,
            None,
        )
        .await;

        let summary = run_outbox_dispatch_batch(&pool, &[], OutboxDispatchConfig::test())
            .await
            .expect("dispatch batch skips without subscribers");

        assert_eq!(summary, OutboxDispatchSummary::default());

        let row = sqlx::query("SELECT status, attempts FROM outbox_events WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("pending row exists");

        assert_eq!(row.get::<String, _>("status"), "pending");
        assert_eq!(row.get::<i64, _>("attempts"), 0);
    }

    #[tokio::test]
    async fn batch_dispatches_same_aggregate_in_id_order() {
        let pool = test_pool().await;
        let first_id = seed_outbox_event(
            &pool,
            "booking.first",
            "booking:B1",
            "pending",
            0,
            None,
            None,
        )
        .await;
        let second_id = seed_outbox_event(
            &pool,
            "booking.second",
            "booking:B1",
            "pending",
            0,
            None,
            None,
        )
        .await;
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber = RecordingSubscriber {
            calls: calls.clone(),
            ..RecordingSubscriber::default()
        };
        let subscribers: [&dyn OutboxSubscriber; 1] = [&subscriber];

        let summary = run_outbox_dispatch_batch(&pool, &subscribers, OutboxDispatchConfig::test())
            .await
            .expect("dispatch batch succeeds");

        assert_eq!(summary.dispatched, 2);
        assert_eq!(
            *calls.lock().expect("calls lock"),
            vec![first_id, second_id]
        );
    }

    #[tokio::test]
    async fn batch_allows_different_aggregates_without_global_order_dependency() {
        let pool = test_pool().await;
        let blocked_id = seed_outbox_event(
            &pool,
            "booking.blocked",
            "booking:B1",
            "pending",
            0,
            Some("2999-05-03T09:00:30+00:00"),
            None,
        )
        .await;
        let due_id =
            seed_outbox_event(&pool, "booking.due", "booking:B2", "pending", 0, None, None).await;
        let subscriber = RecordingSubscriber::default();
        let subscribers: [&dyn OutboxSubscriber; 1] = [&subscriber];

        let summary = run_outbox_dispatch_batch(&pool, &subscribers, OutboxDispatchConfig::test())
            .await
            .expect("dispatch batch succeeds");

        assert_eq!(summary.dispatched, 1);
        assert_eq!(
            *subscriber.calls.lock().expect("calls lock"),
            vec![due_id],
            "due event on a different aggregate dispatches"
        );

        let blocked_status: String =
            sqlx::query_scalar("SELECT status FROM outbox_events WHERE id = ?")
                .bind(blocked_id)
                .fetch_one(&pool)
                .await
                .expect("blocked row exists");
        assert_eq!(blocked_status, "pending");
    }

    #[tokio::test]
    async fn startup_recovery_with_injected_subscriber_processes_expired_processing() {
        let pool = test_pool().await;
        let id = seed_outbox_event(
            &pool,
            "booking.checked_out",
            "booking:B1",
            "processing",
            2,
            None,
            Some("2000-01-01T00:00:00+00:00"),
        )
        .await;
        let subscriber: std::sync::Arc<dyn OutboxSubscriber> =
            std::sync::Arc::new(RecordingSubscriber::default());
        let subscribers = vec![subscriber];

        let summary =
            run_outbox_startup_recovery_once(&pool, &subscribers, OutboxDispatchConfig::test())
                .await
                .expect("startup recovery dispatches expired processing row");

        assert_eq!(summary.dispatched, 1);

        let status: String = sqlx::query_scalar("SELECT status FROM outbox_events WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("recovered row exists");
        assert_eq!(status, "dispatched");
    }

    #[tokio::test]
    async fn dispatcher_loop_runs_batch_before_first_poll_sleep() {
        let pool = test_pool().await;
        let id = seed_outbox_event(
            &pool,
            "booking.checked_out",
            "booking:B1",
            "pending",
            0,
            None,
            None,
        )
        .await;
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let subscriber: std::sync::Arc<dyn OutboxSubscriber> =
            std::sync::Arc::new(RecordingSubscriber {
                calls: calls.clone(),
                ..RecordingSubscriber::default()
            });
        let (shutdown_tx, shutdown_rx) = oneshot::channel();

        let task = tokio::spawn(run_outbox_dispatcher_loop(
            pool.clone(),
            vec![subscriber],
            shutdown_rx,
        ));

        tokio::time::timeout(StdDuration::from_millis(500), async {
            loop {
                if calls.lock().expect("calls lock").contains(&id) {
                    break;
                }
                tokio::time::sleep(StdDuration::from_millis(10)).await;
            }
        })
        .await
        .expect("dispatcher runs the first batch immediately");

        let _ = shutdown_tx.send(());
        task.await.expect("dispatcher task joins");

        let status: String = sqlx::query_scalar("SELECT status FROM outbox_events WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .expect("dispatched row exists");
        assert_eq!(status, "dispatched");
    }

    #[tokio::test]
    async fn dispatch_success_marks_event_dispatched() {
        let pool = test_pool().await;
        let id = seed_outbox_event(
            &pool,
            "booking.checked_out",
            "booking:B1",
            "pending",
            0,
            None,
            None,
        )
        .await;
        let subscriber = RecordingSubscriber::default();
        let subscribers: [&dyn OutboxSubscriber; 1] = [&subscriber];

        let summary = run_outbox_dispatch_batch(&pool, &subscribers, OutboxDispatchConfig::test())
            .await
            .expect("dispatch succeeds");

        assert_eq!(
            summary,
            OutboxDispatchSummary {
                claimed: 1,
                dispatched: 1,
                retried: 0,
                failed: 0,
                retry_limit_failed: 0,
            }
        );
        assert_eq!(
            *subscriber.calls.lock().expect("calls lock"),
            vec![id],
            "subscriber receives claimed event"
        );

        let row = sqlx::query(
            "SELECT status, attempts, worker_token, next_attempt_at,
                    processing_started_at, processing_expires_at,
                    last_error, dispatched_at
             FROM outbox_events WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .expect("dispatched row exists");

        assert_eq!(row.get::<String, _>("status"), "dispatched");
        assert_eq!(row.get::<i64, _>("attempts"), 1);
        assert!(row.get::<Option<String>, _>("worker_token").is_none());
        assert!(row.get::<Option<String>, _>("next_attempt_at").is_none());
        assert!(row
            .get::<Option<String>, _>("processing_started_at")
            .is_none());
        assert!(row
            .get::<Option<String>, _>("processing_expires_at")
            .is_none());
        assert!(row.get::<Option<String>, _>("last_error").is_none());
        assert!(row.get::<Option<String>, _>("dispatched_at").is_some());
    }

    #[tokio::test]
    async fn dispatch_failure_retries_pending_with_backoff() {
        let pool = test_pool().await;
        let id = seed_outbox_event(
            &pool,
            "booking.checked_out",
            "booking:B1",
            "pending",
            0,
            None,
            None,
        )
        .await;
        let subscriber = RecordingSubscriber {
            fail: true,
            ..RecordingSubscriber::default()
        };
        let subscribers: [&dyn OutboxSubscriber; 1] = [&subscriber];

        let summary = run_outbox_dispatch_batch(&pool, &subscribers, OutboxDispatchConfig::test())
            .await
            .expect("dispatch batch records failure");

        assert_eq!(
            summary,
            OutboxDispatchSummary {
                claimed: 1,
                dispatched: 0,
                retried: 1,
                failed: 0,
                retry_limit_failed: 0,
            }
        );

        let row = sqlx::query(
            "SELECT status, attempts, worker_token, next_attempt_at,
                    processing_started_at, processing_expires_at,
                    last_error, dispatched_at
             FROM outbox_events WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .expect("retried row exists");

        assert_eq!(row.get::<String, _>("status"), "pending");
        assert_eq!(row.get::<i64, _>("attempts"), 1);
        assert!(row.get::<Option<String>, _>("worker_token").is_none());
        assert!(row.get::<Option<String>, _>("next_attempt_at").is_some());
        assert!(row
            .get::<Option<String>, _>("processing_started_at")
            .is_none());
        assert!(row
            .get::<Option<String>, _>("processing_expires_at")
            .is_none());
        assert_eq!(
            row.get::<Option<String>, _>("last_error"),
            Some("SYSTEM_INTERNAL_ERROR: subscriber delivery failed".to_string())
        );
        assert!(row.get::<Option<String>, _>("dispatched_at").is_none());
    }

    #[tokio::test]
    async fn dispatch_failure_at_limit_marks_failed() {
        let pool = test_pool().await;
        let id = seed_outbox_event(
            &pool,
            "booking.checked_out",
            "booking:B1",
            "pending",
            4,
            None,
            None,
        )
        .await;
        let subscriber = RecordingSubscriber {
            fail: true,
            ..RecordingSubscriber::default()
        };
        let subscribers: [&dyn OutboxSubscriber; 1] = [&subscriber];

        let summary = run_outbox_dispatch_batch(&pool, &subscribers, OutboxDispatchConfig::test())
            .await
            .expect("dispatch batch records terminal failure");

        assert_eq!(
            summary,
            OutboxDispatchSummary {
                claimed: 1,
                dispatched: 0,
                retried: 0,
                failed: 1,
                retry_limit_failed: 0,
            }
        );

        let row = sqlx::query(
            "SELECT status, attempts, worker_token, next_attempt_at,
                    processing_started_at, processing_expires_at,
                    last_error, dispatched_at
             FROM outbox_events WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .expect("failed row exists");

        assert_eq!(row.get::<String, _>("status"), "failed");
        assert_eq!(row.get::<i64, _>("attempts"), 5);
        assert!(row.get::<Option<String>, _>("worker_token").is_none());
        assert!(row.get::<Option<String>, _>("next_attempt_at").is_none());
        assert!(row
            .get::<Option<String>, _>("processing_started_at")
            .is_none());
        assert!(row
            .get::<Option<String>, _>("processing_expires_at")
            .is_none());
        assert_eq!(
            row.get::<Option<String>, _>("last_error"),
            Some("SYSTEM_INTERNAL_ERROR: subscriber delivery failed".to_string())
        );
        assert!(row.get::<Option<String>, _>("dispatched_at").is_none());
    }

    #[tokio::test]
    async fn dispatch_failure_does_not_store_unsafe_subscriber_error() {
        let pool = test_pool().await;
        let id = seed_outbox_event(
            &pool,
            "booking.checked_out",
            "booking:B1",
            "pending",
            0,
            None,
            None,
        )
        .await;
        let unsafe_message = format!(
            "delivery failed for guest@example.com with token=secret-token passport CCCD {}",
            "x".repeat(700)
        );
        let subscriber = RecordingSubscriber {
            fail: true,
            error_message: Some(unsafe_message),
            ..RecordingSubscriber::default()
        };
        let subscribers: [&dyn OutboxSubscriber; 1] = [&subscriber];

        run_outbox_dispatch_batch(&pool, &subscribers, OutboxDispatchConfig::test())
            .await
            .expect("dispatch batch records sanitized failure");

        let last_error: String =
            sqlx::query_scalar("SELECT last_error FROM outbox_events WHERE id = ?")
                .bind(id)
                .fetch_one(&pool)
                .await
                .expect("last_error is stored");

        assert_eq!(
            last_error,
            "SYSTEM_INTERNAL_ERROR: subscriber delivery failed"
        );
        assert!(last_error.chars().count() <= OUTBOX_ERROR_MAX_CHARS);
        assert!(!last_error.contains("guest@example.com"));
        assert!(!last_error.to_ascii_lowercase().contains("token"));
        assert!(!last_error.to_ascii_lowercase().contains("passport"));
        assert!(!last_error.to_ascii_lowercase().contains("cccd"));
    }

    #[tokio::test]
    async fn stale_worker_token_cannot_mark_dispatched() {
        let pool = test_pool().await;
        let id = seed_outbox_event(
            &pool,
            "booking.checked_out",
            "booking:B1",
            "pending",
            0,
            None,
            None,
        )
        .await;
        let claimed = claim_next_outbox_event(&pool, OutboxDispatchConfig::test())
            .await
            .expect("claim succeeds")
            .expect("event is claimed");

        let updated = mark_outbox_dispatched(&pool, id, "stale-worker-token")
            .await
            .expect("stale finalization is ignored");

        assert!(!updated);

        let row = sqlx::query(
            "SELECT status, worker_token, dispatched_at FROM outbox_events WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .expect("claimed row exists");

        assert_eq!(row.get::<String, _>("status"), "processing");
        assert_eq!(
            row.get::<Option<String>, _>("worker_token"),
            Some(claimed.worker_token)
        );
        assert!(row.get::<Option<String>, _>("dispatched_at").is_none());
    }

    async fn explain_query_plan(
        pool: &sqlx::Pool<Sqlite>,
        sql: &str,
        binds: &[&str],
    ) -> Vec<String> {
        let explain_sql = format!("EXPLAIN QUERY PLAN {sql}");
        let mut query = sqlx::query(&explain_sql);
        for bind in binds {
            query = query.bind(*bind);
        }
        query
            .fetch_all(pool)
            .await
            .expect("query plan explains")
            .into_iter()
            .map(|row| row.get::<String, _>("detail"))
            .collect()
    }

    fn assert_uses_index(plan: &[String], index_name: &str) {
        assert!(
            plan.iter().any(|detail| detail.contains(index_name)),
            "expected plan to use {index_name}; plan: {plan:?}"
        );
    }

    fn assert_no_plain_scan(plan: &[String], table_or_alias: &str) {
        let plain_scan = format!("SCAN {table_or_alias}");
        assert!(
            plan.iter()
                .all(|detail| { !detail.contains(&plain_scan) || detail.contains("USING INDEX") }),
            "expected plan not to scan {table_or_alias} without an index; plan: {plan:?}"
        );
    }

    #[tokio::test]
    async fn claim_candidate_query_uses_outbox_partial_indexes() {
        let pool = test_pool().await;
        let plan = explain_query_plan(
            &pool,
            CLAIM_NEXT_OUTBOX_EVENT_CANDIDATE_SQL,
            &[
                "5",
                "5",
                "2026-05-03T09:00:00+00:00",
                "5",
                "2026-05-03T09:00:00+00:00",
            ],
        )
        .await;

        assert_uses_index(&plan, "outbox_events_pending_idx");
        assert_uses_index(&plan, "outbox_events_processing_idx");
        assert_uses_index(&plan, "outbox_events_aggregate_open_idx");
        assert_no_plain_scan(&plan, "candidate");
    }

    #[tokio::test]
    async fn retry_limit_failover_queries_use_outbox_partial_indexes() {
        let pool = test_pool().await;

        let pending_null_plan = explain_query_plan(
            &pool,
            FAIL_RETRY_LIMIT_PENDING_NULL_SQL,
            &[OUTBOX_STATUS_FAILED, OUTBOX_RETRY_LIMIT_ERROR, "5"],
        )
        .await;
        let pending_due_plan = explain_query_plan(
            &pool,
            FAIL_RETRY_LIMIT_PENDING_DUE_SQL,
            &[
                OUTBOX_STATUS_FAILED,
                OUTBOX_RETRY_LIMIT_ERROR,
                "5",
                "2026-05-03T09:00:00+00:00",
            ],
        )
        .await;
        let processing_plan = explain_query_plan(
            &pool,
            FAIL_RETRY_LIMIT_PROCESSING_SQL,
            &[
                OUTBOX_STATUS_FAILED,
                OUTBOX_RETRY_LIMIT_ERROR,
                "5",
                "2026-05-03T09:00:00+00:00",
            ],
        )
        .await;

        assert_uses_index(&pending_null_plan, "outbox_events_pending_idx");
        assert_uses_index(&pending_due_plan, "outbox_events_pending_idx");
        assert_uses_index(&processing_plan, "outbox_events_processing_idx");
        assert_no_plain_scan(&pending_null_plan, "outbox_events");
        assert_no_plain_scan(&pending_due_plan, "outbox_events");
        assert_no_plain_scan(&processing_plan, "outbox_events");
    }
}
