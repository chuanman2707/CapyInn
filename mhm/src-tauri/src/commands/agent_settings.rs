use super::{get_user, require_admin_user, AppState};
use crate::{
    agent::settings::{
        get_ceo_cloud_data_opt_in as read_ceo_cloud_data_opt_in,
        set_ceo_cloud_data_opt_in_idempotent, SET_CEO_CLOUD_DATA_OPT_IN_COMMAND,
    },
    app_error::CommandResult,
    command_idempotency::WriteCommandContext,
    models::User,
};
use tauri::State;

pub(crate) fn require_ceo_cloud_opt_in_admin(user: Option<User>) -> CommandResult<User> {
    require_admin_user(user)
}

#[tauri::command]
pub async fn get_ceo_cloud_data_opt_in(state: State<'_, AppState>) -> CommandResult<bool> {
    let _user = require_ceo_cloud_opt_in_admin(get_user(&state))?;
    read_ceo_cloud_data_opt_in(&state.db).await
}

#[tauri::command]
pub async fn set_ceo_cloud_data_opt_in(
    state: State<'_, AppState>,
    enabled: bool,
    idempotency_key: String,
) -> CommandResult<()> {
    let user = require_ceo_cloud_opt_in_admin(get_user(&state))?;
    let mut ctx = WriteCommandContext::for_scoped_command(
        uuid::Uuid::new_v4().to_string(),
        idempotency_key,
        SET_CEO_CLOUD_DATA_OPT_IN_COMMAND,
    )?;
    ctx.actor_id = Some(user.id.clone());

    set_ceo_cloud_data_opt_in_idempotent(
        &state.db,
        &ctx,
        enabled,
        serde_json::json!({ "surface": "tauri" }),
    )
    .await
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_error::{codes, AppErrorKind};

    fn mock_user(role: &str) -> User {
        User {
            id: "u1".to_string(),
            name: "Test".to_string(),
            role: role.to_string(),
            active: true,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn admin_user_allowed_by_auth_helper() {
        let user =
            require_ceo_cloud_opt_in_admin(Some(mock_user("admin"))).expect("admin must pass");
        assert_eq!(user.role, "admin");
    }

    #[test]
    fn receptionist_user_rejected() {
        let error = require_ceo_cloud_opt_in_admin(Some(mock_user("receptionist")))
            .expect_err("non-admin must fail");
        assert_eq!(error.code, codes::AUTH_FORBIDDEN);
        assert_eq!(error.kind, AppErrorKind::User);
    }

    #[test]
    fn unauthenticated_user_rejected() {
        let error =
            require_ceo_cloud_opt_in_admin(None).expect_err("missing user must be rejected");
        assert_eq!(error.code, codes::AUTH_NOT_AUTHENTICATED);
        assert_eq!(error.kind, AppErrorKind::User);
    }
}
