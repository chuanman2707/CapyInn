use sqlx::{Pool, Row, Sqlite};

use crate::command_idempotency::WriteCommandContext;

async fn outbox_event_count(pool: &Pool<Sqlite>, command_name: &str, idempotency_key: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM outbox_events
         WHERE origin_command_name = ? AND origin_idempotency_key = ?",
    )
    .bind(command_name)
    .bind(idempotency_key)
    .fetch_one(pool)
    .await
    .expect("counts outbox events")
}

pub async fn assert_single_outbox_event(
    pool: &Pool<Sqlite>,
    ctx: &WriteCommandContext,
    event_type: &str,
) -> serde_json::Value {
    assert_eq!(
        outbox_event_count(pool, &ctx.command_name, &ctx.idempotency_key).await,
        1
    );

    let row = sqlx::query(
        "SELECT event_type, status, attempts, origin_request_id,
                origin_request_hash, payload_json
         FROM outbox_events
         WHERE origin_command_name = ? AND origin_idempotency_key = ?",
    )
    .bind(&ctx.command_name)
    .bind(&ctx.idempotency_key)
    .fetch_one(pool)
    .await
    .expect("reads outbox event");

    assert_eq!(row.get::<String, _>("event_type"), event_type);
    assert_eq!(row.get::<String, _>("status"), "pending");
    assert_eq!(row.get::<i64, _>("attempts"), 0);
    assert_eq!(
        row.get::<String, _>("origin_request_id"),
        ctx.request_id.as_str()
    );
    assert!(!row.get::<String, _>("origin_request_hash").is_empty());

    let payload: serde_json::Value =
        serde_json::from_str(&row.get::<String, _>("payload_json")).expect("payload is JSON");
    assert_eq!(payload["schema_version"], serde_json::json!(1));
    assert_eq!(
        payload["command_name"],
        serde_json::json!(ctx.command_name.as_str())
    );
    assert!(payload
        .get("refresh")
        .and_then(|value| value.as_array())
        .is_some());
    payload
}
