use sqlx::{Pool, Row, Sqlite};

use crate::{
    command_idempotency::WriteCommandContext, models::BookingExportRow,
    queries::booking::audit_queries,
};

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

pub fn assert_replayed_pair<T>(
    first: &crate::command_idempotency::IdempotentCommandResult<T>,
    second: &crate::command_idempotency::IdempotentCommandResult<T>,
) where
    T: PartialEq + std::fmt::Debug,
{
    assert!(!first.replayed);
    assert!(second.replayed);
    assert_eq!(first.response, second.response);
}

pub async fn assert_room_status(pool: &Pool<Sqlite>, room_id: &str, expected_status: &str) {
    let status: String = sqlx::query_scalar("SELECT status FROM rooms WHERE id = ?")
        .bind(room_id)
        .fetch_one(pool)
        .await
        .expect("read room status");
    assert_eq!(status, expected_status);
}

pub async fn assert_booking_status(pool: &Pool<Sqlite>, booking_id: &str, expected_status: &str) {
    let status: String = sqlx::query_scalar("SELECT status FROM bookings WHERE id = ?")
        .bind(booking_id)
        .fetch_one(pool)
        .await
        .expect("read booking status");
    assert_eq!(status, expected_status);
}

pub async fn assert_calendar_rows(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    status: &str,
    expected_count: i64,
) {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM room_calendar WHERE booking_id = ? AND status = ?",
    )
    .bind(booking_id)
    .bind(status)
    .fetch_one(pool)
    .await
    .expect("count calendar rows");
    assert_eq!(count, expected_count);
}

pub async fn assert_housekeeping_rows(
    pool: &Pool<Sqlite>,
    room_id: &str,
    status: &str,
    expected_count: i64,
) {
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM housekeeping WHERE room_id = ? AND status = ?")
            .bind(room_id)
            .bind(status)
            .fetch_one(pool)
            .await
            .expect("count housekeeping rows");
    assert_eq!(count, expected_count);
}

pub async fn assert_transaction_origin(
    pool: &Pool<Sqlite>,
    origin_key: &str,
    expected_ordinal: i64,
    expected_count: i64,
) {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM transactions
         WHERE origin_idempotency_key = ? AND origin_transaction_ordinal = ?",
    )
    .bind(origin_key)
    .bind(expected_ordinal)
    .fetch_one(pool)
    .await
    .expect("count transactions by origin");
    assert_eq!(count, expected_count);
}

pub async fn transaction_count_for_booking(pool: &Pool<Sqlite>, booking_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE booking_id = ?")
        .bind(booking_id)
        .fetch_one(pool)
        .await
        .expect("count booking transactions")
}

pub async fn transaction_sum(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    txn_type: &str,
    note: Option<&str>,
) -> i64 {
    let sql = match note {
        Some(_) => {
            "SELECT COALESCE(SUM(amount), 0) FROM transactions WHERE booking_id = ? AND type = ? AND note = ?"
        }
        None => {
            "SELECT COALESCE(SUM(amount), 0) FROM transactions WHERE booking_id = ? AND type = ?"
        }
    };
    let mut query = sqlx::query_scalar::<_, i64>(sql)
        .bind(booking_id)
        .bind(txn_type);
    if let Some(note) = note {
        query = query.bind(note);
    }
    query.fetch_one(pool).await.expect("sum transactions")
}

pub async fn assert_folio_origin(
    pool: &Pool<Sqlite>,
    origin_key: &str,
    expected_ordinal: i64,
    expected_count: i64,
) {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM folio_lines
         WHERE origin_idempotency_key = ? AND origin_line_ordinal = ?",
    )
    .bind(origin_key)
    .bind(expected_ordinal)
    .fetch_one(pool)
    .await
    .expect("count folio lines by origin");
    assert_eq!(count, expected_count);
}

pub async fn folio_line_count_for_key(pool: &Pool<Sqlite>, origin_key: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM folio_lines WHERE origin_idempotency_key = ?")
        .bind(origin_key)
        .fetch_one(pool)
        .await
        .expect("count folio lines by origin key")
}

pub async fn command_claim_count(
    pool: &Pool<Sqlite>,
    command_name: &str,
    idempotency_key: &str,
) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM command_idempotency
         WHERE command_name = ? AND idempotency_key = ?",
    )
    .bind(command_name)
    .bind(idempotency_key)
    .fetch_one(pool)
    .await
    .expect("count command claims by key")
}

