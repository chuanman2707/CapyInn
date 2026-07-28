//! Reads behind room and room-type administration.
//!
//! Split out of `commands::room_management`, which had accumulated the reads,
//! the writes, and the delete-safety rules in the same functions. Everything
//! here returns raw `sqlx::Error`; deciding what a missing row *means* is the
//! service's job, not the query's.
//!
//! Room *listing* for the floor plan lives in `queries::booking::room_queries`
//! and is unrelated: these are the reads an admin screen needs.

use sqlx::{Pool, Row, Sqlite};

use crate::models::{Room, RoomType};
use crate::queries::booking::room_queries::map_room;

const ROOM_BY_ID_SQL: &str =
    "SELECT id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status
     FROM rooms WHERE id = ?";

pub async fn load_room(pool: &Pool<Sqlite>, room_id: &str) -> Result<Option<Room>, sqlx::Error> {
    let row = sqlx::query(ROOM_BY_ID_SQL)
        .bind(room_id)
        .fetch_optional(pool)
        .await?;

    Ok(row.as_ref().map(map_room))
}

/// Vacant rooms, optionally of one type, ordered by floor then id.
///
/// The order matters to the caller: `domain::booking::room_allocation` groups
/// by floor and needs a stable sequence within each floor.
pub async fn load_vacant_rooms(
    pool: &Pool<Sqlite>,
    room_type: Option<&str>,
) -> Result<Vec<Room>, sqlx::Error> {
    let rows = match room_type {
        Some(room_type) => {
            sqlx::query(
                "SELECT * FROM rooms WHERE status = 'vacant' AND type = ? ORDER BY floor, id",
            )
            .bind(room_type)
            .fetch_all(pool)
            .await?
        }
        None => {
            sqlx::query("SELECT * FROM rooms WHERE status = 'vacant' ORDER BY floor, id")
                .fetch_all(pool)
                .await?
        }
    };

    Ok(rows.iter().map(map_room).collect())
}

pub async fn room_exists(pool: &Pool<Sqlite>, room_id: &str) -> Result<bool, sqlx::Error> {
    let existing: Option<(String,)> = sqlx::query_as("SELECT id FROM rooms WHERE id = ?")
        .bind(room_id)
        .fetch_optional(pool)
        .await?;

    Ok(existing.is_some())
}

/// `None` means the room does not exist, which the caller reports differently
/// from a room that exists and is free.
pub async fn load_room_status(
    pool: &Pool<Sqlite>,
    room_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query("SELECT status FROM rooms WHERE id = ?")
        .bind(room_id)
        .fetch_optional(pool)
        .await?;

    Ok(row.map(|row| row.get("status")))
}

pub async fn count_active_bookings_for_room(
    pool: &Pool<Sqlite>,
    room_id: &str,
) -> Result<i64, sqlx::Error> {
    let count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM bookings WHERE room_id = ? AND status = 'active'")
            .bind(room_id)
            .fetch_one(pool)
            .await?;

    Ok(count.0)
}

pub async fn load_room_types(pool: &Pool<Sqlite>) -> Result<Vec<RoomType>, sqlx::Error> {
    let rows = sqlx::query("SELECT id, name, created_at FROM room_types ORDER BY name")
        .fetch_all(pool)
        .await?;

    Ok(rows
        .iter()
        .map(|row| RoomType {
            id: row.get("id"),
            name: row.get("name"),
            created_at: row.get("created_at"),
        })
        .collect())
}

/// A room type is taken if either the derived id or the display name is used —
/// two different names can slugify to the same id.
pub async fn room_type_exists(
    pool: &Pool<Sqlite>,
    id: &str,
    name: &str,
) -> Result<bool, sqlx::Error> {
    let existing: Option<(String,)> =
        sqlx::query_as("SELECT id FROM room_types WHERE id = ? OR name = ?")
            .bind(id)
            .bind(name)
            .fetch_optional(pool)
            .await?;

    Ok(existing.is_some())
}

pub async fn load_room_type_name(
    pool: &Pool<Sqlite>,
    room_type_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query("SELECT name FROM room_types WHERE id = ?")
        .bind(room_type_id)
        .fetch_optional(pool)
        .await?;

    Ok(row.map(|row| row.get("name")))
}

/// `rooms.type` stores the display name, not the room-type id.
pub async fn count_rooms_with_type(
    pool: &Pool<Sqlite>,
    type_name: &str,
) -> Result<i64, sqlx::Error> {
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM rooms WHERE type = ?")
        .bind(type_name)
        .fetch_one(pool)
        .await?;

    Ok(count.0)
}
