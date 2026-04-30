use super::{emit_db_update, get_money_vnd, require_admin, AppState};
use crate::app_error::{codes, log_system_error, CommandError, CommandResult};
use crate::app_identity;
use crate::models::*;
use crate::money::validate_non_negative_money_vnd;
use serde_json::json;
use sqlx::{Pool, Row, Sqlite};
use tauri::State;

// ─── A5: Update Room ───

#[tauri::command]
pub async fn update_room(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: UpdateRoomRequest,
) -> CommandResult<Room> {
    require_admin(&state)?;

    let r = do_update_room(&state.db, req).await?;
    emit_db_update(&app, "rooms");

    Ok(r)
}

async fn do_update_room(pool: &Pool<Sqlite>, req: UpdateRoomRequest) -> CommandResult<Room> {
    let UpdateRoomRequest {
        room_id,
        name,
        room_type,
        floor,
        has_balcony,
        base_price,
        max_guests,
        extra_person_fee,
    } = req;
    let base_price = base_price
        .map(|value| validate_non_negative_money_vnd(value, "base_price"))
        .transpose()?;
    let extra_person_fee = extra_person_fee
        .map(|value| validate_non_negative_money_vnd(value, "extra_person_fee"))
        .transpose()?;

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
    .bind(name)
    .bind(room_type)
    .bind(floor)
    .bind(has_balcony.map(|value| value as i32))
    .bind(base_price)
    .bind(max_guests)
    .bind(extra_person_fee)
    .bind(&room_id)
    .execute(pool)
    .await
    .map_err(|error| {
        log_system_error(
            "update_room",
            error.to_string(),
            json!({ "step": "update_room", "room_id": &room_id }),
        )
    })?;

    if result.rows_affected() == 0 {
        return Err(CommandError::user(
            codes::ROOM_NOT_FOUND,
            "Phòng không tồn tại",
        ));
    }

    let r = sqlx::query("SELECT id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status FROM rooms WHERE id = ?")
        .bind(&room_id)
        .fetch_one(pool)
        .await
        .map_err(|error| {
            log_system_error(
                "update_room",
                error.to_string(),
                json!({ "step": "fetch_updated_room", "room_id": &room_id }),
            )
        })?;

    Ok(Room {
        id: r.get("id"),
        name: r.get("name"),
        room_type: r.get("type"),
        floor: r.get("floor"),
        has_balcony: r.get::<i32, _>("has_balcony") == 1,
        base_price: get_money_vnd(&r, "base_price"),
        max_guests: r.try_get::<i32, _>("max_guests").unwrap_or(2),
        extra_person_fee: get_money_vnd(&r, "extra_person_fee"),
        status: r.get("status"),
    })
}

// ─── A5b: Create Room ───

#[tauri::command]
pub async fn create_room(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: CreateRoomRequest,
) -> CommandResult<Room> {
    require_admin(&state)?;

    let room = do_create_room(&state.db, req).await?;

    emit_db_update(&app, "rooms");
    Ok(room)
}

async fn do_create_room(pool: &Pool<Sqlite>, req: CreateRoomRequest) -> CommandResult<Room> {
    let CreateRoomRequest {
        id,
        name,
        room_type,
        floor,
        has_balcony,
        base_price,
        max_guests,
        extra_person_fee,
    } = req;
    let base_price = validate_non_negative_money_vnd(base_price, "base_price")?;
    let extra_person_fee = validate_non_negative_money_vnd(extra_person_fee, "extra_person_fee")?;

    let existing: Option<(String,)> = sqlx::query_as("SELECT id FROM rooms WHERE id = ?")
        .bind(&id)
        .fetch_optional(pool)
        .await
        .map_err(|error| {
            log_system_error(
                "create_room",
                error.to_string(),
                json!({ "step": "check_existing_room", "room_id": &id }),
            )
        })?;
    if existing.is_some() {
        return Err(CommandError::user(
            codes::ROOM_ALREADY_EXISTS,
            "Phòng đã tồn tại",
        ));
    }

    sqlx::query(
        "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'vacant')",
    )
    .bind(&id)
    .bind(&name)
    .bind(&room_type)
    .bind(floor)
    .bind(has_balcony as i32)
    .bind(base_price)
    .bind(max_guests)
    .bind(extra_person_fee)
    .execute(pool)
    .await
    .map_err(|error| {
        if is_unique_constraint_error(&error) {
            CommandError::user(codes::ROOM_ALREADY_EXISTS, "Phòng đã tồn tại")
        } else {
            log_system_error(
                "create_room",
                error.to_string(),
                json!({ "step": "insert_room", "room_id": &id }),
            )
        }
    })?;

    Ok(Room {
        id,
        name,
        room_type,
        floor,
        has_balcony,
        base_price,
        max_guests,
        extra_person_fee,
        status: "vacant".to_string(),
    })
}

// ─── A5c: Delete Room ───

#[tauri::command]
pub async fn delete_room(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    room_id: String,
) -> CommandResult<()> {
    require_admin(&state)?;

    do_delete_room(&state.db, room_id.clone()).await?;

    emit_db_update(&app, "rooms");
    Ok(())
}

