use super::{emit_db_update, get_f64, require_admin, AppState};
use crate::app_identity;
use crate::models::*;
use sqlx::{Pool, Row, Sqlite};
use tauri::State;

// ─── A5: Update Room ───

#[tauri::command]
pub async fn update_room(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: UpdateRoomRequest,
) -> Result<Room, String> {
    require_admin(&state)?;

    if let Some(ref name) = req.name {
        sqlx::query("UPDATE rooms SET name = ? WHERE id = ?")
            .bind(name)
            .bind(&req.room_id)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(ref room_type) = req.room_type {
        sqlx::query("UPDATE rooms SET type = ? WHERE id = ?")
            .bind(room_type)
            .bind(&req.room_id)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(floor) = req.floor {
        sqlx::query("UPDATE rooms SET floor = ? WHERE id = ?")
            .bind(floor)
            .bind(&req.room_id)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(has_balcony) = req.has_balcony {
        sqlx::query("UPDATE rooms SET has_balcony = ? WHERE id = ?")
            .bind(has_balcony as i32)
            .bind(&req.room_id)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(price) = req.base_price {
        sqlx::query("UPDATE rooms SET base_price = ? WHERE id = ?")
            .bind(price)
            .bind(&req.room_id)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(max_guests) = req.max_guests {
        sqlx::query("UPDATE rooms SET max_guests = ? WHERE id = ?")
            .bind(max_guests)
            .bind(&req.room_id)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    }
    if let Some(extra_fee) = req.extra_person_fee {
        sqlx::query("UPDATE rooms SET extra_person_fee = ? WHERE id = ?")
            .bind(extra_fee)
            .bind(&req.room_id)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    }

    let r = sqlx::query("SELECT id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status FROM rooms WHERE id = ?")
        .bind(&req.room_id)
        .fetch_one(&state.db).await.map_err(|e| e.to_string())?;

    emit_db_update(&app, "rooms");

    Ok(Room {
        id: r.get("id"),
        name: r.get("name"),
        room_type: r.get("type"),
        floor: r.get("floor"),
        has_balcony: r.get::<i32, _>("has_balcony") == 1,
        base_price: get_f64(&r, "base_price"),
        max_guests: r.try_get::<i32, _>("max_guests").unwrap_or(2),
        extra_person_fee: r.try_get::<f64, _>("extra_person_fee").unwrap_or(0.0),
        status: r.get("status"),
    })
}

// ─── A5b: Create Room ───

#[tauri::command]
pub async fn create_room(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: CreateRoomRequest,
) -> Result<Room, String> {
    require_admin(&state)?;

    // Check ID doesn't already exist
    let existing: Option<(String,)> = sqlx::query_as("SELECT id FROM rooms WHERE id = ?")
        .bind(&req.id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    if existing.is_some() {
        return Err(format!("Phòng '{}' đã tồn tại", req.id));
    }

    sqlx::query(
        "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'vacant')"
    )
    .bind(&req.id)
    .bind(&req.name)
    .bind(&req.room_type)
    .bind(req.floor)
    .bind(req.has_balcony as i32)
    .bind(req.base_price)
    .bind(req.max_guests)
    .bind(req.extra_person_fee)
    .execute(&state.db).await.map_err(|e| e.to_string())?;

    emit_db_update(&app, "rooms");

    Ok(Room {
        id: req.id,
        name: req.name,
        room_type: req.room_type,
        floor: req.floor,
        has_balcony: req.has_balcony,
        base_price: req.base_price,
        max_guests: req.max_guests,
        extra_person_fee: req.extra_person_fee,
        status: "vacant".to_string(),
    })
}

// ─── A5c: Delete Room ───

#[tauri::command]
pub async fn delete_room(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    room_id: String,
) -> Result<(), String> {
    require_admin(&state)?;

    // Check room exists
    let room_row = sqlx::query("SELECT status FROM rooms WHERE id = ?")
        .bind(&room_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    let room_row = room_row.ok_or(format!("Phòng '{}' không tồn tại", room_id))?;
    let status: String = room_row.get("status");

    if status == "occupied" {
        return Err("Không thể xóa phòng đang có khách".to_string());
    }

    // Check no active bookings
    let active: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM bookings WHERE room_id = ? AND status = 'active'")
            .bind(&room_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    if active.0 > 0 {
        return Err("Không thể xóa phòng có booking đang hoạt động".to_string());
    }

    sqlx::query("DELETE FROM rooms WHERE id = ?")
        .bind(&room_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    emit_db_update(&app, "rooms");
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
) -> Result<RoomType, String> {
    require_admin(&state)?;

    let id = req.name.to_lowercase().replace(' ', "_");
    let now = chrono::Local::now().to_rfc3339();

    sqlx::query("INSERT INTO room_types (id, name, created_at) VALUES (?, ?, ?)")
        .bind(&id)
        .bind(&req.name)
        .bind(&now)
        .execute(&state.db)
        .await
        .map_err(|e| {
            if e.to_string().contains("UNIQUE") {
                format!("Loại phòng '{}' đã tồn tại", req.name)
            } else {
                e.to_string()
            }
        })?;

    emit_db_update(&app, "room_types");

    Ok(RoomType {
        id,
        name: req.name,
        created_at: now,
    })
}

#[tauri::command]
pub async fn delete_room_type(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    room_type_id: String,
) -> Result<(), String> {
    require_admin(&state)?;

    // Check no rooms using this type
    let rt_row = sqlx::query("SELECT name FROM room_types WHERE id = ?")
        .bind(&room_type_id)
        .fetch_optional(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    let rt_row = rt_row.ok_or("Loại phòng không tồn tại".to_string())?;
    let type_name: String = rt_row.get("name");

    let in_use: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM rooms WHERE type = ?")
        .bind(&type_name)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;
    if in_use.0 > 0 {
        return Err(format!(
            "Không thể xóa: có {} phòng đang sử dụng loại '{}'",
            in_use.0, type_name
        ));
    }

    sqlx::query("DELETE FROM room_types WHERE id = ?")
        .bind(&room_type_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    emit_db_update(&app, "room_types");
    Ok(())
}

// ─── A5: Export CSV ───

#[tauri::command]
pub async fn export_csv(state: State<'_, AppState>) -> Result<String, String> {
    require_admin(&state)?;

    let export_dir = app_identity::exports_dir_opt().ok_or("Cannot find home directory")?;

    tokio::fs::create_dir_all(&export_dir)
        .await
        .map_err(|e| e.to_string())?;

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
            get_f64(r, "total_price"),
            get_f64(r, "paid_amount"),
            r.get::<String, _>("status"),
            r.get::<Option<String>, _>("source").unwrap_or_default(),
        ));
    }
    tokio::fs::write(&bookings_path, csv)
        .await
        .map_err(|e| e.to_string())?;

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
    tokio::fs::write(&guests_path, csv2)
        .await
        .map_err(|e| e.to_string())?;

    Ok(export_dir.to_string_lossy().to_string())
}