pub async fn command_claim_count_by_request(
    pool: &Pool<Sqlite>,
    command_name: &str,
    request_id: &str,
) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM command_idempotency
         WHERE command_name = ? AND request_id = ?",
    )
    .bind(command_name)
    .bind(request_id)
    .fetch_one(pool)
    .await
    .expect("count command claims by request")
}

pub async fn outbox_count_for_command(
    pool: &Pool<Sqlite>,
    command_name: &str,
    idempotency_key: &str,
) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM outbox_events
         WHERE origin_command_name = ? AND origin_idempotency_key = ?",
    )
    .bind(command_name)
    .bind(idempotency_key)
    .fetch_one(pool)
    .await
    .expect("count outbox events by command")
}

pub async fn booking_export_row(
    pool: &Pool<Sqlite>,
    from_date: &str,
    to_date: &str,
    booking_id: &str,
) -> BookingExportRow {
    let rows = audit_queries::load_booking_export_rows(pool, from_date, to_date)
        .await
        .expect("load booking export rows");
    rows.into_iter()
        .find(|row| row.id == booking_id)
        .expect("booking export row exists")
}

pub async fn missing_booking_export_row(
    pool: &Pool<Sqlite>,
    from_date: &str,
    to_date: &str,
    booking_id: &str,
) -> bool {
    let rows = audit_queries::load_booking_export_rows(pool, from_date, to_date)
        .await
        .expect("load booking export rows");
    rows.into_iter().all(|row| row.id != booking_id)
}

pub async fn group_service_count_for_group(pool: &Pool<Sqlite>, group_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM group_services WHERE group_id = ?")
        .bind(group_id)
        .fetch_one(pool)
        .await
        .expect("count group services")
}

pub async fn booking_count_for_room(pool: &Pool<Sqlite>, room_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM bookings WHERE room_id = ?")
        .bind(room_id)
        .fetch_one(pool)
        .await
        .expect("count bookings by room")
}

pub async fn booking_guest_count_for_booking(pool: &Pool<Sqlite>, booking_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM booking_guests WHERE booking_id = ?")
        .bind(booking_id)
        .fetch_one(pool)
        .await
        .expect("count booking guests")
}

pub async fn calendar_count_for_booking(pool: &Pool<Sqlite>, booking_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM room_calendar WHERE booking_id = ?")
        .bind(booking_id)
        .fetch_one(pool)
        .await
        .expect("count calendar rows by booking")
}

pub async fn calendar_count_for_room_status(
    pool: &Pool<Sqlite>,
    room_id: &str,
    status: &str,
) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM room_calendar WHERE room_id = ? AND status = ?")
        .bind(room_id)
        .bind(status)
        .fetch_one(pool)
        .await
        .expect("count calendar rows by room status")
}

pub async fn folio_line_count_for_booking(pool: &Pool<Sqlite>, booking_id: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM folio_lines WHERE booking_id = ?")
        .bind(booking_id)
        .fetch_one(pool)
        .await
        .expect("count folio lines by booking")
}

pub async fn origin_transaction_count(pool: &Pool<Sqlite>, origin_key: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE origin_idempotency_key = ?")
        .bind(origin_key)
        .fetch_one(pool)
        .await
        .expect("count transactions by origin key")
}

pub async fn transaction_count_for_booking_type(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    txn_type: &str,
) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND type = ?")
        .bind(booking_id)
        .bind(txn_type)
        .fetch_one(pool)
        .await
        .expect("count transactions by booking and type")
}

pub async fn transaction_count_for_booking_type_note(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    txn_type: &str,
    note: &str,
) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND type = ? AND note = ?",
    )
    .bind(booking_id)
    .bind(txn_type)
    .bind(note)
    .fetch_one(pool)
    .await
    .expect("count transactions by booking, type, and note")
}

pub async fn transaction_count_for_note(pool: &Pool<Sqlite>, booking_id: &str, note: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM transactions WHERE booking_id = ? AND note = ?")
        .bind(booking_id)
        .bind(note)
        .fetch_one(pool)
        .await
        .expect("count transactions by booking and note")
}
