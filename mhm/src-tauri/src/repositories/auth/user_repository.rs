//! Writes over the `users` table.

use sqlx::{Pool, Sqlite};

pub struct NewUser<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub pin_hash: &'a str,
    pub role: &'a str,
    pub created_at: &'a str,
}

/// Inserts an active user. The PIN hash must come from
/// `domain::auth::credentials::pin_hash` — this layer stores it, it does not
/// derive it.
pub async fn insert_user(pool: &Pool<Sqlite>, user: NewUser<'_>) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO users (id, name, pin_hash, role, active, created_at)
         VALUES (?, ?, ?, ?, 1, ?)",
    )
    .bind(user.id)
    .bind(user.name)
    .bind(user.pin_hash)
    .bind(user.role)
    .bind(user.created_at)
    .execute(pool)
    .await?;

    Ok(())
}
