use crate::{
    app_error::{codes, CommandError, CommandResult},
    command_idempotency::{system_error, WriteCommandContext},
};
use chrono::{Duration, Utc};
use serde_json::{json, Value};
use sqlx::{sqlite::SqliteRow, Pool, Row, Sqlite, Transaction};

const OUTBOX_STATUS_PENDING: &str = "pending";
const OUTBOX_STATUS_PROCESSING: &str = "processing";
const OUTBOX_STATUS_FAILED: &str = "failed";
const OUTBOX_SAFE_TEXT_MAX_CHARS: usize = 160;
const OUTBOX_RETRY_LIMIT_ERROR: &str = "outbox retry limit reached";

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

    let result = sqlx::query(
        "UPDATE outbox_events
         SET status = ?,
             worker_token = NULL,
             next_attempt_at = NULL,
             processing_started_at = NULL,
             processing_expires_at = NULL,
             last_error = ?
         WHERE attempts >= ?
           AND (
                (
                    status = ?
                    AND (next_attempt_at IS NULL OR next_attempt_at <= ?)
                )
                OR (
                    status = ?
                    AND processing_expires_at <= ?
                )
           )",
    )
    .bind(OUTBOX_STATUS_FAILED)
    .bind(OUTBOX_RETRY_LIMIT_ERROR)
    .bind(config.max_attempts)
    .bind(OUTBOX_STATUS_PENDING)
    .bind(&now)
    .bind(OUTBOX_STATUS_PROCESSING)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(system_error)?;

    tx.commit().await.map_err(system_error)?;

    Ok(result.rows_affected())
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
        let claimable_id = sqlx::query_scalar::<_, i64>(
            "SELECT candidate.id
             FROM outbox_events candidate
             WHERE candidate.attempts < ?
               AND (
                    (
                        candidate.status = ?
                        AND (
                            candidate.next_attempt_at IS NULL
                            OR candidate.next_attempt_at <= ?
                        )
                    )
                    OR (
                        candidate.status = ?
                        AND candidate.processing_expires_at <= ?
                    )
               )
               AND NOT EXISTS (
                    SELECT 1
                    FROM outbox_events older
                    WHERE older.aggregate_key = candidate.aggregate_key
                      AND older.id < candidate.id
                      AND older.status IN ('pending', 'processing')
               )
             ORDER BY candidate.id
             LIMIT 1",
        )
        .bind(config.max_attempts)
        .bind(OUTBOX_STATUS_PENDING)
        .bind(&now_string)
        .bind(OUTBOX_STATUS_PROCESSING)
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
        validate_safe_outbox_text(origin_request_hash)?;

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
            validate_safe_outbox_text(aggregate_id)?;
            validate_safe_outbox_text(aggregate_key)?;
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
            validate_safe_outbox_text(aggregate_id)?;
            Ok((
                aggregate_type.to_string(),
                aggregate_id.to_string(),
                format!("{aggregate_type}:{aggregate_id}"),
            ))
        }
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
    validate_safe_outbox_text(&event.aggregate_key)?;
    validate_non_empty(event.origin_request_id.clone(), "origin_request_id")?;
    validate_non_empty(
        event.origin_idempotency_key.clone(),
        "origin_idempotency_key",
    )?;
    validate_safe_outbox_text(&event.origin_command_name)?;
    validate_safe_outbox_text(&event.origin_request_hash)?;
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

fn contains_forbidden_payload_term(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "phone", "email", "card", "token", "secret", "password", "passport", "cccd", "prompt",
        "raw",
    ]
    .iter()
    .any(|term| lower.contains(term))
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
        assert!(expires_at > "2000-01-01T00:00:00+00:00".to_string());
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
}
