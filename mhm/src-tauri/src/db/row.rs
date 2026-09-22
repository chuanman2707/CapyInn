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

/// Đọc cột tiền. Một giá trị REAL lẻ là lỗi dữ liệu — nhưng phải là Err,
/// không được panic: panic trong `#[tauri::command]` không reject invoke
/// promise mà treo nó mãi mãi, đóng băng chuỗi await phía frontend (sự cố
/// "mini-tab đơ sau mỗi action" 2026-09).
pub(crate) fn get_money_vnd(
    row: &sqlx::sqlite::SqliteRow,
    col: &str,
) -> Result<MoneyVnd, sqlx::Error> {
    match row.try_get::<MoneyVnd, _>(col) {
        Ok(value) => Ok(value),
        Err(_) => match row.try_get::<f64, _>(col)? {
            value if value.is_finite() && value.fract() == 0.0 => Ok(value as MoneyVnd),
            value => Err(sqlx::Error::Decode(
                format!("money column {col} must be a whole VND amount, got {value}").into(),
            )),
        },
    }
}

pub(crate) fn get_optional_money_vnd(
    row: &sqlx::sqlite::SqliteRow,
    col: &str,
) -> Result<Option<MoneyVnd>, sqlx::Error> {
    match row.try_get::<Option<MoneyVnd>, _>(col) {
        Ok(value) => Ok(value),
        Err(_) => match row.try_get::<Option<f64>, _>(col)? {
            Some(value) if value.is_finite() && value.fract() == 0.0 => Ok(Some(value as MoneyVnd)),
            Some(value) => Err(sqlx::Error::Decode(
                format!("money column {col} must be a whole VND amount, got {value}").into(),
            )),
            None => Ok(None),
        },
    }
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    async fn pool() -> sqlx::Pool<sqlx::Sqlite> {
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect in-memory sqlite")
    }

    #[tokio::test]
    async fn fractional_money_column_is_a_decode_error_not_a_panic() {
        let pool = pool().await;
        let row = sqlx::query("SELECT CAST(133333.5 AS REAL) AS value")
            .fetch_one(&pool)
            .await
            .unwrap();

        assert!(get_money_vnd(&row, "value").is_err());
        assert!(get_optional_money_vnd(&row, "value").is_err());
    }

    #[tokio::test]
    async fn whole_real_money_column_still_decodes() {
        let pool = pool().await;
        let row = sqlx::query("SELECT CAST(400000.0 AS REAL) AS value")
            .fetch_one(&pool)
            .await
            .unwrap();

        assert_eq!(get_money_vnd(&row, "value").unwrap(), 400_000);
        assert_eq!(
            get_optional_money_vnd(&row, "value").unwrap(),
            Some(400_000)
        );
    }
}
