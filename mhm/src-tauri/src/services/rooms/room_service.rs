//! Room and room-type administration rules.
//!
//! This is where the delete-safety rules live: a room with guests in it or an
//! active booking cannot be removed, and a room type still referenced by a room
//! cannot be removed. `commands::room_management` used to hold these rules
//! interleaved with the SQL they depend on; the reads now come from
//! `queries::rooms::room_admin_queries` and the writes go through
//! `repositories::rooms::room_repository`.
//!
//! The `log_system_error` step names are preserved verbatim from the command
//! module so existing support logs stay greppable.

use serde_json::json;
use sqlx::{Pool, Sqlite};

use crate::app_error::{codes, log_system_error, CommandError, CommandResult};
use crate::models::{CreateRoomRequest, CreateRoomTypeRequest, Room, RoomType, UpdateRoomRequest};
use crate::money::validate_non_negative_money_vnd;
use crate::queries::rooms::room_admin_queries as room_reads;
use crate::repositories::rooms::room_repository as room_writes;

pub async fn update_room(pool: &Pool<Sqlite>, req: UpdateRoomRequest) -> CommandResult<Room> {
    let mut req = req;
    req.base_price = req
        .base_price
        .map(|value| validate_non_negative_money_vnd(value, "base_price"))
        .transpose()?;
    req.extra_person_fee = req
        .extra_person_fee
        .map(|value| validate_non_negative_money_vnd(value, "extra_person_fee"))
        .transpose()?;

    let rows_affected = room_writes::update_room(pool, &req)
        .await
        .map_err(|error| {
            log_system_error(
                "update_room",
                error.to_string(),
                json!({ "step": "update_room", "room_id": &req.room_id }),
            )
        })?;

    if rows_affected == 0 {
        return Err(CommandError::user(
            codes::ROOM_NOT_FOUND,
            "Phòng không tồn tại",
        ));
    }

    room_reads::load_room(pool, &req.room_id)
        .await
        .map_err(|error| {
            log_system_error(
                "update_room",
                error.to_string(),
                json!({ "step": "fetch_updated_room", "room_id": &req.room_id }),
            )
        })?
        .ok_or_else(|| CommandError::user(codes::ROOM_NOT_FOUND, "Phòng không tồn tại"))
}

pub async fn create_room(pool: &Pool<Sqlite>, req: CreateRoomRequest) -> CommandResult<Room> {
    let room = Room {
        id: req.id,
        name: req.name,
        room_type: req.room_type,
        floor: req.floor,
        has_balcony: req.has_balcony,
        base_price: validate_non_negative_money_vnd(req.base_price, "base_price")?,
        max_guests: req.max_guests,
        extra_person_fee: validate_non_negative_money_vnd(
            req.extra_person_fee,
            "extra_person_fee",
        )?,
        status: "vacant".to_string(),
    };

    let taken = room_reads::room_exists(pool, &room.id)
        .await
        .map_err(|error| {
            log_system_error(
                "create_room",
                error.to_string(),
                json!({ "step": "check_existing_room", "room_id": &room.id }),
            )
        })?;
    if taken {
        return Err(room_already_exists());
    }

    room_writes::insert_room(pool, &room)
        .await
        .map_err(|error| {
            if room_writes::is_unique_constraint_error(&error) {
                room_already_exists()
            } else {
                log_system_error(
                    "create_room",
                    error.to_string(),
                    json!({ "step": "insert_room", "room_id": &room.id }),
                )
            }
        })?;

    Ok(room)
}

pub async fn delete_room(pool: &Pool<Sqlite>, room_id: &str) -> CommandResult<()> {
    let status = room_reads::load_room_status(pool, room_id)
        .await
        .map_err(|error| {
            log_system_error(
                "delete_room",
                error.to_string(),
                json!({ "step": "fetch_room", "room_id": room_id }),
            )
        })?
        .ok_or_else(|| CommandError::user(codes::ROOM_NOT_FOUND, "Phòng không tồn tại"))?;

    if status == "occupied" {
        return Err(CommandError::user(
            codes::ROOM_DELETE_OCCUPIED,
            "Không thể xóa phòng đang có khách",
        ));
    }

    let active_bookings = room_reads::count_active_bookings_for_room(pool, room_id)
        .await
        .map_err(|error| {
            log_system_error(
                "delete_room",
                error.to_string(),
                json!({ "step": "count_active_bookings", "room_id": room_id }),
            )
        })?;
    if active_bookings > 0 {
        return Err(CommandError::user(
            codes::ROOM_DELETE_ACTIVE_BOOKING,
            "Không thể xóa phòng có booking đang hoạt động",
        ));
    }

    room_writes::delete_room(pool, room_id)
        .await
        .map_err(|error| {
            log_system_error(
                "delete_room",
                error.to_string(),
                json!({ "step": "delete_room", "room_id": room_id }),
            )
        })
}

