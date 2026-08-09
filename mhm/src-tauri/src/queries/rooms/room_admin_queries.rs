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

#[cfg(test)]
mod tests {
    use super::load_vacant_rooms;
    use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};

    /// Rooms are inserted in an order that is neither by floor nor by id, so a
    /// missing `ORDER BY` would show up as the insertion order leaking through.
    async fn seeded_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect in-memory sqlite");
        crate::db::run_migrations(&pool)
            .await
            .expect("run migrations");

        for (id, floor, room_type, status) in [
            ("305", 3, "deluxe", "vacant"),
            ("101", 1, "standard", "vacant"),
            ("204", 2, "standard", "occupied"),
            ("201", 2, "deluxe", "vacant"),
            ("102", 1, "deluxe", "vacant"),
            ("301", 3, "standard", "booked"),
        ] {
            sqlx::query(
                "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price,
                                    max_guests, extra_person_fee, status)
                 VALUES (?, ?, ?, ?, 0, 300000, 2, 0, ?)",
            )
            .bind(id)
            .bind(format!("Room {id}"))
            .bind(room_type)
            .bind(floor)
            .bind(status)
            .execute(&pool)
            .await
            .expect("seed room");
        }

        pool
    }

    async fn vacant_ids(pool: &Pool<Sqlite>, room_type: Option<&str>) -> Vec<String> {
        load_vacant_rooms(pool, room_type)
            .await
            .expect("load vacant rooms")
            .into_iter()
            .map(|room| room.id)
            .collect()
    }

    #[tokio::test]
    async fn only_vacant_rooms_come_back() {
        let pool = seeded_pool().await;

        let ids = vacant_ids(&pool, None).await;

        assert!(
            !ids.contains(&"204".to_string()) && !ids.contains(&"301".to_string()),
            "occupied and booked rooms are not vacant: {ids:?}"
        );
        assert_eq!(ids.len(), 4);
    }

    #[tokio::test]
    async fn the_order_is_by_floor_then_id_not_by_insertion() {
        let pool = seeded_pool().await;

        // Room 305 was inserted first; it must still come last.
        assert_eq!(
            vacant_ids(&pool, None).await,
            vec!["101", "102", "201", "305"]
        );
    }

    #[tokio::test]
    async fn the_type_filter_narrows_without_disturbing_the_order() {
        let pool = seeded_pool().await;

        assert_eq!(
            vacant_ids(&pool, Some("deluxe")).await,
            vec!["102", "201", "305"]
        );
        assert_eq!(vacant_ids(&pool, Some("standard")).await, vec!["101"]);
        assert!(
            vacant_ids(&pool, Some("presidential")).await.is_empty(),
            "an unknown type matches nothing rather than everything"
        );
    }

    #[tokio::test]
    async fn the_rows_decode_into_whole_rooms() {
        let pool = seeded_pool().await;

        let rooms = load_vacant_rooms(&pool, Some("deluxe"))
            .await
            .expect("load vacant rooms");

        assert_eq!(rooms[0].id, "102");
        assert_eq!(rooms[0].floor, 1);
        assert_eq!(rooms[0].room_type, "deluxe");
        assert_eq!(rooms[0].status, "vacant");
        assert_eq!(rooms[0].base_price, 300_000);
        assert_eq!(rooms[0].max_guests, 2);
    }
}
