use sqlx::{Pool, Sqlite};
use tauri::State;
use super::AppState;


// ─── Settings Commands ───

#[tauri::command]
pub async fn save_settings(state: State<'_, AppState>, key: String, value: String) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value"
    )
    .bind(&key).bind(&value)
    .execute(&state.db).await.map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn do_get_settings(pool: &Pool<Sqlite>, key: &str) -> Result<Option<String>, String> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT value FROM settings WHERE key = ?"
    )
    .bind(key)
    .fetch_optional(pool).await.map_err(|e| e.to_string())?;
    Ok(row.map(|r| r.0))
}

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>, key: String) -> Result<Option<String>, String> {
    do_get_settings(&state.db, &key).await
}
