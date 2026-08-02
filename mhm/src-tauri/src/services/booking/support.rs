use chrono::{DateTime, Duration, FixedOffset, Local, NaiveDate};
use sqlx::{sqlite::SqliteRow, Pool, Row, Sqlite, Transaction};

use crate::app_error::{CommandError, CommandResult};
use crate::domain::booking::{BookingError, BookingResult};
use crate::models::Booking;
use crate::money::{validate_non_negative_money_vnd, MoneyVnd};

pub async fn begin_tx<'a>(pool: &'a Pool<Sqlite>) -> BookingResult<Transaction<'a, Sqlite>> {
    pool.begin().await.map_err(BookingError::from)
}

pub async fn begin_immediate_tx<'a>(
    pool: &'a Pool<Sqlite>,
) -> BookingResult<Transaction<'a, Sqlite>> {
    pool.begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(BookingError::from)
}

pub fn invalid_state_transition(message: impl AsRef<str>) -> BookingError {
    BookingError::conflict(format!(
        "{}: {}",
        crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION,
        message.as_ref()
    ))
}

pub fn ensure_one_row_affected(
    result: sqlx::sqlite::SqliteQueryResult,
    message: impl AsRef<str>,
) -> BookingResult<()> {
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(invalid_state_transition(message))
    }
}

pub fn ensure_rows_affected(
    result: sqlx::sqlite::SqliteQueryResult,
    expected: u64,
    message: impl AsRef<str>,
) -> BookingResult<()> {
    if result.rows_affected() == expected {
        Ok(())
    } else {
        Err(invalid_state_transition(message))
    }
}

pub async fn lookup_booking_room_id(
    pool: &Pool<Sqlite>,
    booking_id: &str,
) -> BookingResult<String> {
    sqlx::query_scalar::<_, String>("SELECT room_id FROM bookings WHERE id = ?")
        .bind(booking_id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| BookingError::not_found(format!("Không tìm thấy booking {}", booking_id)))
}

pub fn rfc3339_now() -> String {
    Local::now().to_rfc3339()
}

pub fn parse_booking_datetime(value: &str) -> BookingResult<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value)
        .map_err(|error| BookingError::datetime_parse(error.to_string()))
}

pub async fn insert_room_calendar_rows(
    tx: &mut Transaction<'_, Sqlite>,
    room_id: &str,
    booking_id: &str,
    from: NaiveDate,
    to: NaiveDate,
    calendar_status: &str,
) -> BookingResult<()> {
    let mut date = from;
    while date < to {
        sqlx::query(
            "INSERT INTO room_calendar (room_id, date, booking_id, status) VALUES (?, ?, ?, ?)",
        )
        .bind(room_id)
        .bind(date.format("%Y-%m-%d").to_string())
        .bind(booking_id)
        .bind(calendar_status)
        .execute(&mut **tx)
        .await
        .map_err(|error| map_room_calendar_insert_error(error, date))?;
        date += Duration::days(1);
    }

    Ok(())
}

pub(crate) fn map_room_calendar_insert_error(error: sqlx::Error, date: NaiveDate) -> BookingError {
    let message = error.to_string();
    if crate::db_error_monitoring::classify_db_error_code(&message)
        == Some(crate::app_error::codes::CONFLICT_ROOM_UNAVAILABLE)
    {
        return BookingError::conflict(format!(
            "{}: room_calendar conflict on {}",
            crate::app_error::codes::CONFLICT_ROOM_UNAVAILABLE,
            date.format("%Y-%m-%d")
        ));
    }

    BookingError::from(error)
}

/// One **occupancy segment**: a run of consecutive nights the booking spent in
/// one room, in date order. Feeds `pricing_snapshot.room_stays`: written
/// wholesale by `room_change::change_room_tx` at move time, and re-derived by
/// `stay_lifecycle::check_out_tx` (then truncated to the settled night count)
/// and `group_lifecycle::group_checkout_tx` right before they delete the
/// booking's `room_calendar` rows.
///
/// A segment, not a room: a guest who moves A → B → A produces three rows, not
/// two. Grouping by room instead would collapse the two A stays into one row
/// carrying the *earliest* `first_night`, and every consumer that walks the
/// list in stay order (`truncate_room_stays_to_settled_nights`, the invoice
/// split) would then hand B's nights to A.
pub struct RoomStayRow {
    pub room_id: String,
    pub room_name: String,
    pub nights: i64,
    pub first_night: String,
}

