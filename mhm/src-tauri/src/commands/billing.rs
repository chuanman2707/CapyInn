use super::{emit_db_update, get_user_id, AppState};
use crate::{
    app_error::{codes, CommandError, CommandResult},
    command_idempotency::WriteCommandContext,
    queries::booking::billing_queries,
    services::booking::billing_service,
};
use tauri::State;

// ═══════════════════════════════════════════════
// Phase 3: Folio / Billing Commands
// ═══════════════════════════════════════════════

#[tauri::command]
pub async fn add_folio_line(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    booking_id: String,
    category: String,
    description: String,
    amount: f64,
    idempotency_key: String,
) -> CommandResult<crate::models::FolioLine> {
    let user_id = get_user_id(&state);
    let write_command_context = WriteCommandContext::for_scoped_command(
        uuid::Uuid::new_v4().to_string(),
        idempotency_key,
        "add_folio_line",
    )?;
    let result = billing_service::add_folio_line_idempotent(
        &state.db,
        &write_command_context,
        &booking_id,
        &category,
        &description,
        amount,
        user_id.as_deref(),
    )
    .await?;
    let line: crate::models::FolioLine =
        serde_json::from_value(result.response).map_err(|error| {
            CommandError::system(
                codes::SYSTEM_INTERNAL_ERROR,
                format!("Invalid add_folio_line idempotent response: {error}"),
            )
            .with_request_id(write_command_context.request_id.clone())
        })?;

    emit_db_update(&app, "folio");

    Ok(line)
}

#[tauri::command]
pub async fn get_folio_lines(
    state: State<'_, AppState>,
    booking_id: String,
) -> Result<Vec<crate::models::FolioLine>, String> {
    billing_queries::list_folio_lines(&state.db, &booking_id)
        .await
        .map_err(|e| e.to_string())
}
