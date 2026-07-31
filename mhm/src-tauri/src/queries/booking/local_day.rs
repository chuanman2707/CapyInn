//! What "a day" means in SQL, in one place.
//!
//! Timestamps in this database are written as **local** rfc3339
//! (`chrono::Local::now().to_rfc3339()`), and the hotel is at UTC+07:00. That
//! makes the first ten characters the local calendar date, already correct — so
//! a day is taken by `substr`, never by SQLite's `DATE()` on the raw stamp:
//!
//! ```text
//! DATE('2026-08-01T02:00:00+07:00')            -> '2026-07-31'   (the UTC day)
//! substr('2026-08-01T02:00:00+07:00', 1, 10)   -> '2026-08-01'   (the local day)
//! ```
//!
//! For seven hours of every Vietnamese day those two disagree, and the wrong one
//! is a day the desk never worked.
//!
//! These expressions also have to survive the column holding **two shapes**. The
//! same column carries both a bare `YYYY-MM-DD` and a full stamp — bookings
//! written by different paths over the app's life — and comparing those as raw
//! text is not the same question as comparing them as dates:
//!
//! ```text
//! '2026-04-12T12:00:00+07:00' <= '2026-04-12'   -> false   (text)
//! '2026-04-12'                <= '2026-04-12'   -> true    (text)
//! ```
//!
//! Two rows meaning the same day, answering a day-range filter differently.
//! Folding both sides through `local_date_sql` first is what makes the question
//! well-posed, which is why callers must fold the **bind** too and not just the
//! column.
//!
//! This module exists because that decision used to live inside
//! `revenue_queries`: `audit_queries` could reach only the primitive and so
//! re-spelled `DATE(substr(...))` by hand six times, and `booking_list_queries`
//! reached none of it and compared raw text instead.

/// The local calendar date of a local-rfc3339 stamp, as SQL.
///
/// `NULLIF(…, '')` keeps an empty string from becoming `''` rather than `NULL`,
/// so an unset column compares as unknown instead of as the year zero.
pub(crate) fn local_date_sql(expression: &str) -> String {
    format!("substr(NULLIF({expression}, ''), 1, 10)")
}

/// The same date as a SQLite date value, for comparing against other `DATE()`
/// expressions or doing calendar arithmetic on it.
pub(crate) fn date_sql(expression: &str) -> String {
    format!("DATE({})", local_date_sql(expression))
}

/// A local day shifted by whole days — the exclusive end of an inclusive range,
/// usually.
pub(crate) fn date_plus_days_sql(expression: &str, days: i64) -> String {
    format!("DATE({}, '+{days} day')", local_date_sql(expression))
}

#[cfg(test)]
mod tests {
    use super::{date_plus_days_sql, date_sql, local_date_sql};
    use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};

    async fn pool() -> Pool<Sqlite> {
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect in-memory sqlite")
    }

    async fn eval(pool: &Pool<Sqlite>, expression: &str) -> Option<String> {
        sqlx::query_scalar::<_, Option<String>>(&format!("SELECT {expression}"))
            .fetch_one(pool)
            .await
            .expect("evaluate expression")
    }

    /// The whole reason this is `substr` and not `DATE`. A stamp just after local
    /// midnight is still the previous day in UTC, and `DATE()` would report that
    /// day — the one the desk had already closed.
    #[tokio::test]
    async fn a_stamp_just_after_local_midnight_keeps_its_local_day() {
        let pool = pool().await;
        let stamp = "'2026-08-01T02:00:00+07:00'";

        assert_eq!(
            eval(&pool, &local_date_sql(stamp)).await.as_deref(),
            Some("2026-08-01"),
        );
        assert_eq!(
            eval(&pool, &format!("DATE({stamp})")).await.as_deref(),
            Some("2026-07-31"),
            "if this ever agrees with the line above, the UTC hazard is gone \
             and this module has less to justify it",
        );
    }

    /// The two shapes the column actually holds have to land on one answer.
    #[tokio::test]
    async fn both_stored_shapes_reduce_to_the_same_day() {
        let pool = pool().await;

        let from_stamp = eval(&pool, &local_date_sql("'2026-04-12T12:00:00+07:00'")).await;
        let from_date = eval(&pool, &local_date_sql("'2026-04-12'")).await;

        assert_eq!(from_stamp, from_date, "same day, two shapes, one answer");
        assert_eq!(from_stamp.as_deref(), Some("2026-04-12"));
    }

    #[tokio::test]
    async fn an_empty_stamp_is_unknown_rather_than_a_date() {
        let pool = pool().await;

        assert_eq!(eval(&pool, &local_date_sql("''")).await, None);
    }

    #[tokio::test]
    async fn the_date_forms_build_on_the_local_day() {
        let pool = pool().await;
        let stamp = "'2026-08-01T02:00:00+07:00'";

        assert_eq!(
            eval(&pool, &date_sql(stamp)).await.as_deref(),
            Some("2026-08-01"),
        );
        assert_eq!(
            eval(&pool, &date_plus_days_sql(stamp, 1)).await.as_deref(),
            Some("2026-08-02"),
        );
    }
}
