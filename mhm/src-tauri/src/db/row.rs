//! Row-decoding helpers shared by the query, service, and command layers.
//!
//! These live in the database layer because every layer that reads a
//! `SqliteRow` may depend on `db`, while `db` depends on nobody. Keeping them
//! in `commands/` made inner layers (`queries/`, `services/`) import from the
//! outermost boundary, which inverts the intended dependency direction.

use sqlx::Row;

use crate::money::MoneyVnd;

/// Safely get an f64 from a SQLite row.
/// SQLite stores round numbers as INTEGER even in REAL columns,
/// so we try f64 first, then fall back to i64→f64.
pub(crate) fn get_f64(row: &sqlx::sqlite::SqliteRow, col: &str) -> f64 {
    row.try_get::<f64, _>(col)
        .unwrap_or_else(|_| row.get::<i64, _>(col) as f64)
}

pub(crate) fn get_money_vnd(row: &sqlx::sqlite::SqliteRow, col: &str) -> MoneyVnd {
    row.try_get::<MoneyVnd, _>(col).unwrap_or_else(|_| {
        let value = row.get::<f64, _>(col);
        assert!(
            value.is_finite() && value.fract() == 0.0,
            "money column {col} must be a whole VND amount"
        );
        value as MoneyVnd
    })
}

pub(crate) fn get_optional_money_vnd(row: &sqlx::sqlite::SqliteRow, col: &str) -> Option<MoneyVnd> {
    row.try_get::<Option<MoneyVnd>, _>(col).unwrap_or_else(|_| {
        row.try_get::<Option<f64>, _>(col)
            .ok()
            .flatten()
            .map(|value| {
                assert!(
                    value.is_finite() && value.fract() == 0.0,
                    "money column {col} must be a whole VND amount"
                );
                value as MoneyVnd
            })
    })
}
