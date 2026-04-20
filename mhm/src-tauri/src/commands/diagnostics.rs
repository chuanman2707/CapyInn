use tauri::State;

use crate::diagnostics::{self, CrashBundle, JsCrashReportInput};

use super::AppState;

#[tauri::command]
pub async fn record_js_crash(
    _state: State<'_, AppState>,
    report: JsCrashReportInput,
) -> Result<(), String> {
    diagnostics::record_js_crash(report)
}

#[tauri::command]
pub async fn get_pending_crash_report(
    _state: State<'_, AppState>,
) -> Result<Option<CrashBundle>, String> {
    diagnostics::get_pending_crash_report()
}

#[tauri::command]
pub async fn mark_crash_report_submitted(
    _state: State<'_, AppState>,
    bundle_id: String,
) -> Result<(), String> {
    diagnostics::mark_crash_report_submitted(&bundle_id)
}

#[tauri::command]
pub async fn mark_crash_report_dismissed(
    _state: State<'_, AppState>,
    bundle_id: String,
) -> Result<(), String> {
    diagnostics::mark_crash_report_dismissed(&bundle_id)
}

#[tauri::command]
pub async fn mark_crash_report_send_failed(
    _state: State<'_, AppState>,
    bundle_id: String,
) -> Result<(), String> {
    diagnostics::mark_crash_report_send_failed(&bundle_id)
}

#[tauri::command]
pub async fn export_crash_report(
    _state: State<'_, AppState>,
    bundle_id: String,
) -> Result<String, String> {
    Ok(diagnostics::export_crash_report(&bundle_id)?
        .to_string_lossy()
        .to_string())
}