pub async fn room_calendar_stays_tx(
    tx: &mut Transaction<'_, Sqlite>,
    booking_id: &str,
) -> BookingResult<Vec<RoomStayRow>> {
    // One row per night, in date order, then run-length encoded below. The
    // `room_id` tiebreak only matters for the pathological case of two rooms
    // holding the same booking on the same date; it keeps the output stable.
    let rows = sqlx::query(
        "SELECT rc.room_id, r.name AS room_name, rc.date
         FROM room_calendar rc
         JOIN rooms r ON r.id = rc.room_id
         WHERE rc.booking_id = ?
         ORDER BY rc.date, rc.room_id",
    )
    .bind(booking_id)
    .fetch_all(&mut **tx)
    .await?;

    let mut segments: Vec<RoomStayRow> = Vec::new();
    for row in rows {
        let room_id: String = row.get("room_id");
        let room_name: String = row.get("room_name");
        let date: String = row.get("date");
        match segments.last_mut() {
            Some(last) if last.room_id == room_id => last.nights += 1,
            _ => segments.push(RoomStayRow {
                room_id,
                room_name,
                nights: 1,
                first_night: date,
            }),
        }
    }

    Ok(segments)
}

/// Serializes `RoomStayRow`s into the JSON shape stored under
/// `pricing_snapshot.room_stays` and read back by `invoice_generation.rs`.
pub fn room_stays_to_json(rows: &[RoomStayRow]) -> serde_json::Value {
    serde_json::Value::Array(
        rows.iter()
            .map(|row| {
                serde_json::json!({
                    "room_id": row.room_id,
                    "room_name": row.room_name,
                    "nights": row.nights,
                    "first_night": row.first_night,
                })
            })
            .collect(),
    )
}

#[allow(dead_code)]
pub async fn fetch_booking<F>(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    not_found_message: String,
    read_numeric: F,
) -> BookingResult<Booking>
where
    F: Fn(&SqliteRow, &str) -> MoneyVnd,
{
    let row = sqlx::query(
        "SELECT id, room_id, primary_guest_id, check_in_at, expected_checkout,
                actual_checkout, nights, total_price, paid_amount, status,
                source, notes, created_at
         FROM bookings WHERE id = ?",
    )
    .bind(booking_id)
    .fetch_optional(pool)
    .await?;

    let row = row.ok_or_else(|| BookingError::not_found(not_found_message))?;

    Ok(Booking {
        id: row.get("id"),
        room_id: row.get("room_id"),
        primary_guest_id: row.get("primary_guest_id"),
        check_in_at: row.get("check_in_at"),
        expected_checkout: row.get("expected_checkout"),
        actual_checkout: row.get("actual_checkout"),
        nights: row.get("nights"),
        total_price: read_numeric(&row, "total_price"),
        paid_amount: read_numeric(&row, "paid_amount"),
        status: row.get("status"),
        source: row.get("source"),
        notes: row.get("notes"),
        created_at: row.get("created_at"),
    })
}

pub fn read_money_vnd_or_zero(row: &SqliteRow, column: &str) -> MoneyVnd {
    row.try_get::<Option<MoneyVnd>, _>(column)
        .ok()
        .flatten()
        .or_else(|| {
            row.try_get::<Option<f64>, _>(column)
                .ok()
                .flatten()
                .map(|value| {
                    assert!(
                        value.is_finite() && value.fract() == 0.0,
                        "money column {column} must be a whole VND amount"
                    );
                    value as MoneyVnd
                })
        })
        .unwrap_or(0)
}

pub fn read_money_vnd_strict(row: &SqliteRow, column: &str) -> MoneyVnd {
    row.try_get::<MoneyVnd, _>(column).unwrap_or_else(|_| {
        let value = row.get::<f64, _>(column);
        assert!(
            value.is_finite() && value.fract() == 0.0,
            "money column {column} must be a whole VND amount"
        );
        value as MoneyVnd
    })
}

pub fn validate_non_negative_booking_money(
    value: MoneyVnd,
    field: &str,
) -> BookingResult<MoneyVnd> {
    validate_non_negative_money_vnd(value, field)
        .map_err(|error| BookingError::validation(error.message))
}

