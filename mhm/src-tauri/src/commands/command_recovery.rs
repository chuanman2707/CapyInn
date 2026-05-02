use super::{require_admin, AppState};
use crate::app_error::CommandResult;
use crate::command_recovery::{
    dismiss_command_recovery as dismiss_command_recovery_action,
    inspect_command_recovery as inspect_command_recovery_query,
    list_command_recovery_queue as list_command_recovery_queue_query,
    mark_command_recovery_terminal as mark_command_recovery_terminal_action,
    request_command_recovery_retry as request_command_recovery_retry_action,
    CommandRecoveryActionRequest, CommandRecoveryDetail, CommandRecoveryQueueItem,
    RecoveryActionResponse, RecoveryOperator,
};
use crate::models::User;
use tauri::State;

fn recovery_operator_from_admin(user: &User) -> RecoveryOperator {
    RecoveryOperator {
        id: user.id.clone(),
        role: user.role.clone(),
    }
}

#[tauri::command]
pub async fn list_command_recovery_queue(
    state: State<'_, AppState>,
) -> CommandResult<Vec<CommandRecoveryQueueItem>> {
    require_admin(&state)?;
    list_command_recovery_queue_query(&state.db).await
}

#[tauri::command]
pub async fn inspect_command_recovery(
    state: State<'_, AppState>,
    id: i64,
) -> CommandResult<CommandRecoveryDetail> {
    require_admin(&state)?;
    inspect_command_recovery_query(&state.db, id).await
}

#[tauri::command]
pub async fn request_command_recovery_retry(
    state: State<'_, AppState>,
    request: CommandRecoveryActionRequest,
) -> CommandResult<RecoveryActionResponse> {
    let user = require_admin(&state)?;
    request_command_recovery_retry_action(&state.db, recovery_operator_from_admin(&user), request)
        .await
}

#[tauri::command]
pub async fn dismiss_command_recovery(
    state: State<'_, AppState>,
    request: CommandRecoveryActionRequest,
) -> CommandResult<RecoveryActionResponse> {
    let user = require_admin(&state)?;
    dismiss_command_recovery_action(&state.db, recovery_operator_from_admin(&user), request).await
}

#[tauri::command]
pub async fn mark_command_recovery_terminal(
    state: State<'_, AppState>,
    request: CommandRecoveryActionRequest,
) -> CommandResult<RecoveryActionResponse> {
    let user = require_admin(&state)?;
    mark_command_recovery_terminal_action(&state.db, recovery_operator_from_admin(&user), request)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(role: &str) -> User {
        User {
            id: "admin-1".to_string(),
            name: "Admin".to_string(),
            role: role.to_string(),
            active: true,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn recovery_operator_uses_current_admin_identity() {
        let admin = user("admin");

        let operator = recovery_operator_from_admin(&admin);

        assert_eq!(operator.id, "admin-1");
        assert_eq!(operator.role, "admin");
    }

    #[test]
    fn wrapper_functions_remain_registered_at_compile_time() {
        let _ = list_command_recovery_queue;
        let _ = inspect_command_recovery;
        let _ = request_command_recovery_retry;
        let _ = dismiss_command_recovery;
        let _ = mark_command_recovery_terminal;
    }
}
