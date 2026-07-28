//! The reservations list read.
//!
//! The only query in the codebase that builds its WHERE clause at runtime. The
//! status filter appends fixed literals — the caller's string selects a branch,
//! it never reaches the SQL — while the date bounds are bound as parameters.

use sqlx::{Pool, Row, Sqlite};

use crate::db::row::{get_money_vnd, get_optional_money_vnd};
use crate::models::{BookingFilter, BookingWithGuest};

const BOOKING_LIST_SQL: &str = "SELECT b.id, b.room_id, r.name as room_name, g.full_name as guest_name,
                b.check_in_at, b.expected_checkout, b.actual_checkout,
                b.nights, b.total_price, b.paid_amount, b.status, b.source,
                b.booking_type, b.deposit_amount, b.scheduled_checkin, b.scheduled_checkout, b.guest_phone
         FROM bookings b
         JOIN rooms r ON r.id = b.room_id
         JOIN guests g ON g.id = b.primary_guest_id
         WHERE 1=1";

/// The UI's vocabulary is not the database's: `completed` means `checked_out`.
/// An unrecognised value filters nothing rather than erroring, which is the
/// behaviour the list has always had.
fn status_clause(status: &str) -> Option<&'static str> {
    match status {
        "active" => Some(" AND b.status = 'active'"),
        "completed" => Some(" AND b.status = 'checked_out'"),
        "booked" => Some(" AND b.status = 'booked'"),
        _ => None,
    }
}

pub async fn load_bookings_with_guest(
    pool: &Pool<Sqlite>,
    filter: Option<BookingFilter>,
) -> Result<Vec<BookingWithGuest>, sqlx::Error> {
    let mut sql = String::from(BOOKING_LIST_SQL);
    let mut binds: Vec<String> = Vec::new();

    if let Some(filter) = &filter {
        if let Some(status) = filter.status.as_deref().and_then(status_clause) {
            sql.push_str(status);
        }
        if let Some(from) = &filter.from {
            sql.push_str(" AND b.check_in_at >= ?");
            binds.push(from.clone());
        }
        if let Some(to) = &filter.to {
            sql.push_str(" AND b.expected_checkout <= ?");
            binds.push(to.clone());
        }
    }

    sql.push_str(" ORDER BY b.check_in_at DESC");

    let mut query = sqlx::query(&sql);
    for bind in &binds {
        query = query.bind(bind);
    }

    let rows = query.fetch_all(pool).await?;

    Ok(rows.iter().map(map_booking_with_guest).collect())
}

fn map_booking_with_guest(row: &sqlx::sqlite::SqliteRow) -> BookingWithGuest {
    BookingWithGuest {
        id: row.get("id"),
        room_id: row.get("room_id"),
        room_name: row.get("room_name"),
        guest_name: row.get("guest_name"),
        check_in_at: row.get("check_in_at"),
        expected_checkout: row.get("expected_checkout"),
        actual_checkout: row.get("actual_checkout"),
        nights: row.get("nights"),
        total_price: get_money_vnd(row, "total_price"),
        paid_amount: get_money_vnd(row, "paid_amount"),
        status: row.get("status"),
        source: row.get("source"),
        booking_type: row.get("booking_type"),
        deposit_amount: get_optional_money_vnd(row, "deposit_amount"),
        scheduled_checkin: row.get("scheduled_checkin"),
        scheduled_checkout: row.get("scheduled_checkout"),
        guest_phone: row.get("guest_phone"),
    }
}