async fn do_delete_room(pool: &Pool<Sqlite>, room_id: String) -> CommandResult<()> {
    let room_row = sqlx::query("SELECT status FROM rooms WHERE id = ?")
        .bind(&room_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| {
            log_system_error(
                "delete_room",
                error.to_string(),
                json!({ "step": "fetch_room", "room_id": &room_id }),
            )
        })?;
    let room_row = match room_row {
        Some(row) => row,
        None => {
            return Err(CommandError::user(
                codes::ROOM_NOT_FOUND,
                "Phòng không tồn tại",
            ))
        }
    };
    let status: String = room_row.get("status");

    if status == "occupied" {
        return Err(CommandError::user(
            codes::ROOM_DELETE_OCCUPIED,
            "Không thể xóa phòng đang có khách",
        ));
    }

    let active: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM bookings WHERE room_id = ? AND status = 'active'")
            .bind(&room_id)
            .fetch_one(pool)
            .await
            .map_err(|error| {
                log_system_error(
                    "delete_room",
                    error.to_string(),
                    json!({ "step": "count_active_bookings", "room_id": &room_id }),
                )
            })?;
    if active.0 > 0 {
        return Err(CommandError::user(
            codes::ROOM_DELETE_ACTIVE_BOOKING,
            "Không thể xóa phòng có booking đang hoạt động",
        ));
    }

    sqlx::query("DELETE FROM rooms WHERE id = ?")
        .bind(&room_id)
        .execute(pool)
        .await
        .map_err(|error| {
            log_system_error(
                "delete_room",
                error.to_string(),
                json!({ "step": "delete_room", "room_id": &room_id }),
            )
        })?;

    Ok(())
}

// ─── Room Types Management ───

pub async fn do_get_room_types(pool: &Pool<Sqlite>) -> Result<Vec<RoomType>, String> {
    let rows = sqlx::query("SELECT id, name, created_at FROM room_types ORDER BY name")
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| RoomType {
            id: r.get("id"),
            name: r.get("name"),
            created_at: r.get("created_at"),
        })
        .collect())
}

#[tauri::command]
pub async fn get_room_types(state: State<'_, AppState>) -> Result<Vec<RoomType>, String> {
    do_get_room_types(&state.db).await
}

#[tauri::command]
pub async fn create_room_type(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: CreateRoomTypeRequest,
) -> CommandResult<RoomType> {
    require_admin(&state)?;

    let room_type = do_create_room_type(&state.db, req).await?;

    emit_db_update(&app, "room_types");
    Ok(room_type)
}

