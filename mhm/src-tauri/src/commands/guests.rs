use super::AppState;
use crate::models::*;
use crate::queries::guests::guest_queries;
use tauri::State;

// ─── A2: Get All Guests ───

#[tauri::command]
pub async fn get_all_guests(
    state: State<'_, AppState>,
    search: Option<String>,
) -> Result<Vec<GuestSummary>, String> {
    guest_queries::load_guest_summaries(&state.db, search.as_deref())
        .await
        .map_err(|e| e.to_string())
}

// ─── A2: Get Guest History ───

#[tauri::command]
pub async fn get_guest_history(
    state: State<'_, AppState>,
    guest_id: String,
) -> Result<GuestHistoryResponse, String> {
    let guest = guest_queries::load_guest(&state.db, &guest_id)
        .await
        .map_err(|e| e.to_string())?;
    let bookings = guest_queries::load_guest_bookings(&state.db, &guest_id)
        .await
        .map_err(|e| e.to_string())?;

    Ok(GuestHistoryResponse { guest, bookings })
}