#[cfg(test)]
mod tests {
    use super::{load_bookings_with_guest, status_clause};
    use crate::models::BookingFilter;
    use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};

    fn filter(status: Option<&str>, from: Option<&str>, to: Option<&str>) -> Option<BookingFilter> {
        Some(BookingFilter {
            status: status.map(str::to_string),
            from: from.map(str::to_string),
            to: to.map(str::to_string),
        })
    }

    async fn seeded_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect in-memory sqlite");
        crate::db::run_migrations(&pool)
            .await
            .expect("run migrations");

        sqlx::query(
            "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status)
             VALUES ('R1', 'Room 1', 'standard', 1, 0, 300000, 2, 0, 'vacant')",
        )
        .execute(&pool)
        .await
        .expect("seed room");
        sqlx::query(
            "INSERT INTO guests (id, guest_type, full_name, doc_number, created_at)
             VALUES ('G1', 'domestic', 'Nguyen Van A', 'DOC-1', '2026-04-01T00:00:00+07:00')",
        )
        .execute(&pool)
        .await
        .expect("seed guest");

        for (id, status, check_in, checkout) in [
            (
                "B-ACTIVE",
                "active",
                "2026-04-10T14:00:00+07:00",
                "2026-04-12T12:00:00+07:00",
            ),
            (
                "B-OUT",
                "checked_out",
                "2026-04-05T14:00:00+07:00",
                "2026-04-07T12:00:00+07:00",
            ),
            (
                "B-BOOKED",
                "booked",
                "2026-04-20T14:00:00+07:00",
                "2026-04-22T12:00:00+07:00",
            ),
        ] {
            sqlx::query(
                "INSERT INTO bookings (
                    id, room_id, primary_guest_id, check_in_at, expected_checkout, nights,
                    total_price, paid_amount, status, booking_type, pricing_type, created_at
                 ) VALUES (?, 'R1', 'G1', ?, ?, 2, 600000, 0, ?, 'walk-in', 'nightly', ?)",
            )
            .bind(id)
            .bind(check_in)
            .bind(checkout)
            .bind(status)
            .bind("2026-04-01T00:00:00+07:00")
            .execute(&pool)
            .await
            .expect("seed booking");
        }

        pool
    }

    #[tokio::test]
    async fn a_status_string_cannot_reach_the_sql() {
        let pool = seeded_pool().await;

        // The module doc claims the caller's string selects a branch and never
        // reaches the SQL. This is that claim as evidence rather than prose: a
        // payload that would be devastating if concatenated returns the
        // unfiltered list, because it simply fails to match any branch.
        for payload in [
            "' OR 1=1 --",
            "active'; DROP TABLE bookings; --",
            "\u{0}active",
        ] {
            assert_eq!(
                status_clause(payload),
                None,
                "payload matched a branch: {payload}"
            );

            let bookings = load_bookings_with_guest(&pool, filter(Some(payload), None, None))
                .await
                .expect("an unmatched status must not error");
            assert_eq!(bookings.len(), 3, "payload changed the result: {payload}");
        }

        let still_there: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM bookings")
            .fetch_one(&pool)
            .await
            .expect("count bookings");
        assert_eq!(still_there.0, 3, "the bookings table survived");
    }

    #[test]
    fn status_clause_translates_the_ui_vocabulary_and_ignores_the_unknown() {
        assert_eq!(status_clause("active"), Some(" AND b.status = 'active'"));
        assert_eq!(
            status_clause("completed"),
            Some(" AND b.status = 'checked_out'"),
            "the UI says completed, the column says checked_out"
        );
        assert_eq!(status_clause("booked"), Some(" AND b.status = 'booked'"));
        assert_eq!(status_clause("nonsense"), None);
        assert_eq!(status_clause(""), None);
    }

    #[tokio::test]
    async fn no_filter_returns_everything_newest_first() {
        let pool = seeded_pool().await;

        let bookings = load_bookings_with_guest(&pool, None)
            .await
            .expect("load bookings");

        let ids: Vec<&str> = bookings.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, vec!["B-BOOKED", "B-ACTIVE", "B-OUT"]);
        assert_eq!(bookings[0].room_name, "Room 1");
        assert_eq!(bookings[0].guest_name, "Nguyen Van A");
    }

    #[tokio::test]
    async fn the_completed_filter_selects_checked_out_bookings() {
        let pool = seeded_pool().await;

        let bookings = load_bookings_with_guest(&pool, filter(Some("completed"), None, None))
            .await
            .expect("load bookings");

        let ids: Vec<&str> = bookings.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, vec!["B-OUT"]);
    }

    #[tokio::test]
    async fn an_unknown_status_filters_nothing() {
        let pool = seeded_pool().await;

        let bookings = load_bookings_with_guest(&pool, filter(Some("nonsense"), None, None))
            .await
            .expect("load bookings");

        assert_eq!(bookings.len(), 3);
    }

    #[tokio::test]
    async fn the_date_bounds_are_inclusive_and_combine_with_status() {
        let pool = seeded_pool().await;

        let bookings =
            load_bookings_with_guest(&pool, filter(None, Some("2026-04-10T14:00:00+07:00"), None))
                .await
                .expect("load from");
        let ids: Vec<&str> = bookings.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, vec!["B-BOOKED", "B-ACTIVE"], "from is inclusive");

        let bookings = load_bookings_with_guest(
            &pool,
            filter(Some("active"), Some("2026-04-01"), Some("2026-04-13")),
        )
        .await
        .expect("load combined");
        let ids: Vec<&str> = bookings.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, vec!["B-ACTIVE"]);
    }
}
