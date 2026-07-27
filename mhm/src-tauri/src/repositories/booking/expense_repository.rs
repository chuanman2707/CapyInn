//! Operating-expense writes.

use sqlx::{Pool, Sqlite};

use crate::models::Expense;

pub async fn insert_expense(pool: &Pool<Sqlite>, expense: &Expense) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO expenses (id, category, amount, note, expense_date, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&expense.id)
    .bind(&expense.category)
    .bind(expense.amount)
    .bind(&expense.note)
    .bind(&expense.expense_date)
    .bind(&expense.created_at)
    .execute(pool)
    .await?;

    Ok(())
}