pub async fn list_room_types(pool: &Pool<Sqlite>) -> Result<Vec<RoomType>, String> {
    room_reads::load_room_types(pool)
        .await
        .map_err(|error| error.to_string())
}

pub async fn create_room_type(
    pool: &Pool<Sqlite>,
    req: CreateRoomTypeRequest,
) -> CommandResult<RoomType> {
    let name = req.name;
    let id = name.to_lowercase().replace(' ', "_");
    let now = chrono::Local::now().to_rfc3339();

    let taken = room_reads::room_type_exists(pool, &id, &name)
        .await
        .map_err(|error| {
            log_system_error(
                "create_room_type",
                error.to_string(),
                json!({ "step": "check_existing_room_type", "room_type_id": &id, "room_type_name": &name }),
            )
        })?;
    if taken {
        return Err(room_type_already_exists());
    }

    room_writes::insert_room_type(pool, &id, &name, &now)
        .await
        .map_err(|error| {
            if room_writes::is_unique_constraint_error(&error) {
                room_type_already_exists()
            } else {
                log_system_error(
                    "create_room_type",
                    error.to_string(),
                    json!({ "step": "insert_room_type", "room_type_id": &id, "room_type_name": &name }),
                )
            }
        })?;

    Ok(RoomType {
        id,
        name,
        created_at: now,
    })
}

pub async fn delete_room_type(pool: &Pool<Sqlite>, room_type_id: &str) -> CommandResult<()> {
    let type_name = room_reads::load_room_type_name(pool, room_type_id)
        .await
        .map_err(|error| {
            log_system_error(
                "delete_room_type",
                error.to_string(),
                json!({ "step": "fetch_room_type", "room_type_id": room_type_id }),
            )
        })?
        .ok_or_else(|| {
            CommandError::user(codes::ROOM_TYPE_NOT_FOUND, "Loại phòng không tồn tại")
        })?;

    let in_use = room_reads::count_rooms_with_type(pool, &type_name)
        .await
        .map_err(|error| {
            log_system_error(
                "delete_room_type",
                error.to_string(),
                json!({ "step": "count_rooms_by_type", "room_type_id": room_type_id, "room_type_name": &type_name }),
            )
        })?;
    if in_use > 0 {
        return Err(CommandError::user(
            codes::ROOM_TYPE_IN_USE,
            "Không thể xóa loại phòng đang được sử dụng",
        ));
    }

    room_writes::delete_room_type(pool, room_type_id)
        .await
        .map_err(|error| {
            log_system_error(
                "delete_room_type",
                error.to_string(),
                json!({ "step": "delete_room_type", "room_type_id": room_type_id }),
            )
        })
}

fn room_already_exists() -> CommandError {
    CommandError::user(codes::ROOM_ALREADY_EXISTS, "Phòng đã tồn tại")
}

fn room_type_already_exists() -> CommandError {
    CommandError::user(codes::ROOM_TYPE_ALREADY_EXISTS, "Loại phòng đã tồn tại")
}

#[cfg(test)]
mod tests {
    use super::{create_room, create_room_type, delete_room, delete_room_type, update_room};
    use crate::app_error::codes;
    use crate::models::{CreateRoomRequest, CreateRoomTypeRequest, UpdateRoomRequest};
    use sqlx::{sqlite::SqlitePoolOptions, Pool, Row, Sqlite};

    async fn test_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite pool");

