use super::AppState;
use crate::models::*;
use crate::queries::booking::booking_list_queries;
use sqlx::{Pool, Sqlite};
use tauri::State;

// ─── A1: Get All Bookings (Reservations) ───

pub async fn do_get_all_bookings(
    pool: &Pool<Sqlite>,
    filter: Option<BookingFilter>,
) -> Result<Vec<BookingWithGuest>, String> {
    booking_list_queries::load_bookings_with_guest(pool, filter)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_all_bookings(
    state: State<'_, AppState>,
    filter: Option<BookingFilter>,
) -> Result<Vec<BookingWithGuest>, String> {
    do_get_all_bookings(&state.db, filter).await
}
