use sqlx::{Pool, Row, Sqlite};

use crate::models::BootstrapStatus;

pub async fn read_bootstrap_status(pool: &Pool<Sqlite>) -> Result<BootstrapStatus, String> {
    let setup_completed = matches!(
        crate::services::settings_store::get_setting(pool, "setup_completed").await?,
        Some(ref value) if value == "true"
    );

    let app_lock_enabled = crate::services::settings_store::get_setting(pool, "app_lock")
        .await?
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|json| json.get("enabled").and_then(|value| value.as_bool()))
        .unwrap_or(false);

    let current_user = if setup_completed && !app_lock_enabled {
        load_default_user(pool).await?
    } else {
        None
    };

    Ok(BootstrapStatus {
        setup_completed,
        app_lock_enabled,
        current_user,
    })
}

async fn load_default_user(pool: &Pool<Sqlite>) -> Result<Option<crate::models::User>, String> {
    let Some(user_id) =
        crate::services::settings_store::get_setting(pool, "default_user_id").await?
    else {
        return Ok(None);
    };

    let row = sqlx::query(
        "SELECT id, name, role, active, created_at FROM users WHERE id = ? AND active = 1",
    )
    .bind(&user_id)
    .fetch_optional(pool)
    .await
    .map_err(|error| error.to_string())?;

    Ok(row.map(|row| crate::models::User {
        id: row.get("id"),
        name: row.get("name"),
        role: row.get("role"),
        active: row.get::<i32, _>("active") == 1,
        created_at: row.get("created_at"),
    }))
}
