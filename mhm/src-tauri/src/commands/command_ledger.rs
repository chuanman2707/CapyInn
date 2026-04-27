use super::{require_admin, AppState};
use crate::command_ledger::{
    get_command_ledger_detail as get_command_ledger_detail_query,
    list_command_ledger as list_command_ledger_query,
    list_command_ledger_attention as list_command_ledger_attention_query,
    CommandLedgerDetail, CommandLedgerListItem, CommandLedgerListOptions,
};
use tauri::State;

#[tauri::command]
pub async fn list_command_ledger(
    state: State<'_, AppState>,
    options: Option<CommandLedgerListOptions>,
) -> Result<Vec<CommandLedgerListItem>, String> {
    require_admin(&state)?;
    list_command_ledger_query(&state.db, options.unwrap_or_default())
        .await
        .map_err(String::from)
}

#[tauri::command]
pub async fn list_command_ledger_attention(
    state: State<'_, AppState>,
) -> Result<Vec<CommandLedgerListItem>, String> {
    require_admin(&state)?;
    list_command_ledger_attention_query(&state.db)
        .await
        .map_err(String::from)
}

#[tauri::command]
pub async fn get_command_ledger_detail(
    state: State<'_, AppState>,
    id: i64,
) -> Result<CommandLedgerDetail, String> {
    require_admin(&state)?;
    get_command_ledger_detail_query(&state.db, id)
        .await
        .map_err(String::from)
}
