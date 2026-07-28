//! Reads over the `users` table.

use sqlx::{Pool, Row, Sqlite};

use crate::models::User;

const USER_COLUMNS: &str = "SELECT id, name, role, active, created_at FROM users";

/// The single active user holding this PIN hash, if any.
///
/// Inactive users are filtered in SQL rather than by the caller: an
/// unauthenticated login must not be able to tell a deactivated account from a
/// wrong PIN.
pub async fn load_active_user_by_pin_hash(
    pool: &Pool<Sqlite>,
    pin_hash: &str,
) -> Result<Option<User>, sqlx::Error> {
    let row = sqlx::query(&format!("{USER_COLUMNS} WHERE pin_hash = ? AND active = 1"))
        .bind(pin_hash)
        .fetch_optional(pool)
        .await?;

    Ok(row.as_ref().map(map_user))
}

pub async fn load_users(pool: &Pool<Sqlite>) -> Result<Vec<User>, sqlx::Error> {
    let rows = sqlx::query(&format!("{USER_COLUMNS} ORDER BY created_at"))
        .fetch_all(pool)
        .await?;

    Ok(rows.iter().map(map_user).collect())
}

fn map_user(row: &sqlx::sqlite::SqliteRow) -> User {
    User {
        id: row.get("id"),
        name: row.get("name"),
        role: row.get("role"),
        active: row.get::<i32, _>("active") == 1,
        created_at: row.get("created_at"),
    }
}
