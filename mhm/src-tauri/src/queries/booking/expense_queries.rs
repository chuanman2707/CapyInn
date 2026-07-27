//! Operating-expense reads.
//!
//! Expenses are part of financial reporting rather than a booking, but they feed
//! `revenue_queries`, which is why they live in the same context.

use sqlx::{Pool, Row, Sqlite};

use crate::db::row::get_money_vnd;
use crate::models::Expense;

/// `from`/`to` are inclusive `YYYY-MM-DD` bounds matched against `expense_date`.
pub async fn load_expenses_between(
    pool: &Pool<Sqlite>,
    from: &str,
    to: &str,
) -> Result<Vec<Expense>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, category, amount, note, expense_date, created_at
         FROM expenses
         WHERE expense_date BETWEEN ? AND ?
         ORDER BY expense_date DESC",
    )
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| Expense {
            id: row.get("id"),
            category: row.get("category"),
            amount: get_money_vnd(row, "amount"),
            note: row.get("note"),
            expense_date: row.get("expense_date"),
            created_at: row.get("created_at"),
        })
        .collect())
}
