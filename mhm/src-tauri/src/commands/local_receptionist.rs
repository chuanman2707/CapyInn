use super::AppState;
use crate::{
    agent::receptionist_demo::{
        run_local_receptionist_chat, LocalReceptionistChatRequest, LocalReceptionistChatResponse,
    },
    app_error::CommandResult,
};
use tauri::State;

#[tauri::command]
pub async fn local_receptionist_chat(
    state: State<'_, AppState>,
    endpoint: String,
    model: String,
    message: String,
) -> CommandResult<LocalReceptionistChatResponse> {
    run_local_receptionist_chat(
        &state.db,
        LocalReceptionistChatRequest {
            endpoint,
            model,
            message,
        },
    )
    .await
}
