use crate::{
    app_error::{codes, CommandError, CommandResult},
    command_idempotency::{system_error, WriteCommandContext},
};
use chrono::Utc;
use serde_json::{json, Value};
use sqlx::{Sqlite, Transaction};

const OUTBOX_STATUS_PENDING: &str = "pending";
const OUTBOX_SAFE_TEXT_MAX_CHARS: usize = 160;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command_idempotency::WriteCommandContext;
    use chrono::DateTime;
    use serde_json::json;
    use sqlx::{sqlite::SqlitePoolOptions, Row};

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
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite connects");
        crate::db::run_migrations(&pool)
            .await
            .expect("migrations run");

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
}
