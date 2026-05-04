use async_stream::stream;
use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Row, Sqlite};
use std::{convert::Infallible, time::Duration};

const OBSERVER_BATCH_LIMIT: i64 = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObserverErrorCode {
    InvalidCursor,
    CursorExpired,
    ObserverUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObserverError {
    pub code: ObserverErrorCode,
    pub message: String,
}

#[derive(Debug, Serialize)]
struct ObserverErrorResponse<'a> {
    ok: bool,
    error: ObserverErrorBody<'a>,
}

#[derive(Debug, Serialize)]
struct ObserverErrorBody<'a> {
    code: &'a str,
    message: &'a str,
}

#[derive(Debug, Deserialize)]
pub struct ObserverQuery {
    pub last_event_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ObserverEvent {
    pub event_id: i64,
    pub event_type: String,
    pub aggregate: ObserverAggregate,
    pub created_at: String,
    pub refresh: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObserverEventBatch {
    pub events: Vec<ObserverEvent>,
    pub high_watermark_event_id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ObserverAggregate {
    #[serde(rename = "type")]
    pub ref_type: String,
    pub id: String,
}

#[derive(Debug, Deserialize)]
struct StoredOutboxPayload {
    schema_version: i64,
    aggregate: StoredOutboxAggregate,
    refresh: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct StoredOutboxAggregate {
    #[serde(rename = "type")]
    ref_type: String,
    id: String,
}

impl ObserverErrorCode {
    fn as_str(&self) -> &'static str {
        match self {
            ObserverErrorCode::InvalidCursor => "invalid_cursor",
            ObserverErrorCode::CursorExpired => "cursor_expired",
            ObserverErrorCode::ObserverUnavailable => "observer_unavailable",
        }
    }

    fn status_code(&self) -> StatusCode {
        match self {
            ObserverErrorCode::InvalidCursor => StatusCode::BAD_REQUEST,
            ObserverErrorCode::CursorExpired => StatusCode::CONFLICT,
            ObserverErrorCode::ObserverUnavailable => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

impl IntoResponse for ObserverError {
    fn into_response(self) -> axum::response::Response {
        let status = self.code.status_code();
        let body = ObserverErrorResponse {
            ok: false,
            error: ObserverErrorBody {
                code: self.code.as_str(),
                message: &self.message,
            },
        };

        (status, Json(body)).into_response()
    }
}

pub fn parse_cursor(
    query_last_event_id: Option<&str>,
    headers: &HeaderMap,
) -> Result<Option<i64>, ObserverError> {
    let cursor = match query_last_event_id {
        Some(cursor) => Some(cursor),
        None => headers
            .get("Last-Event-ID")
            .map(|value| value.to_str())
            .transpose()
            .map_err(|_| invalid_cursor("Last-Event-ID header must be a valid integer cursor"))?,
    };

    cursor.map(parse_cursor_value).transpose()
}

fn parse_cursor_value(cursor: &str) -> Result<i64, ObserverError> {
    let parsed = cursor
        .parse::<i64>()
        .map_err(|_| invalid_cursor("Cursor must be a non-negative integer"))?;

    if parsed < 0 {
        return Err(invalid_cursor("Cursor must be a non-negative integer"));
    }

    Ok(parsed)
}

fn invalid_cursor(message: &str) -> ObserverError {
    ObserverError {
        code: ObserverErrorCode::InvalidCursor,
        message: message.to_string(),
    }
}

fn observer_unavailable() -> ObserverError {
    ObserverError {
        code: ObserverErrorCode::ObserverUnavailable,
        message: "observer stream is unavailable".to_string(),
    }
}

fn cursor_expired() -> ObserverError {
    ObserverError {
        code: ObserverErrorCode::CursorExpired,
        message: "last_event_id is older than the earliest observer event still available"
            .to_string(),
    }
}

fn parse_safe_payload(
    payload_json: &str,
) -> Result<(ObserverAggregate, Vec<String>), ObserverError> {
    let payload: StoredOutboxPayload =
        serde_json::from_str(payload_json).map_err(|_| observer_unavailable())?;

    if payload.schema_version != 1 {
        return Err(observer_unavailable());
    }

    let ref_type = payload.aggregate.ref_type.trim();
    let id = payload.aggregate.id.trim();
    if ref_type.is_empty() || id.is_empty() || payload.refresh.is_empty() {
        return Err(observer_unavailable());
    }

    let mut refresh = Vec::with_capacity(payload.refresh.len());
    for item in payload.refresh {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            return Err(observer_unavailable());
        }
        refresh.push(trimmed.to_string());
    }

    Ok((
        ObserverAggregate {
            ref_type: ref_type.to_string(),
            id: id.to_string(),
        },
        refresh,
    ))
}

pub async fn resolve_start_after_event_id(
    pool: &Pool<Sqlite>,
    cursor: Option<i64>,
) -> Result<i64, ObserverError> {
    let row = sqlx::query("SELECT MIN(id) AS min_id, MAX(id) AS max_id FROM outbox_events")
        .fetch_one(pool)
        .await
        .map_err(|_| observer_unavailable())?;

    let min_id: Option<i64> = row.try_get("min_id").map_err(|_| observer_unavailable())?;
    let max_id: Option<i64> = row.try_get("max_id").map_err(|_| observer_unavailable())?;

    match (cursor, min_id, max_id) {
        (None, _, Some(max_id)) => Ok(max_id),
        (None, _, None) => Ok(0),
        (Some(cursor), Some(min_id), _) if cursor < min_id => Err(cursor_expired()),
        (Some(cursor), _, Some(max_id)) if cursor > max_id => Err(invalid_cursor(
            "Cursor is newer than the latest observer event",
        )),
        (Some(cursor), None, None) if cursor > 0 => Err(invalid_cursor(
            "Cursor is newer than the latest observer event",
        )),
        (Some(cursor), _, _) => Ok(cursor),
    }
}

pub async fn load_observer_events_after(
    pool: &Pool<Sqlite>,
    after_event_id: i64,
    limit: i64,
) -> Result<ObserverEventBatch, ObserverError> {
    let limit = limit.clamp(1, OBSERVER_BATCH_LIMIT);
    let rows = sqlx::query(
        "SELECT id, event_type, payload_json, created_at
         FROM outbox_events
         WHERE id > ?
         ORDER BY id
         LIMIT ?",
    )
    .bind(after_event_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|_| observer_unavailable())?;

    let mut events = Vec::new();
    let mut high_watermark_event_id = after_event_id;
    for row in rows {
        let event_id = row.try_get("id").map_err(|_| observer_unavailable())?;
        high_watermark_event_id = event_id;

        let payload_json: String = row
            .try_get("payload_json")
            .map_err(|_| observer_unavailable())?;
        let Ok((aggregate, refresh)) = parse_safe_payload(&payload_json) else {
            continue;
        };

        events.push(ObserverEvent {
            event_id,
            event_type: row
                .try_get("event_type")
                .map_err(|_| observer_unavailable())?,
            aggregate,
            created_at: row
                .try_get("created_at")
                .map_err(|_| observer_unavailable())?,
            refresh,
        });
    }

    Ok(ObserverEventBatch {
        events,
        high_watermark_event_id,
    })
}

pub async fn observe_events(
    State(pool): State<Pool<Sqlite>>,
    Query(query): Query<ObserverQuery>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ObserverError> {
    let cursor = parse_cursor(query.last_event_id.as_deref(), &headers)?;
    let start_after = resolve_start_after_event_id(&pool, cursor).await?;

    Ok(Sse::new(build_observer_sse_stream(
        pool,
        start_after,
        Duration::from_secs(1),
    ))
    .keep_alive(KeepAlive::default()))
}

pub fn build_observer_sse_stream(
    pool: Pool<Sqlite>,
    start_after: i64,
    poll_interval: Duration,
) -> impl Stream<Item = Result<Event, Infallible>> {
    stream! {
        let mut last_sent_id = start_after;

        loop {
            match load_observer_events_after(&pool, last_sent_id, OBSERVER_BATCH_LIMIT).await {
                Ok(batch) => {
                    for observer_event in batch.events {
                        let event_id = observer_event.event_id.to_string();
                        let event = Event::default()
                            .event("pms.changed")
                            .id(event_id)
                            .json_data(observer_event)
                            .unwrap_or_else(|_| observer_error_event(observer_unavailable()));

                        yield Ok(event);
                    }
                    last_sent_id = batch.high_watermark_event_id;
                }
                Err(error) => {
                    yield Ok(observer_error_event(error));
                }
            }

            tokio::time::sleep(poll_interval).await;
        }
    }
}

fn observer_error_event(error: ObserverError) -> Event {
    let payload = ObserverErrorResponse {
        ok: false,
        error: ObserverErrorBody {
            code: error.code.as_str(),
            message: &error.message,
        },
    };

    Event::default()
        .event("observer.error")
        .json_data(payload)
        .unwrap_or_else(|_| {
            Event::default().event("observer.error").data(
                r#"{"ok":false,"error":{"code":"observer_unavailable","message":"observer stream is unavailable"}}"#,
            )
        })
}

#[cfg(test)]
mod tests {
    use super::{ObserverErrorCode, parse_cursor};
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn parse_cursor_missing_cursor_returns_none() {
        let headers = HeaderMap::new();

        let cursor = parse_cursor(None, &headers).expect("missing cursor is valid");

        assert_eq!(cursor, None);
    }

    #[test]
    fn parse_cursor_reads_query_last_event_id() {
        let headers = HeaderMap::new();

        let cursor = parse_cursor(Some("42"), &headers).expect("query cursor is valid");

        assert_eq!(cursor, Some(42));
    }

    #[test]
    fn parse_cursor_reads_last_event_id_header() {
        let mut headers = HeaderMap::new();
        headers.insert("Last-Event-ID", HeaderValue::from_static("84"));

        let cursor = parse_cursor(None, &headers).expect("header cursor is valid");

        assert_eq!(cursor, Some(84));
    }

    #[test]
    fn parse_cursor_query_wins_over_last_event_id_header() {
        let mut headers = HeaderMap::new();
        headers.insert("Last-Event-ID", HeaderValue::from_static("84"));

        let cursor = parse_cursor(Some("42"), &headers).expect("query cursor is valid");

        assert_eq!(cursor, Some(42));
    }

    #[test]
    fn parse_cursor_rejects_negative_query_cursor() {
        let headers = HeaderMap::new();

        let error = parse_cursor(Some("-1"), &headers).expect_err("negative cursor is invalid");

        assert_eq!(error.code, ObserverErrorCode::InvalidCursor);
    }

    #[test]
    fn parse_cursor_rejects_non_numeric_query_cursor() {
        let headers = HeaderMap::new();

        let error = parse_cursor(Some("abc"), &headers).expect_err("non-numeric cursor is invalid");

        assert_eq!(error.code, ObserverErrorCode::InvalidCursor);
    }

    #[test]
    fn parse_cursor_rejects_negative_header_cursor() {
        let mut headers = HeaderMap::new();
        headers.insert("Last-Event-ID", HeaderValue::from_static("-1"));

        let error = parse_cursor(None, &headers).expect_err("negative cursor is invalid");

        assert_eq!(error.code, ObserverErrorCode::InvalidCursor);
    }

    #[test]
    fn parse_cursor_rejects_non_numeric_header_cursor() {
        let mut headers = HeaderMap::new();
        headers.insert("Last-Event-ID", HeaderValue::from_static("abc"));

        let error = parse_cursor(None, &headers).expect_err("non-numeric cursor is invalid");

        assert_eq!(error.code, ObserverErrorCode::InvalidCursor);
    }
}

#[cfg(test)]
mod db_tests {
    use super::{ObserverErrorCode, load_observer_events_after, resolve_start_after_event_id};
    use serde_json::json;
    use sqlx::{Pool, Sqlite, sqlite::SqlitePoolOptions};

    async fn test_pool() -> Pool<Sqlite> {
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

    async fn seed_outbox_event(pool: &Pool<Sqlite>, aggregate_id: &str, created_at: &str) -> i64 {
        let payload = json!({
            "schema_version": 1,
            "command_name": "check_out",
            "aggregate": { "type": "booking", "id": aggregate_id },
            "refresh": ["bookings", "rooms", "folio"],
            "guest_name": "Private Guest",
            "guest_phone": "0900000000",
            "doc_number": "P1234567",
            "email": "private@example.com"
        });

        let result = sqlx::query(
            "INSERT INTO outbox_events (
                event_type, aggregate_key, payload_json,
                origin_request_id, origin_idempotency_key,
                origin_command_name, origin_request_hash,
                status, attempts, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("pms.command.completed")
        .bind(format!("booking:{aggregate_id}"))
        .bind(payload.to_string())
        .bind(format!("req-{aggregate_id}"))
        .bind(format!("idem-{aggregate_id}"))
        .bind("check_out")
        .bind(format!("hash-{aggregate_id}"))
        .bind("pending")
        .bind(3_i64)
        .bind(created_at)
        .execute(pool)
        .await
        .expect("seeds outbox event");

        result.last_insert_rowid()
    }

    async fn seed_malformed_outbox_event(pool: &Pool<Sqlite>, created_at: &str) -> i64 {
        let result = sqlx::query(
            "INSERT INTO outbox_events (
                event_type, aggregate_key, payload_json,
                origin_request_id, origin_idempotency_key,
                origin_command_name, origin_request_hash,
                status, attempts, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("pms.command.completed")
        .bind("booking:malformed")
        .bind(r#"{"schema_version":2,"aggregate":{"type":"booking","id":"malformed"},"refresh":["bookings"]}"#)
        .bind("req-malformed")
        .bind("idem-malformed")
        .bind("check_out")
        .bind("hash-malformed")
        .bind("pending")
        .bind(3_i64)
        .bind(created_at)
        .execute(pool)
        .await
        .expect("seeds malformed outbox event");

        result.last_insert_rowid()
    }

    #[tokio::test]
    async fn no_cursor_starts_after_current_max_id() {
        let pool = test_pool().await;
        let first_id = seed_outbox_event(&pool, "booking-1", "2026-05-03T09:00:00+00:00").await;
        let second_id = seed_outbox_event(&pool, "booking-2", "2026-05-03T09:01:00+00:00").await;

        let start_after = resolve_start_after_event_id(&pool, None)
            .await
            .expect("resolves no cursor");
        let batch = load_observer_events_after(&pool, start_after, 100)
            .await
            .expect("loads observer events");

        assert!(second_id > first_id);
        assert_eq!(start_after, second_id);
        assert!(batch.events.is_empty());
        assert_eq!(batch.high_watermark_event_id, second_id);
    }

    #[tokio::test]
    async fn cursor_replays_only_newer_events_with_safe_payload() {
        let pool = test_pool().await;
        let first_id = seed_outbox_event(&pool, "booking-1", "2026-05-03T09:00:00+00:00").await;
        let second_id = seed_outbox_event(&pool, "booking-2", "2026-05-03T09:01:00+00:00").await;

        let start_after = resolve_start_after_event_id(&pool, Some(first_id))
            .await
            .expect("resolves cursor");
        let batch = load_observer_events_after(&pool, start_after, 100)
            .await
            .expect("loads observer events");

        assert_eq!(batch.events.len(), 1);
        assert_eq!(batch.high_watermark_event_id, second_id);
        let event = &batch.events[0];
        assert_eq!(event.event_id, second_id);
        assert_eq!(event.event_type, "pms.command.completed");
        assert_eq!(event.aggregate.ref_type, "booking");
        assert_eq!(event.aggregate.id, "booking-2");
        assert_eq!(event.created_at, "2026-05-03T09:01:00+00:00");
        assert_eq!(event.refresh, vec!["bookings", "rooms", "folio"]);

        let serialized = serde_json::to_string(event).expect("serializes observer event");
        for forbidden in [
            "origin_request_id",
            "origin_idempotency_key",
            "origin_request_hash",
            "origin_command_name",
            "status",
            "attempts",
            "worker_token",
            "last_error",
            "payload_json",
            "guest_name",
            "guest_phone",
            "doc_number",
            "email",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "serialized observer event exposed {forbidden}: {serialized}"
            );
        }
    }

    #[tokio::test]
    async fn malformed_payload_rows_do_not_block_later_events() {
        let pool = test_pool().await;
        let first_id = seed_outbox_event(&pool, "booking-1", "2026-05-03T09:00:00+00:00").await;
        let malformed_id = seed_malformed_outbox_event(&pool, "2026-05-03T09:01:00+00:00").await;
        let second_id = seed_outbox_event(&pool, "booking-2", "2026-05-03T09:02:00+00:00").await;

        let batch = load_observer_events_after(&pool, first_id, 100)
            .await
            .expect("loads observer events");

        assert!(first_id < malformed_id);
        assert!(malformed_id < second_id);
        assert_eq!(batch.events.len(), 1);
        assert_eq!(batch.events[0].event_id, second_id);
        assert_eq!(batch.high_watermark_event_id, second_id);
    }

    #[tokio::test]
    async fn malformed_only_batches_advance_high_watermark() {
        let pool = test_pool().await;
        let first_id = seed_outbox_event(&pool, "booking-1", "2026-05-03T09:00:00+00:00").await;
        let malformed_id = seed_malformed_outbox_event(&pool, "2026-05-03T09:01:00+00:00").await;

        let batch = load_observer_events_after(&pool, first_id, 100)
            .await
            .expect("loads observer events");

        assert_eq!(batch.events, Vec::new());
        assert_eq!(batch.high_watermark_event_id, malformed_id);
    }

    #[tokio::test]
    async fn cursor_below_existing_min_id_is_expired() {
        let pool = test_pool().await;
        seed_outbox_event(&pool, "booking-1", "2026-05-03T09:00:00+00:00").await;

        let error = resolve_start_after_event_id(&pool, Some(0))
            .await
            .expect_err("cursor below min is expired");

        assert_eq!(error.code, ObserverErrorCode::CursorExpired);
        assert_eq!(
            error.message,
            "last_event_id is older than the earliest observer event still available"
        );
    }

    #[tokio::test]
    async fn cursor_above_current_max_id_is_invalid() {
        let pool = test_pool().await;
        let current_id = seed_outbox_event(&pool, "booking-1", "2026-05-03T09:00:00+00:00").await;

        let error = resolve_start_after_event_id(&pool, Some(current_id + 1))
            .await
            .expect_err("cursor above max is invalid");

        assert_eq!(error.code, ObserverErrorCode::InvalidCursor);
    }
}
