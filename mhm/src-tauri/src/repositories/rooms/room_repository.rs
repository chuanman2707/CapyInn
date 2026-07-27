//! Writes for room and room-type administration.
//!
//! These are the only functions that mutate `rooms` and `room_types` outside
//! the stay lifecycle. They take already-validated values and do not decide
//! policy: whether a room *may* be deleted is settled in
//! `services::rooms::room_service` before anything reaches here.

use sqlx::{Pool, Sqlite};

use crate::models::{Room, UpdateRoomRequest};

/// Returns the number of rows affected so the caller can tell "updated" from
/// "no such room" — a `COALESCE` update of a missing id is not an error.
pub async fn update_room(pool: &Pool<Sqlite>, req: &UpdateRoomRequest) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE rooms
         SET name = COALESCE(?, name),
             type = COALESCE(?, type),
             floor = COALESCE(?, floor),
             has_balcony = COALESCE(?, has_balcony),
             base_price = COALESCE(?, base_price),
             max_guests = COALESCE(?, max_guests),
             extra_person_fee = COALESCE(?, extra_person_fee)
         WHERE id = ?",
    )
    .bind(&req.name)
    .bind(&req.room_type)
    .bind(req.floor)
    .bind(req.has_balcony.map(|value| value as i32))
    .bind(req.base_price)
    .bind(req.max_guests)
    .bind(req.extra_person_fee)
    .bind(&req.room_id)
    .execute(pool)
    .await?;

    Ok(result.rows_affected())
}

pub async fn insert_room(pool: &Pool<Sqlite>, room: &Room) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&room.id)
    .bind(&room.name)
    .bind(&room.room_type)
    .bind(room.floor)
    .bind(room.has_balcony as i32)
    .bind(room.base_price)
    .bind(room.max_guests)
    .bind(room.extra_person_fee)
    .bind(&room.status)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn delete_room(pool: &Pool<Sqlite>, room_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM rooms WHERE id = ?")
        .bind(room_id)
        .execute(pool)
        .await?;

    Ok(())
}

pub async fn insert_room_type(
    pool: &Pool<Sqlite>,
    id: &str,
    name: &str,
    created_at: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO room_types (id, name, created_at) VALUES (?, ?, ?)")
        .bind(id)
        .bind(name)
        .bind(created_at)
        .execute(pool)
        .await?;

    Ok(())
}

pub async fn delete_room_type(pool: &Pool<Sqlite>, room_type_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM room_types WHERE id = ?")
        .bind(room_type_id)
        .execute(pool)
        .await?;

    Ok(())
}

/// The pre-insert existence check is advisory: a concurrent insert can still
/// lose the race, so both create paths also map the constraint violation back
/// to the same user-facing "already exists" error.
pub fn is_unique_constraint_error(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database_error)
            if database_error.message().contains("UNIQUE constraint failed")
                || database_error.message().contains("UNIQUE")
    )
}