async fn do_create_room_type(
    pool: &Pool<Sqlite>,
    req: CreateRoomTypeRequest,
) -> CommandResult<RoomType> {
    let name = req.name;
    let id = name.to_lowercase().replace(' ', "_");
    let now = chrono::Local::now().to_rfc3339();

    let existing: Option<(String,)> = sqlx::query_as(
        "SELECT id FROM room_types WHERE id = ? OR name = ?",
    )
    .bind(&id)
    .bind(&name)
    .fetch_optional(pool)
    .await
    .map_err(|error| {
        log_system_error(
            "create_room_type",
            error.to_string(),
            json!({ "step": "check_existing_room_type", "room_type_id": &id, "room_type_name": &name }),
        )
    })?;
    if existing.is_some() {
        return Err(CommandError::user(
            codes::ROOM_TYPE_ALREADY_EXISTS,
            "Loại phòng đã tồn tại",
        ));
    }

    sqlx::query("INSERT INTO room_types (id, name, created_at) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(&name)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(|error| {
            if is_unique_constraint_error(&error) {
                CommandError::user(
                    codes::ROOM_TYPE_ALREADY_EXISTS,
                    "Loại phòng đã tồn tại",
                )
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

#[tauri::command]
pub async fn delete_room_type(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    room_type_id: String,
) -> CommandResult<()> {
    require_admin(&state)?;

    do_delete_room_type(&state.db, room_type_id.clone()).await?;

    emit_db_update(&app, "room_types");
    Ok(())
}

async fn do_delete_room_type(pool: &Pool<Sqlite>, room_type_id: String) -> CommandResult<()> {
    let rt_row = sqlx::query("SELECT name FROM room_types WHERE id = ?")
        .bind(&room_type_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| {
            log_system_error(
                "delete_room_type",
                error.to_string(),
                json!({ "step": "fetch_room_type", "room_type_id": &room_type_id }),
            )
        })?;
    let rt_row = match rt_row {
        Some(row) => row,
        None => {
            return Err(CommandError::user(
                codes::ROOM_TYPE_NOT_FOUND,
                "Loại phòng không tồn tại",
            ))
        }
    };
    let type_name: String = rt_row.get("name");

    let in_use: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM rooms WHERE type = ?")
        .bind(&type_name)
        .fetch_one(pool)
        .await
        .map_err(|error| {
            log_system_error(
                "delete_room_type",
                error.to_string(),
                json!({ "step": "count_rooms_by_type", "room_type_id": &room_type_id, "room_type_name": &type_name }),
            )
        })?;
    if in_use.0 > 0 {
        return Err(CommandError::user(
            codes::ROOM_TYPE_IN_USE,
            "Không thể xóa loại phòng đang được sử dụng",
        ));
    }

    sqlx::query("DELETE FROM room_types WHERE id = ?")
        .bind(&room_type_id)
        .execute(pool)
        .await
        .map_err(|error| {
            log_system_error(
                "delete_room_type",
                error.to_string(),
                json!({ "step": "delete_room_type", "room_type_id": &room_type_id }),
            )
        })?;

    Ok(())
}

fn is_unique_constraint_error(error: &sqlx::Error) -> bool {
    matches!(
        error,
        sqlx::Error::Database(database_error)
            if database_error.message().contains("UNIQUE constraint failed")
                || database_error.message().contains("UNIQUE")
    )
}

// ─── A5: Export CSV ───

#[tauri::command]
pub async fn export_csv(state: State<'_, AppState>) -> Result<String, String> {
    require_admin(&state)?;

    let export_dir = app_identity::exports_dir_opt().ok_or("Cannot find home directory")?;

    std::fs::create_dir_all(&export_dir).map_err(|e| e.to_string())?;

    let now = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();

    // Export bookings
    let bookings = sqlx::query("SELECT b.id, b.room_id, g.full_name, b.check_in_at, b.expected_checkout, b.nights, b.total_price, b.paid_amount, b.status, b.source FROM bookings b JOIN guests g ON g.id = b.primary_guest_id ORDER BY b.check_in_at DESC")
        .fetch_all(&state.db).await.map_err(|e| e.to_string())?;

    let bookings_path = export_dir.join(format!("bookings_{}.csv", now));
    let mut csv = String::from("ID,Room,Guest,Check-in,Checkout,Nights,Total,Paid,Status,Source\n");
    for r in &bookings {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{}\n",
            r.get::<String, _>("id"),
            r.get::<String, _>("room_id"),
            r.get::<String, _>("full_name"),
            r.get::<String, _>("check_in_at"),
            r.get::<String, _>("expected_checkout"),
            r.get::<i32, _>("nights"),
            get_money_vnd(r, "total_price"),
            get_money_vnd(r, "paid_amount"),
            r.get::<String, _>("status"),
            r.get::<Option<String>, _>("source").unwrap_or_default(),
        ));
    }
    std::fs::write(&bookings_path, csv).map_err(|e| e.to_string())?;

    // Export guests
    let guests = sqlx::query("SELECT id, full_name, doc_number, nationality, created_at FROM guests ORDER BY created_at DESC")
        .fetch_all(&state.db).await.map_err(|e| e.to_string())?;

    let guests_path = export_dir.join(format!("guests_{}.csv", now));
    let mut csv2 = String::from("ID,Name,DocNumber,Nationality,CreatedAt\n");
    for r in &guests {
        csv2.push_str(&format!(
            "{},{},{},{},{}\n",
            r.get::<String, _>("id"),
            r.get::<String, _>("full_name"),
            r.get::<String, _>("doc_number"),
            r.get::<Option<String>, _>("nationality")
                .unwrap_or_default(),
            r.get::<String, _>("created_at"),
        ));
    }
    std::fs::write(&guests_path, csv2).map_err(|e| e.to_string())?;

    Ok(export_dir.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        do_create_room, do_create_room_type, do_delete_room, do_delete_room_type, do_update_room,
    };
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
        .bind(300_000.0)
        .bind(2)
        .bind(100_000.0)
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

        let updated = do_update_room(
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

        let updated = do_update_room(
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

        let error = do_update_room(
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

        let error = do_update_room(
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

        let error = do_update_room(
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

        let error = do_create_room(
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
    async fn create_room_rejects_negative_money_fields() {
        let pool = test_pool().await;

        let error = do_create_room(
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

        let error = do_create_room(
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

        let error = do_delete_room(&pool, "R301".to_string())
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

        let error = do_delete_room(&pool, "R302".to_string())
            .await
            .expect_err("active booking must fail");

        assert_eq!(error.code, codes::ROOM_DELETE_ACTIVE_BOOKING);
        assert_eq!(
            error.message,
            "Không thể xóa phòng có booking đang hoạt động"
        );
    }

    #[tokio::test]
    async fn create_room_type_returns_duplicate_error_for_taken_name() {
        let pool = test_pool().await;
        seed_room_type(&pool, "standard", "Standard").await;

        let error = do_create_room_type(
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
    async fn delete_room_type_returns_not_found_error_for_missing_type() {
        let pool = test_pool().await;

        let error = do_delete_room_type(&pool, "missing-type".to_string())
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

        let error = do_delete_room_type(&pool, "standard".to_string())
            .await
            .expect_err("room type in use must fail");

        assert_eq!(error.code, codes::ROOM_TYPE_IN_USE);
        assert_eq!(error.message, "Không thể xóa loại phòng đang được sử dụng");
    }
}
