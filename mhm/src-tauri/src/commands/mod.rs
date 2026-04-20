use crate::models::*;
use sqlx::{Pool, Row, Sqlite};
use std::sync::{Arc, Mutex};
use tauri::{Emitter, State};

/// Safely get an f64 from a SQLite row.
/// SQLite stores round numbers as INTEGER even in REAL columns,
/// so we try f64 first, then fall back to i64→f64.
pub(crate) fn get_f64(row: &sqlx::sqlite::SqliteRow, col: &str) -> f64 {
    row.try_get::<f64, _>(col)
        .unwrap_or_else(|_| row.get::<i64, _>(col) as f64)
}

pub struct AppState {
    pub db: Pool<Sqlite>,
    pub current_user: Arc<Mutex<Option<User>>>,
}

// ─── Auth Helpers ───

pub(crate) fn get_user(state: &State<'_, AppState>) -> Option<User> {
    state.current_user.lock().ok()?.clone()
}

pub(crate) fn get_user_id(state: &State<'_, AppState>) -> Option<String> {
    get_user(state).map(|u| u.id)
}

pub(crate) fn require_admin_user(user: Option<User>) -> Result<User, String> {
    let user = user.ok_or("Chưa đăng nhập".to_string())?;
    if user.role != "admin" {
        return Err("Không có quyền thực hiện. Yêu cầu quyền Admin.".to_string());
    }
    Ok(user)
}

pub(crate) fn require_admin(state: &State<'_, AppState>) -> Result<User, String> {
    require_admin_user(get_user(state))
}

pub(crate) fn emit_db_update(app: &tauri::AppHandle, entity: &str) {
    let _ = app.emit("db-updated", serde_json::json!({ "entity": entity }));
}

pub mod analytics;
pub mod audit;
pub mod auth;
pub mod billing;
pub mod bookings;
pub mod diagnostics;
pub mod groups;
pub mod guests;
pub mod invoices;
pub mod onboarding;
pub mod pricing;
pub mod reservations;
pub mod room_management;
pub mod rooms;
pub mod settings;

// Re-export all Tauri commands for lib.rs registration

// Re-export do_* helpers used by gateway
pub use bookings::do_get_all_bookings;
pub use invoices::do_get_invoice;
pub use pricing::{do_calculate_price_preview, do_get_pricing_rules};
pub use reservations::{
    do_cancel_reservation, do_check_availability, do_create_reservation, do_get_rooms_availability,
    do_modify_reservation,
};
pub use room_management::do_get_room_types;
pub use rooms::{do_get_dashboard_stats, do_get_room_detail, do_get_rooms};

#[cfg(test)]
mod tests {
    use super::require_admin_user;
    use crate::models::User;

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
    fn require_admin_user_rejects_missing_user() {
        let error = require_admin_user(None).expect_err("missing user must be rejected");
        assert_eq!(error, "Chưa đăng nhập");
    }

    #[test]
    fn require_admin_user_rejects_non_admin_user() {
        let error =
            require_admin_user(Some(mock_user("receptionist"))).expect_err("non-admin must fail");
        assert_eq!(error, "Không có quyền thực hiện. Yêu cầu quyền Admin.");
    }

    #[test]
    fn require_admin_user_accepts_admin_user() {
        let user = require_admin_user(Some(mock_user("admin"))).expect("admin must pass");
        assert_eq!(user.role, "admin");
    }
}
