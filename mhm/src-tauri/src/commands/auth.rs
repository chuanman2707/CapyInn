use super::{get_money_vnd, get_user, require_admin, AppState};
use crate::app_error::{codes, log_system_error, CommandError, CommandResult};
use crate::models::*;
use serde_json::json;
use sqlx::Row;
use tauri::State;

// ═══════════════════════════════════════════════
// Phase 1: Auth & RBAC Commands
// ═══════════════════════════════════════════════

#[tauri::command]
pub async fn login(state: State<'_, AppState>, req: LoginRequest) -> CommandResult<LoginResponse> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(req.pin.as_bytes());
    let pin_hash = format!("{:x}", hasher.finalize());

    let row = sqlx::query(
        "SELECT id, name, role, active, created_at FROM users WHERE pin_hash = ? AND active = 1",
    )
    .bind(&pin_hash)
    .fetch_optional(&state.db)
    .await
    .map_err(|error| {
        log_system_error("login", error.to_string(), json!({ "step": "fetch_user" }))
    })?;

    let row = match row {
        Some(row) => row,
        None => {
            return Err(CommandError::user(
                codes::AUTH_INVALID_PIN,
                "Mã PIN không đúng",
            ))
        }
    };

    let user = User {
        id: row.get("id"),
        name: row.get("name"),
        role: row.get("role"),
        active: row.get::<i32, _>("active") == 1,
        created_at: row.get("created_at"),
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

    let rows =
        sqlx::query("SELECT id, name, role, active, created_at FROM users ORDER BY created_at")
            .fetch_all(&state.db)
            .await
            .map_err(|error| {
                log_system_error(
                    "list_users",
                    error.to_string(),
                    json!({ "step": "fetch_users" }),
                )
            })?;

    Ok(rows
        .iter()
        .map(|r| User {
            id: r.get("id"),
            name: r.get("name"),
            role: r.get("role"),
            active: r.get::<i32, _>("active") == 1,
            created_at: r.get("created_at"),
        })
        .collect())
}

#[tauri::command]
pub async fn create_user(
    state: State<'_, AppState>,
    req: CreateUserRequest,
) -> CommandResult<User> {
    require_admin(&state)?;

    let CreateUserRequest { name, pin, role } = req;
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(pin.as_bytes());
    let pin_hash = format!("{:x}", hasher.finalize());

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Local::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO users (id, name, pin_hash, role, active, created_at)
         VALUES (?, ?, ?, ?, 1, ?)",
    )
    .bind(&id)
    .bind(&name)
    .bind(&pin_hash)
    .bind(&role)
    .bind(&now)
    .execute(&state.db)
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
    if phone.len() < 3 {
        return Ok(vec![]);
    }

    let pattern = format!("%{}%", phone);
    let rows = sqlx::query(
        "SELECT g.id, g.full_name, g.doc_number, g.nationality,
                COUNT(bg.booking_id) as total_stays,
                COALESCE(SUM(b.total_price), 0) as total_spent,
                MAX(b.check_in_at) as last_visit
         FROM guests g
         LEFT JOIN booking_guests bg ON bg.guest_id = g.id
         LEFT JOIN bookings b ON b.id = bg.booking_id
         WHERE g.phone LIKE ?
         GROUP BY g.id
         ORDER BY last_visit DESC
         LIMIT 5",
    )
    .bind(&pattern)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| GuestSummary {
            id: r.get("id"),
            full_name: r.get("full_name"),
            doc_number: r.get("doc_number"),
            nationality: r.get("nationality"),
            total_stays: r.get::<i32, _>("total_stays"),
            total_spent: get_money_vnd(r, "total_spent"),
            last_visit: r.get("last_visit"),
        })
        .collect())
}
