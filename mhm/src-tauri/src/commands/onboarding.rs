use std::sync::{Arc, Mutex};
use tauri::State;

use crate::models::*;
use crate::services::setup::{complete_setup, read_bootstrap_status};

use super::AppState;

fn sync_bootstrap_session(current_user: &Arc<Mutex<Option<User>>>, status: &BootstrapStatus) {
    if let Ok(mut session_user) = current_user.lock() {
        *session_user = if status.setup_completed && !status.app_lock_enabled {
            status.current_user.clone()
        } else {
            None
        };
    }
}

#[tauri::command]
pub async fn get_bootstrap_status(state: State<'_, AppState>) -> Result<BootstrapStatus, String> {
    let status = read_bootstrap_status(&state.db).await?;
    sync_bootstrap_session(&state.current_user, &status);
    Ok(status)
}

#[tauri::command]
pub async fn complete_onboarding(
    state: State<'_, AppState>,
    req: OnboardingCompleteRequest,
) -> Result<BootstrapStatus, String> {
    let status = complete_setup(&state.db, req).await?;
    sync_bootstrap_session(&state.current_user, &status);
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::sync_bootstrap_session;
    use crate::models::BootstrapStatus;
    use std::sync::{Arc, Mutex};

    #[test]
    fn sync_bootstrap_session_populates_current_user_for_unlocked_mode() {
        let current_user = Arc::new(Mutex::new(None));
        let status = BootstrapStatus {
            setup_completed: true,
            app_lock_enabled: false,
            current_user: Some(crate::models::User {
                id: "owner".to_string(),
                name: "Owner".to_string(),
                role: "admin".to_string(),
                active: true,
                created_at: "2026-04-15T00:00:00+07:00".to_string(),
            }),
        };

        sync_bootstrap_session(&current_user, &status);

        let hydrated = current_user.lock().unwrap().clone();
        assert_eq!(
            hydrated.as_ref().map(|user| user.id.as_str()),
            Some("owner")
        );
    }

    #[test]
    fn sync_bootstrap_session_clears_current_user_for_locked_mode() {
        let current_user = Arc::new(Mutex::new(Some(crate::models::User {
            id: "owner".to_string(),
            name: "Owner".to_string(),
            role: "admin".to_string(),
            active: true,
            created_at: "2026-04-15T00:00:00+07:00".to_string(),
        })));
        let status = BootstrapStatus {
            setup_completed: true,
            app_lock_enabled: true,
            current_user: None,
        };

        sync_bootstrap_session(&current_user, &status);

        assert!(current_user.lock().unwrap().is_none());
    }
}
