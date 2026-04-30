use chrono::{DateTime, Duration, FixedOffset, Local, NaiveDate};
use sqlx::{sqlite::SqliteRow, Pool, Row, Sqlite, Transaction};

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

pub fn read_f64_or_zero(row: &SqliteRow, column: &str) -> f64 {
    row.try_get::<Option<f64>, _>(column)
        .ok()
        .flatten()
        .or_else(|| {
            row.try_get::<Option<i64>, _>(column)
                .ok()
                .flatten()
                .map(|value| value as f64)
        })
        .unwrap_or(0.0)
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

#[cfg(test)]
pub mod tests {
    use super::{begin_immediate_tx, invalid_state_transition};
    use sqlx::{sqlite::SqlitePoolOptions, Row};

    #[test]
    fn invalid_state_transition_error_includes_shared_code() {
        let error = invalid_state_transition("booking is no longer active");
        assert!(error
            .to_string()
            .contains(crate::app_error::codes::CONFLICT_INVALID_STATE_TRANSITION));
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
