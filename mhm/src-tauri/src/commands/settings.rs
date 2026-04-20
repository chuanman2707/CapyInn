use super::{require_admin, AppState};
use crate::services::settings_store;
use tauri::State;

// ─── Settings Commands ───

const SEND_CRASH_REPORTS_KEY: &str = "send_crash_reports";

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

#[tauri::command]
pub async fn get_settings(
    state: State<'_, AppState>,
    key: String,
) -> Result<Option<String>, String> {
    settings_store::get_setting(&state.db, &key).await
}

#[tauri::command]
pub async fn get_crash_reporting_preference(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(matches!(
        settings_store::get_setting(&state.db, SEND_CRASH_REPORTS_KEY)
            .await?
            .as_deref(),
        Some("true")
    ))
}

#[tauri::command]
pub async fn set_crash_reporting_preference(
    state: State<'_, AppState>,
    enabled: bool,
) -> Result<(), String> {
    settings_store::save_setting(
        &state.db,
        SEND_CRASH_REPORTS_KEY,
        if enabled { "true" } else { "false" },
    )
    .await
}