        sqlx::query(
            "CREATE TABLE rooms (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                type TEXT NOT NULL,
                floor INTEGER NOT NULL,
                has_balcony INTEGER NOT NULL,
                base_price INTEGER NOT NULL,
                max_guests INTEGER NOT NULL,
                extra_person_fee INTEGER NOT NULL,
                status TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("create rooms table");

        sqlx::query(
            "CREATE TABLE bookings (
                id TEXT PRIMARY KEY,
                room_id TEXT NOT NULL,
                status TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("create bookings table");

        sqlx::query(
            "CREATE TABLE room_types (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                created_at TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .expect("create room_types table");

        pool
    }

    async fn seed_room(pool: &Pool<Sqlite>, room_id: &str, room_type: &str, status: &str) {
        sqlx::query(
            "INSERT INTO rooms (
                id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(room_id)
        .bind("Room 101")
        .bind(room_type)
        .bind(1)
        .bind(1)
        .bind(300_000)
        .bind(2)
        .bind(100_000)
        .bind(status)
        .execute(pool)
        .await
        .expect("seed room");
    }

    async fn seed_room_type(pool: &Pool<Sqlite>, id: &str, name: &str) {
        sqlx::query("INSERT INTO room_types (id, name, created_at) VALUES (?, ?, ?)")
            .bind(id)
            .bind(name)
            .bind("2026-04-22T00:00:00Z")
            .execute(pool)
            .await
            .expect("seed room type");
    }

    #[tokio::test]
    async fn update_room_leaves_unspecified_fields_unchanged() {
        let pool = test_pool().await;
        seed_room(&pool, "R101", "standard", "vacant").await;

        let updated = update_room(
            &pool,
            UpdateRoomRequest {
                room_id: "R101".to_string(),
                name: Some("Renamed Room".to_string()),
                room_type: None,
                floor: None,
                has_balcony: None,
                base_price: None,
                max_guests: None,
                extra_person_fee: None,
            },
        )
        .await
        .expect("update room name");

        assert_eq!(updated.name, "Renamed Room");
        assert_eq!(updated.room_type, "standard");
        assert_eq!(updated.floor, 1);
        assert!(updated.has_balcony);
        assert_eq!(updated.base_price, 300_000);
        assert_eq!(updated.max_guests, 2);
        assert_eq!(updated.extra_person_fee, 100_000);
    }

    #[tokio::test]
    async fn update_room_updates_multiple_fields_in_one_call() {
        let pool = test_pool().await;
        seed_room(&pool, "R102", "standard", "vacant").await;

        let updated = update_room(
            &pool,
            UpdateRoomRequest {
                room_id: "R102".to_string(),
                name: None,
                room_type: Some("suite".to_string()),
                floor: Some(5),
                has_balcony: Some(false),
                base_price: Some(450_000),
                max_guests: Some(4),
                extra_person_fee: Some(150_000),
            },
        )
        .await
        .expect("update multiple fields");

        assert_eq!(updated.room_type, "suite");
        assert_eq!(updated.floor, 5);
        assert!(!updated.has_balcony);
        assert_eq!(updated.base_price, 450_000);
        assert_eq!(updated.max_guests, 4);
        assert_eq!(updated.extra_person_fee, 150_000);

        let row = sqlx::query(
            "SELECT type, floor, has_balcony, base_price, max_guests, extra_person_fee
             FROM rooms
             WHERE id = ?",
        )
        .bind("R102")
        .fetch_one(&pool)
        .await
        .expect("fetch updated room");

        assert_eq!(row.get::<String, _>("type"), "suite");
        assert_eq!(row.get::<i32, _>("floor"), 5);
        assert_eq!(row.get::<i32, _>("has_balcony"), 0);
        assert_eq!(row.get::<i64, _>("base_price"), 450_000);
        assert_eq!(row.get::<i32, _>("max_guests"), 4);
        assert_eq!(row.get::<i64, _>("extra_person_fee"), 150_000);
    }

    #[tokio::test]
    async fn update_room_returns_room_not_found_error_for_missing_room() {
        let pool = test_pool().await;

        let error = update_room(
            &pool,
            UpdateRoomRequest {
                room_id: "missing-room".to_string(),
                name: Some("Ghost".to_string()),
                room_type: None,
                floor: None,
                has_balcony: None,
                base_price: None,
                max_guests: None,
                extra_person_fee: None,
            },
        )
        .await
        .expect_err("missing room must return an error");

        assert_eq!(error.code, codes::ROOM_NOT_FOUND);
        assert_eq!(error.message, "Phòng không tồn tại");
    }

    #[tokio::test]
    async fn update_room_rejects_negative_money_fields() {
        let pool = test_pool().await;
        seed_room(&pool, "R103", "standard", "vacant").await;

        let error = update_room(
            &pool,
            UpdateRoomRequest {
                room_id: "R103".to_string(),
                name: None,
                room_type: None,
                floor: None,
                has_balcony: None,
                base_price: Some(-1),
                max_guests: None,
                extra_person_fee: None,
            },
        )
        .await
        .expect_err("negative base_price must fail");

        assert_eq!(error.code, codes::VALIDATION_INVALID_INPUT);
        assert!(error.message.contains("base_price"));

        let error = update_room(
            &pool,
            UpdateRoomRequest {
                room_id: "R103".to_string(),
                name: None,
                room_type: None,
                floor: None,
                has_balcony: None,
                base_price: None,
                max_guests: None,
                extra_person_fee: Some(-1),
            },
        )
        .await
        .expect_err("negative extra_person_fee must fail");

        assert_eq!(error.code, codes::VALIDATION_INVALID_INPUT);
        assert!(error.message.contains("extra_person_fee"));
    }

    #[tokio::test]
    async fn create_room_returns_duplicate_room_error_for_taken_id() {
        let pool = test_pool().await;
        seed_room(&pool, "R201", "standard", "vacant").await;

        let error = create_room(
            &pool,
            CreateRoomRequest {
                id: "R201".to_string(),
                name: "Suite 201".to_string(),
                room_type: "standard".to_string(),
                floor: 2,
                has_balcony: false,
                base_price: 500_000,
                max_guests: 2,
                extra_person_fee: 150_000,
            },
        )
        .await
        .expect_err("duplicate room id must fail");

        assert_eq!(error.code, codes::ROOM_ALREADY_EXISTS);
        assert_eq!(error.message, "Phòng đã tồn tại");
    }

    #[tokio::test]
    async fn create_room_stores_the_room_as_vacant() {
        let pool = test_pool().await;

        let created = create_room(
            &pool,
            CreateRoomRequest {
                id: "R204".to_string(),
                name: "Room 204".to_string(),
                room_type: "standard".to_string(),
                floor: 2,
                has_balcony: true,
                base_price: 500_000,
                max_guests: 3,
                extra_person_fee: 150_000,
            },
        )
        .await
        .expect("create room");

        assert_eq!(created.status, "vacant");

        let row = sqlx::query("SELECT status, has_balcony, base_price FROM rooms WHERE id = ?")
            .bind("R204")
            .fetch_one(&pool)
            .await
            .expect("fetch created room");

        assert_eq!(row.get::<String, _>("status"), "vacant");
        assert_eq!(row.get::<i32, _>("has_balcony"), 1);
        assert_eq!(row.get::<i64, _>("base_price"), 500_000);
    }

    #[tokio::test]
    async fn create_room_rejects_negative_money_fields() {
        let pool = test_pool().await;

        let error = create_room(
            &pool,
            CreateRoomRequest {
                id: "R202".to_string(),
                name: "Room 202".to_string(),
                room_type: "standard".to_string(),
                floor: 2,
                has_balcony: false,
                base_price: -1,
                max_guests: 2,
                extra_person_fee: 150_000,
            },
        )
        .await
        .expect_err("negative base_price must fail");

        assert_eq!(error.code, codes::VALIDATION_INVALID_INPUT);
        assert!(error.message.contains("base_price"));

        let error = create_room(
            &pool,
            CreateRoomRequest {
                id: "R203".to_string(),
                name: "Room 203".to_string(),
                room_type: "standard".to_string(),
                floor: 2,
                has_balcony: false,
                base_price: 500_000,
                max_guests: 2,
                extra_person_fee: -1,
            },
        )
        .await
        .expect_err("negative extra_person_fee must fail");

        assert_eq!(error.code, codes::VALIDATION_INVALID_INPUT);
        assert!(error.message.contains("extra_person_fee"));
    }

    #[tokio::test]
    async fn delete_room_returns_occupied_error_for_occupied_room() {
        let pool = test_pool().await;
        seed_room(&pool, "R301", "standard", "occupied").await;

        let error = delete_room(&pool, "R301")
            .await
            .expect_err("occupied room must fail");

        assert_eq!(error.code, codes::ROOM_DELETE_OCCUPIED);
        assert_eq!(error.message, "Không thể xóa phòng đang có khách");
    }

    #[tokio::test]
    async fn delete_room_returns_active_booking_error_for_room_with_booking() {
        let pool = test_pool().await;
        seed_room(&pool, "R302", "standard", "vacant").await;
        sqlx::query("INSERT INTO bookings (id, room_id, status) VALUES (?, ?, ?)")
            .bind("B302")
            .bind("R302")
            .bind("active")
            .execute(&pool)
            .await
            .expect("seed active booking");

        let error = delete_room(&pool, "R302")
            .await
            .expect_err("active booking must fail");

        assert_eq!(error.code, codes::ROOM_DELETE_ACTIVE_BOOKING);
        assert_eq!(
            error.message,
            "Không thể xóa phòng có booking đang hoạt động"
        );
    }

    #[tokio::test]
    async fn delete_room_removes_a_free_room() {
        let pool = test_pool().await;
        seed_room(&pool, "R303", "standard", "vacant").await;

        delete_room(&pool, "R303").await.expect("delete free room");

        let remaining: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM rooms WHERE id = ?")
            .bind("R303")
            .fetch_one(&pool)
            .await
            .expect("count rooms");
        assert_eq!(remaining.0, 0);
    }

    #[tokio::test]
    async fn delete_room_returns_not_found_error_for_missing_room() {
        let pool = test_pool().await;

        let error = delete_room(&pool, "missing-room")
            .await
            .expect_err("missing room must fail");

        assert_eq!(error.code, codes::ROOM_NOT_FOUND);
        assert_eq!(error.message, "Phòng không tồn tại");
    }

    #[tokio::test]
    async fn create_room_type_returns_duplicate_error_for_taken_name() {
        let pool = test_pool().await;
        seed_room_type(&pool, "standard", "Standard").await;

        let error = create_room_type(
            &pool,
            CreateRoomTypeRequest {
                name: "Standard".to_string(),
            },
        )
        .await
        .expect_err("duplicate room type must fail");

        assert_eq!(error.code, codes::ROOM_TYPE_ALREADY_EXISTS);
        assert_eq!(error.message, "Loại phòng đã tồn tại");
    }

    #[tokio::test]
    async fn create_room_type_slugifies_the_name_into_the_id() {
        let pool = test_pool().await;

        let created = create_room_type(
            &pool,
            CreateRoomTypeRequest {
                name: "Deluxe Double".to_string(),
            },
        )
        .await
        .expect("create room type");

        assert_eq!(created.id, "deluxe_double");
        assert_eq!(created.name, "Deluxe Double");
    }

    #[tokio::test]
    async fn delete_room_type_returns_not_found_error_for_missing_type() {
        let pool = test_pool().await;

        let error = delete_room_type(&pool, "missing-type")
            .await
            .expect_err("missing room type must fail");

        assert_eq!(error.code, codes::ROOM_TYPE_NOT_FOUND);
        assert_eq!(error.message, "Loại phòng không tồn tại");
    }

    #[tokio::test]
    async fn delete_room_type_returns_in_use_error_when_rooms_reference_it() {
        let pool = test_pool().await;
        seed_room_type(&pool, "standard", "Standard").await;
        seed_room(&pool, "R401", "Standard", "vacant").await;

        let error = delete_room_type(&pool, "standard")
            .await
            .expect_err("room type in use must fail");

        assert_eq!(error.code, codes::ROOM_TYPE_IN_USE);
        assert_eq!(error.message, "Không thể xóa loại phòng đang được sử dụng");
    }

    #[tokio::test]
    async fn delete_room_type_removes_an_unused_type() {
        let pool = test_pool().await;
        seed_room_type(&pool, "standard", "Standard").await;

        delete_room_type(&pool, "standard")
            .await
            .expect("delete unused room type");

        let remaining: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM room_types")
            .fetch_one(&pool)
            .await
            .expect("count room types");
        assert_eq!(remaining.0, 0);
    }
}