/// Merge `value` under `key` into the JSON object held by a booking's
/// `pricing_snapshot` column, preserving every other key already present.
///
/// `pricing_snapshot` is a shared scratch space (`checkout_settlement`,
/// `room_stays`, ...); callers that write one key must never clobber the
/// others. A booking with no prior snapshot (`existing == None`) or a
/// snapshot that failed to parse as a JSON object starts from `{}`.
pub fn merge_pricing_snapshot(
    existing: Option<&str>,
    key: &str,
    value: serde_json::Value,
) -> String {
    let mut map = existing
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|parsed| match parsed {
            serde_json::Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default();
    map.insert(key.to_string(), value);
    serde_json::Value::Object(map).to_string()
}

/// Có lấy kèm khoá `folio:` ở pha 1 hay không. Ba lệnh reservation hôm nay chỉ
/// khoá `booking` + `room`; giữ nguyên bộ khoá đó, đổi thứ tự thôi.
pub enum FolioLock {
    Include,
    #[allow(dead_code)]
    Skip,
}

/// Pha 1 của mọi lệnh ghi phạm vi-booking: lấy khoá `booking:` (kèm `folio:`
/// nếu cần) **rồi mới** đọc phòng.
///
/// Đọc sau khi đã cầm `booking:B` là chân lý, vì mọi lệnh ghi muốn dời một
/// booking sang phòng khác đều phải cầm đúng khoá đó. Người gọi tự ráp pha 2
/// bằng `guard.acquire_next(...)` — `change_room` cần hai khoá phòng chứ không
/// phải một, nên pha 2 không gói được vào đây.
pub async fn lock_booking_and_read_room(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    folio: FolioLock,
    map_err: fn(BookingError) -> CommandError,
) -> CommandResult<(crate::aggregate_locks::AggregateLockGuard, String)> {
    let mut phase_one = vec![crate::aggregate_locks::booking_key(booking_id)?];
    if matches!(folio, FolioLock::Include) {
        phase_one.push(crate::aggregate_locks::folio_key(booking_id)?);
    }

    let guard = crate::aggregate_locks::global_manager()
        .acquire(phase_one)
        .await?;
    let room_id = lookup_booking_room_id(pool, booking_id)
        .await
        .map_err(map_err)?;

    Ok((guard, room_id))
}

#[cfg(test)]
pub mod tests {
    use super::{begin_immediate_tx, invalid_state_transition, merge_pricing_snapshot};
    use sqlx::{sqlite::SqlitePoolOptions, Row};

    #[test]
    fn invalid_state_transition_error_includes_shared_code() {
        let error = invalid_state_transition("booking is no longer active");
        assert!(error
            .to_string()
            .contains(crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION));
    }

    #[test]
    fn merge_pricing_snapshot_starts_from_empty_object_when_none() {
        let result = merge_pricing_snapshot(None, "room_stays", serde_json::json!([1, 2]));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(value, serde_json::json!({"room_stays": [1, 2]}));
    }

    #[test]
    fn merge_pricing_snapshot_preserves_other_keys() {
        let existing = r#"{"checkout_settlement":{"mode":"hourly"}}"#;
        let result = merge_pricing_snapshot(Some(existing), "room_stays", serde_json::json!([3]));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "checkout_settlement": {"mode": "hourly"},
                "room_stays": [3],
            })
        );
    }

    #[test]
    fn merge_pricing_snapshot_overwrites_only_the_targeted_key() {
        let existing = r#"{"room_stays":[1],"checkout_settlement":{"mode":"hourly"}}"#;
        let result = merge_pricing_snapshot(Some(existing), "room_stays", serde_json::json!([2]));
        let value: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "checkout_settlement": {"mode": "hourly"},
                "room_stays": [2],
            })
        );
    }

    #[tokio::test]
    async fn begin_immediate_tx_starts_transaction_that_can_write() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connects in-memory sqlite");

        sqlx::query("CREATE TABLE sample (id INTEGER PRIMARY KEY, value TEXT NOT NULL)")
            .execute(&pool)
            .await
            .expect("creates sample table");

        let mut tx = begin_immediate_tx(&pool)
            .await
            .expect("starts immediate transaction");
        sqlx::query("INSERT INTO sample (value) VALUES ('written')")
            .execute(&mut *tx)
            .await
            .expect("writes inside immediate transaction");
        tx.commit().await.expect("commits immediate transaction");

        let count: i64 = sqlx::query("SELECT COUNT(*) AS count FROM sample")
            .fetch_one(&pool)
            .await
            .expect("reads sample rows")
            .get("count");

        assert_eq!(count, 1);
    }
}
