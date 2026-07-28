use super::{get_user, require_admin, AppState};
use crate::app_error::{codes, log_system_error, CommandError, CommandResult};
use crate::domain::auth::credentials::pin_hash;
use crate::models::*;
use crate::queries::auth::user_queries;
use crate::queries::guests::guest_queries;
use crate::repositories::auth::user_repository::{self, NewUser};
use serde_json::json;
use tauri::State;

// ═══════════════════════════════════════════════
// Phase 1: Auth & RBAC Commands
// ═══════════════════════════════════════════════

/// Quick check-in searches on at least this many digits, so a stray keystroke
/// does not pull the whole guest book back.
const MIN_PHONE_SEARCH_LEN: usize = 3;

/// Quick check-in shows a short pick-list, not a full result set.
const PHONE_SEARCH_LIMIT: i32 = 5;

#[tauri::command]
pub async fn login(state: State<'_, AppState>, req: LoginRequest) -> CommandResult<LoginResponse> {
    let user = user_queries::load_active_user_by_pin_hash(&state.db, &pin_hash(&req.pin))
        .await
        .map_err(|error| {
            log_system_error("login", error.to_string(), json!({ "step": "fetch_user" }))
        })?;

    let user = match user {
        Some(user) => user,
        None => {
            return Err(CommandError::user(
                codes::AUTH_INVALID_PIN,
                "Mã PIN không đúng",
            ))
        }
    };

    // Store in AppState
    let mut current = state.current_user.lock().map_err(|error| {
        log_system_error(
            "login",
            error.to_string(),
            json!({ "step": "store_current_user" }),
        )
    })?;
    *current = Some(user.clone());

    Ok(LoginResponse { user })
}

#[tauri::command]
pub async fn logout(state: State<'_, AppState>) -> Result<(), String> {
    if let Ok(mut current) = state.current_user.lock() {
        *current = None;
    }
    Ok(())
}

#[tauri::command]
pub async fn get_current_user(state: State<'_, AppState>) -> Result<Option<User>, String> {
    Ok(get_user(&state))
}

#[tauri::command]
pub async fn list_users(state: State<'_, AppState>) -> CommandResult<Vec<User>> {
    require_admin(&state)?;

    user_queries::load_users(&state.db).await.map_err(|error| {
        log_system_error(
            "list_users",
            error.to_string(),
            json!({ "step": "fetch_users" }),
        )
    })
}

#[tauri::command]
pub async fn create_user(
    state: State<'_, AppState>,
    req: CreateUserRequest,
) -> CommandResult<User> {
    require_admin(&state)?;

    let CreateUserRequest { name, pin, role } = req;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Local::now().to_rfc3339();

    user_repository::insert_user(
        &state.db,
        NewUser {
            id: &id,
            name: &name,
            pin_hash: &pin_hash(&pin),
            role: &role,
            created_at: &now,
        },
    )
    .await
    .map_err(|error| {
        log_system_error(
            "create_user",
            error.to_string(),
            json!({ "step": "insert_user", "name": &name, "role": &role }),
        )
    })?;

    Ok(User {
        id,
        name,
        role,
        active: true,
        created_at: now,
    })
}

// ─── Search Guest by Phone (Quick Check-in) ───

#[tauri::command]
pub async fn search_guest_by_phone(
    state: State<'_, AppState>,
    phone: String,
) -> Result<Vec<GuestSummary>, String> {
    if phone.len() < MIN_PHONE_SEARCH_LEN {
        return Ok(vec![]);
    }

    guest_queries::search_guest_summaries_by_phone(&state.db, &phone, PHONE_SEARCH_LIMIT)
        .await
        .map_err(|e| e.to_string())
}
