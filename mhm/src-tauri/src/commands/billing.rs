use super::{emit_db_update, get_user_id, AppState};
use crate::{
    app_error::{codes, CommandError, CommandResult},
    command_idempotency::WriteCommandContext,
    money::MoneyVnd,
    queries::booking::billing_queries,
    services::booking::billing_service,
};
use tauri::State;

// ═══════════════════════════════════════════════
// Phase 3: Folio / Billing Commands
// ═══════════════════════════════════════════════

fn require_add_folio_line_actor_id(user_id: Option<String>) -> CommandResult<String> {
    user_id.ok_or_else(|| CommandError::user(codes::AUTH_NOT_AUTHENTICATED, "Chưa đăng nhập"))
}

fn add_folio_line_write_command_context(
    idempotency_key: String,
    actor_id: String,
) -> CommandResult<WriteCommandContext> {
    let mut context = WriteCommandContext::for_scoped_command(
        uuid::Uuid::new_v4().to_string(),
        idempotency_key,
        "add_folio_line",
    )?;
    context.actor_id = Some(actor_id);
    Ok(context)
}

#[tauri::command]
pub async fn add_folio_line(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    booking_id: String,
    category: String,
    description: String,
    amount: MoneyVnd,
    idempotency_key: String,
) -> CommandResult<crate::models::FolioLine> {
    let actor_id = require_add_folio_line_actor_id(get_user_id(&state))?;
    let write_command_context =
        add_folio_line_write_command_context(idempotency_key, actor_id.clone())?;
    let result = billing_service::add_folio_line_idempotent(
        &state.db,
        &write_command_context,
        &booking_id,
        &category,
        &description,
        amount,
        Some(actor_id.as_str()),
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
pub async fn record_payment(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    booking_id: String,
    amount: MoneyVnd,
    note: String,
    idempotency_key: String,
) -> CommandResult<crate::models::RecordPaymentResponse> {
    let actor_id = get_user_id(&state)
        .ok_or_else(|| CommandError::user(codes::AUTH_NOT_AUTHENTICATED, "Chưa đăng nhập"))?;
    let mut write_command_context = WriteCommandContext::for_scoped_command(
        uuid::Uuid::new_v4().to_string(),
        idempotency_key,
        "record_payment",
    )?;
    write_command_context.actor_id = Some(actor_id);
    let result = billing_service::record_payment_idempotent(
        &state.db,
        &write_command_context,
        &booking_id,
        amount,
        &note,
    )
    .await?;
    let response: crate::models::RecordPaymentResponse = serde_json::from_value(result.response)
        .map_err(|error| {
            CommandError::system(
                codes::SYSTEM_INTERNAL_ERROR,
                format!("Invalid record_payment idempotent response: {error}"),
            )
            .with_request_id(write_command_context.request_id.clone())
        })?;

    emit_db_update(&app, "folio");
    Ok(response)
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

#[cfg(test)]
mod tests {
    use super::{add_folio_line_write_command_context, require_add_folio_line_actor_id};
    use crate::app_error::codes;

    #[test]
    fn add_folio_line_write_context_sets_actor_id() {
        let ctx = add_folio_line_write_command_context(
            "idem-folio-command".to_string(),
            "user-1".to_string(),
        )
        .expect("context builds");

        assert_eq!(ctx.command_name, "add_folio_line");
        assert_eq!(ctx.idempotency_key, "idem-folio-command");
        assert_eq!(ctx.actor_id.as_deref(), Some("user-1"));
    }

    #[test]
    fn add_folio_line_requires_authenticated_actor() {
        let error = require_add_folio_line_actor_id(None).expect_err("missing user rejects");

        assert_eq!(error.code, codes::AUTH_NOT_AUTHENTICATED);
    }
}
