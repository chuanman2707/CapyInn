use sqlx::{Pool, Row, Sqlite};

use crate::{commands::get_money_vnd, models::FolioLine};

pub async fn list_folio_lines(
    pool: &Pool<Sqlite>,
    booking_id: &str,
) -> Result<Vec<FolioLine>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, booking_id, category, description, amount, created_by, created_at
         FROM folio_lines
         WHERE booking_id = ?
         ORDER BY created_at",
    )
    .bind(booking_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| FolioLine {
            id: row.get("id"),
            booking_id: row.get("booking_id"),
            category: row.get("category"),
            description: row.get("description"),
            amount: get_money_vnd(row, "amount"),
            created_by: row.get("created_by"),
            created_at: row.get("created_at"),
        })
        .collect())
}
