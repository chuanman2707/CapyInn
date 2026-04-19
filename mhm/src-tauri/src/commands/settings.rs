use super::{require_admin, AppState};
use crate::services::settings_store;
use sqlx::{Pool, Sqlite};
use tauri::State;

// ─── Settings Commands ───

#[tauri::command]
pub async fn save_settings(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    key: String,
    value: String,
) -> Result<(), String> {
    require_admin(&state)?;
    settings_store::save_setting(&state.db, &key, &value).await?;

    if let Err(error) =
        crate::backup::request_backup(&app, crate::backup::BackupReason::Settings).await
    {
        crate::backup::log_backup_request_error("save_settings", &error);
    }

    Ok(())
}

pub async fn do_get_settings(pool: &Pool<Sqlite>, key: &str) -> Result<Option<String>, String> {
    settings_store::get_setting(pool, key).await
}

#[tauri::command]
pub async fn get_settings(
    state: State<'_, AppState>,
    key: String,
) -> Result<Option<String>, String> {
    settings_store::get_setting(&state.db, &key).await
}
