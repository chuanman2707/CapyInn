use chrono::{DateTime, Duration, FixedOffset, Local, NaiveDate};
use sqlx::{sqlite::SqliteRow, Pool, Row, Sqlite, Transaction};

use crate::domain::booking::{BookingError, BookingResult};
use crate::models::Booking;

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CalendarInsertMode {
    Insert,
    InsertOrReplace,
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
    insert_mode: CalendarInsertMode,
) -> BookingResult<()> {
    let insert_sql = match insert_mode {
        CalendarInsertMode::Insert => {
            "INSERT INTO room_calendar (room_id, date, booking_id, status) VALUES (?, ?, ?, ?)"
        }
        CalendarInsertMode::InsertOrReplace => {
            "INSERT OR REPLACE INTO room_calendar (room_id, date, booking_id, status) VALUES (?, ?, ?, ?)"
        }
    };

    let mut date = from;
    while date < to {
        sqlx::query(insert_sql)
            .bind(room_id)
            .bind(date.format("%Y-%m-%d").to_string())
            .bind(booking_id)
            .bind(calendar_status)
            .execute(&mut **tx)
            .await?;
        date += Duration::days(1);
    }

    Ok(())
}

pub async fn fetch_booking<F>(
    pool: &Pool<Sqlite>,
    booking_id: &str,
    not_found_message: String,
    read_numeric: F,
) -> BookingResult<Booking>
where
    F: Fn(&SqliteRow, &str) -> f64,
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

pub fn read_f64_strict(row: &SqliteRow, column: &str) -> f64 {
    row.try_get::<f64, _>(column)
        .unwrap_or_else(|_| row.get::<i64, _>(column) as f64)
}

#[cfg(test)]
pub mod tests {
    use super::begin_immediate_tx;
    use sqlx::{sqlite::SqlitePoolOptions, Row};

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
