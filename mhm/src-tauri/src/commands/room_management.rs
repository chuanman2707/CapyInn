//! Room and room-type administration commands.
//!
//! These are boundary adapters: check the caller is an admin, hand the request
//! to `services::rooms::room_service`, and tell the UI what changed. The rules
//! and the SQL live behind the service.

use super::{emit_db_update, require_admin, AppState};
use crate::app_error::CommandResult;
use crate::app_identity;
use crate::models::*;
use crate::queries::export_queries;
use crate::services::rooms::room_service;
use sqlx::{Pool, Sqlite};
use tauri::State;

// ─── A5: Update Room ───

#[tauri::command]
pub async fn update_room(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: UpdateRoomRequest,
) -> CommandResult<Room> {
    require_admin(&state)?;

    let room = room_service::update_room(&state.db, req).await?;
    emit_db_update(&app, "rooms");

    Ok(room)
}

// ─── A5b: Create Room ───

#[tauri::command]
pub async fn create_room(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: CreateRoomRequest,
) -> CommandResult<Room> {
    require_admin(&state)?;

    let room = room_service::create_room(&state.db, req).await?;

    emit_db_update(&app, "rooms");
    Ok(room)
}

// ─── A5c: Delete Room ───

#[tauri::command]
pub async fn delete_room(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    room_id: String,
) -> CommandResult<()> {
    require_admin(&state)?;

    room_service::delete_room(&state.db, &room_id).await?;

    emit_db_update(&app, "rooms");
    Ok(())
}

// ─── Room Types Management ───

/// Re-exported through `commands` for the gateway tool surface, which reads
/// through the same entry points the UI uses.
pub async fn do_get_room_types(pool: &Pool<Sqlite>) -> Result<Vec<RoomType>, String> {
    room_service::list_room_types(pool).await
}

#[tauri::command]
pub async fn get_room_types(state: State<'_, AppState>) -> Result<Vec<RoomType>, String> {
    room_service::list_room_types(&state.db).await
}

#[tauri::command]
pub async fn create_room_type(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: CreateRoomTypeRequest,
) -> CommandResult<RoomType> {
    require_admin(&state)?;

    let room_type = room_service::create_room_type(&state.db, req).await?;

    emit_db_update(&app, "room_types");
    Ok(room_type)
}

#[tauri::command]
pub async fn delete_room_type(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    room_type_id: String,
) -> CommandResult<()> {
    require_admin(&state)?;

    room_service::delete_room_type(&state.db, &room_type_id).await?;

    emit_db_update(&app, "room_types");
    Ok(())
}

// ─── A5: Export CSV ───

#[tauri::command]
pub async fn export_csv(state: State<'_, AppState>) -> Result<String, String> {
    require_admin(&state)?;

    let export_dir = app_identity::exports_dir_opt().ok_or("Cannot find home directory")?;

    std::fs::create_dir_all(&export_dir).map_err(|e| e.to_string())?;

    let now = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();

    let bookings = export_queries::load_booking_export_rows(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    let bookings_path = export_dir.join(format!("bookings_{}.csv", now));
    let mut csv = String::from("ID,Room,Guest,Check-in,Checkout,Nights,Total,Paid,Status,Source\n");
    for row in &bookings {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{}\n",
            row.id,
            row.room_id,
            row.guest_name,
            row.check_in_at,
            row.expected_checkout,
            row.nights,
            row.total_price,
            row.paid_amount,
            row.status,
            row.source,
        ));
    }
    std::fs::write(&bookings_path, csv).map_err(|e| e.to_string())?;

    let guests = export_queries::load_guest_export_rows(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    let guests_path = export_dir.join(format!("guests_{}.csv", now));
    let mut csv2 = String::from("ID,Name,DocNumber,Nationality,CreatedAt\n");
    for row in &guests {
        csv2.push_str(&format!(
            "{},{},{},{},{}\n",
            row.id, row.full_name, row.doc_number, row.nationality, row.created_at,
        ));
    }
    std::fs::write(&guests_path, csv2).map_err(|e| e.to_string())?;

    Ok(export_dir.to_string_lossy().to_string())
}
