use sqlx::{Pool, Sqlite, Transaction, Row};
use tauri::State;
use std::sync::{Arc, Mutex};

use crate::models::*;

use super::AppState;

pub async fn do_get_bootstrap_status(pool: &Pool<Sqlite>) -> Result<BootstrapStatus, String> {
    let setup_completed = matches!(
        crate::commands::settings::do_get_settings(pool, "setup_completed").await?,
        Some(ref value) if value == "true"
    );

    let app_lock_enabled = crate::commands::settings::do_get_settings(pool, "app_lock")
        .await?
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|json| json.get("enabled").and_then(|v| v.as_bool()))
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

fn sync_bootstrap_session(current_user: &Arc<Mutex<Option<User>>>, status: &BootstrapStatus) {
    if let Ok(mut session_user) = current_user.lock() {
        *session_user = if status.setup_completed && !status.app_lock_enabled {
            status.current_user.clone()
        } else {
            None
        };
    }
}

async fn load_default_user(pool: &Pool<Sqlite>) -> Result<Option<User>, String> {
    let Some(user_id) = crate::commands::settings::do_get_settings(pool, "default_user_id").await? else {
        return Ok(None);
    };

    let row = sqlx::query(
        "SELECT id, name, role, active, created_at FROM users WHERE id = ? AND active = 1"
    )
    .bind(&user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(row.map(|row| User {
        id: row.get("id"),
        name: row.get("name"),
        role: row.get("role"),
        active: row.get::<i32, _>("active") == 1,
        created_at: row.get("created_at"),
    }))
}

fn validate_onboarding_request(req: &OnboardingCompleteRequest) -> Result<(), String> {
    if req.hotel.name.trim().is_empty() {
        return Err("Tên khách sạn là bắt buộc".to_string());
    }
    if req.hotel.address.trim().is_empty() {
        return Err("Địa chỉ là bắt buộc".to_string());
    }
    if req.hotel.phone.trim().is_empty() {
        return Err("Số điện thoại là bắt buộc".to_string());
    }
    if !is_hhmm(&req.hotel.default_checkin_time) || !is_hhmm(&req.hotel.default_checkout_time) {
        return Err("Giờ check-in/check-out không hợp lệ".to_string());
    }
    if req.room_types.is_empty() {
        return Err("Phải có ít nhất một loại phòng".to_string());
    }
    if req.rooms.is_empty() {
        return Err("Phải có ít nhất một phòng".to_string());
    }

    let mut room_type_names = std::collections::HashSet::new();
    for room_type in &req.room_types {
        let trimmed = room_type.name.trim();
        if trimmed.is_empty() {
            return Err("Tên loại phòng là bắt buộc".to_string());
        }
        if room_type.base_price < 0.0 || room_type.extra_person_fee < 0.0 || room_type.max_guests < 1 {
            return Err(format!("Loại phòng '{}' có giá trị không hợp lệ", room_type.name));
        }
        let normalized = trimmed.to_lowercase();
        if !room_type_names.insert(normalized) {
            return Err(format!("Loại phòng '{}' bị trùng", room_type.name));
        }
    }

    let valid_room_types: std::collections::HashSet<String> =
        req.room_types.iter().map(|room_type| room_type.name.trim().to_lowercase()).collect();
    let mut room_ids = std::collections::HashSet::new();
    for room in &req.rooms {
        if room.id.trim().is_empty() || room.name.trim().is_empty() {
            return Err("Mỗi phòng phải có mã và tên".to_string());
        }
        if room.floor < 1 || room.base_price < 0.0 || room.extra_person_fee < 0.0 || room.max_guests < 1 {
            return Err(format!("Phòng '{}' có dữ liệu không hợp lệ", room.id));
        }
        if !room_ids.insert(room.id.trim().to_string()) {
            return Err(format!("Mã phòng '{}' bị trùng", room.id));
        }
        if !valid_room_types.contains(&room.room_type_name.trim().to_lowercase()) {
            return Err(format!("Phòng '{}' tham chiếu loại phòng không tồn tại", room.id));
        }
    }

    if req.app_lock.enabled {
        let admin_name = req.app_lock.admin_name.as_deref().unwrap_or("").trim();
        let pin = req.app_lock.pin.as_deref().unwrap_or("");
        if admin_name.is_empty() {
            return Err("Tên admin là bắt buộc khi bật PIN".to_string());
        }
        if pin.len() != 4 || !pin.chars().all(|c| c.is_ascii_digit()) {
            return Err("PIN phải gồm đúng 4 chữ số".to_string());
        }
    }

    Ok(())
}

fn is_hhmm(value: &str) -> bool {
    chrono::NaiveTime::parse_from_str(value, "%H:%M").is_ok()
}

async fn save_string_setting(tx: &mut Transaction<'_, Sqlite>, key: &str, value: &str) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value"
    )
    .bind(key)
    .bind(value)
    .execute(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

async fn save_json_setting(
    tx: &mut Transaction<'_, Sqlite>,
    key: &str,
    value: &serde_json::Value,
) -> Result<(), String> {
    save_string_setting(tx, key, &value.to_string()).await
}

async fn insert_initial_admin(
    tx: &mut Transaction<'_, Sqlite>,
    app_lock: &OnboardingAppLockInput,
) -> Result<User, String> {
    use sha2::{Digest, Sha256};

    let id = uuid::Uuid::new_v4().to_string();
    let name = app_lock
        .admin_name
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Owner")
        .to_string();
    let pin_source = app_lock
        .pin
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().simple().to_string()[..4].to_string());

    let mut hasher = Sha256::new();
    hasher.update(pin_source.as_bytes());
    let pin_hash = format!("{:x}", hasher.finalize());
    let now = chrono::Local::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO users (id, name, pin_hash, role, active, created_at)
         VALUES (?, ?, ?, 'admin', 1, ?)"
    )
    .bind(&id)
    .bind(&name)
    .bind(&pin_hash)
    .bind(&now)
    .execute(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;

    Ok(User {
        id,
        name,
        role: "admin".to_string(),
        active: true,
        created_at: now,
    })
}

fn room_type_id(name: &str) -> String {
    name.trim().to_lowercase().replace(' ', "_")
}

async fn insert_room_types(
    tx: &mut Transaction<'_, Sqlite>,
    room_types: &[OnboardingRoomTypeInput],
) -> Result<(), String> {
    let now = chrono::Local::now().to_rfc3339();

    for room_type in room_types {
        sqlx::query("INSERT INTO room_types (id, name, created_at) VALUES (?, ?, ?)")
            .bind(room_type_id(&room_type.name))
            .bind(room_type.name.trim())
            .bind(&now)
            .execute(&mut **tx)
            .await
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

async fn insert_rooms(
    tx: &mut Transaction<'_, Sqlite>,
    rooms: &[OnboardingRoomInput],
) -> Result<(), String> {
    for room in rooms {
        sqlx::query(
            "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'vacant')"
        )
        .bind(room.id.trim())
        .bind(room.name.trim())
        .bind(room.room_type_name.trim())
        .bind(room.floor)
        .bind(room.has_balcony as i32)
        .bind(room.base_price)
        .bind(room.max_guests)
        .bind(room.extra_person_fee)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
    }

    Ok(())
}

async fn insert_pricing_rules(
    tx: &mut Transaction<'_, Sqlite>,
    room_types: &[OnboardingRoomTypeInput],
    hotel: &OnboardingHotelInfoInput,
) -> Result<(), String> {
    let now = chrono::Local::now().to_rfc3339();

    for room_type in room_types {
        sqlx::query(
            "INSERT INTO pricing_rules
             (id, room_type, hourly_rate, overnight_rate, daily_rate,
              overnight_start, overnight_end, daily_checkin, daily_checkout,
              early_checkin_surcharge_pct, late_checkout_surcharge_pct,
              weekend_uplift_pct, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(room_type.name.trim())
        .bind((room_type.base_price / 5.0).max(0.0))
        .bind(room_type.base_price)
        .bind(room_type.base_price)
        .bind("22:00")
        .bind("11:00")
        .bind(&hotel.default_checkin_time)
        .bind(&hotel.default_checkout_time)
        .bind(30.0)
        .bind(30.0)
        .bind(0.0)
        .bind(&now)
        .bind(&now)
        .execute(&mut **tx)
        .await
        .map_err(|e| e.to_string())?;
    }

    Ok(())
}

pub async fn do_complete_onboarding(
    pool: &Pool<Sqlite>,
    req: OnboardingCompleteRequest,
) -> Result<BootstrapStatus, String> {
    validate_onboarding_request(&req)?;

    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;

    sqlx::query("DELETE FROM pricing_rules")
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM rooms")
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM room_types")
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("DELETE FROM users")
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;

    save_json_setting(
        &mut tx,
        "hotel_info",
        &serde_json::json!({
            "name": req.hotel.name,
            "address": req.hotel.address,
            "phone": req.hotel.phone,
            "rating": req.hotel.rating.clone().unwrap_or_else(|| "4.8".to_string()),
        }),
    )
    .await?;
    save_json_setting(
        &mut tx,
        "checkin_rules",
        &serde_json::json!({
            "default_checkin_time": req.hotel.default_checkin_time,
            "default_checkout_time": req.hotel.default_checkout_time,
        }),
    )
    .await?;
    save_json_setting(
        &mut tx,
        "app_lock",
        &serde_json::json!({ "enabled": req.app_lock.enabled }),
    )
    .await?;
    save_string_setting(&mut tx, "app_locale", &req.hotel.locale).await?;

    let owner = insert_initial_admin(&mut tx, &req.app_lock).await?;
    save_string_setting(&mut tx, "default_user_id", &owner.id).await?;

    insert_room_types(&mut tx, &req.room_types).await?;
    insert_rooms(&mut tx, &req.rooms).await?;
    insert_pricing_rules(&mut tx, &req.room_types, &req.hotel).await?;

    save_string_setting(&mut tx, "setup_completed", "true").await?;
    tx.commit().await.map_err(|e| e.to_string())?;

    Ok(BootstrapStatus {
        setup_completed: true,
        app_lock_enabled: req.app_lock.enabled,
        current_user: if req.app_lock.enabled { None } else { Some(owner) },
    })
}

#[tauri::command]
pub async fn get_bootstrap_status(state: State<'_, AppState>) -> Result<BootstrapStatus, String> {
    let status = do_get_bootstrap_status(&state.db).await?;
    sync_bootstrap_session(&state.current_user, &status);
    Ok(status)
}

#[tauri::command]
pub async fn complete_onboarding(
    state: State<'_, AppState>,
    req: OnboardingCompleteRequest,
) -> Result<BootstrapStatus, String> {
    let status = do_complete_onboarding(&state.db, req).await?;
    sync_bootstrap_session(&state.current_user, &status);
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::{do_complete_onboarding, do_get_bootstrap_status, sync_bootstrap_session};
    use crate::models::{
        BootstrapStatus, OnboardingAppLockInput, OnboardingCompleteRequest, OnboardingHotelInfoInput,
        OnboardingRoomInput, OnboardingRoomTypeInput,
    };
    use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};
    use std::sync::{Arc, Mutex};

    async fn test_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        crate::db::run_migrations(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn bootstrap_requires_onboarding_when_setup_completed_missing() {
        let pool = test_pool().await;
        let status = do_get_bootstrap_status(&pool).await.unwrap();

        assert!(!status.setup_completed);
        assert!(!status.app_lock_enabled);
        assert!(status.current_user.is_none());
    }

    #[tokio::test]
    async fn bootstrap_does_not_seed_demo_rooms_or_default_admin() {
        let pool = test_pool().await;
        let room_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM rooms")
            .fetch_one(&pool)
            .await
            .unwrap();
        let user_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&pool)
            .await
            .unwrap();

        assert_eq!(room_count.0, 0);
        assert_eq!(user_count.0, 0);
    }

    fn sample_onboarding_request(with_pin: bool) -> OnboardingCompleteRequest {
        OnboardingCompleteRequest {
            hotel: OnboardingHotelInfoInput {
                name: "Sunrise Hotel".to_string(),
                address: "12 Tran Hung Dao".to_string(),
                phone: "0909123456".to_string(),
                rating: Some("4.8".to_string()),
                default_checkin_time: "14:00".to_string(),
                default_checkout_time: "12:00".to_string(),
                locale: "vi".to_string(),
            },
            room_types: vec![
                OnboardingRoomTypeInput {
                    name: "Deluxe".to_string(),
                    base_price: 500_000.0,
                    max_guests: 4,
                    extra_person_fee: 50_000.0,
                    default_has_balcony: true,
                    bed_note: Some("2 giường đôi".to_string()),
                },
                OnboardingRoomTypeInput {
                    name: "Standard".to_string(),
                    base_price: 300_000.0,
                    max_guests: 2,
                    extra_person_fee: 100_000.0,
                    default_has_balcony: false,
                    bed_note: Some("1 giường đôi".to_string()),
                },
            ],
            rooms: vec![
                OnboardingRoomInput {
                    id: "1A".to_string(),
                    name: "Phòng 1A".to_string(),
                    floor: 1,
                    room_type_name: "Deluxe".to_string(),
                    has_balcony: true,
                    base_price: 500_000.0,
                    max_guests: 4,
                    extra_person_fee: 50_000.0,
                },
                OnboardingRoomInput {
                    id: "1B".to_string(),
                    name: "Phòng 1B".to_string(),
                    floor: 1,
                    room_type_name: "Standard".to_string(),
                    has_balcony: false,
                    base_price: 300_000.0,
                    max_guests: 2,
                    extra_person_fee: 100_000.0,
                },
            ],
            app_lock: OnboardingAppLockInput {
                enabled: with_pin,
                admin_name: if with_pin { Some("Owner".to_string()) } else { None },
                pin: if with_pin { Some("1234".to_string()) } else { None },
            },
        }
    }

    #[tokio::test]
    async fn complete_onboarding_with_pin_enables_lock_and_creates_admin() {
        let pool = test_pool().await;
        let req = sample_onboarding_request(true);

        let status = do_complete_onboarding(&pool, req).await.unwrap();

        assert!(status.setup_completed);
        assert!(status.app_lock_enabled);

        let room_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM rooms")
            .fetch_one(&pool)
            .await
            .unwrap();
        let user_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
            .fetch_one(&pool)
            .await
            .unwrap();

        assert_eq!(room_count.0, 2);
        assert_eq!(user_count.0, 1);
    }

    #[tokio::test]
    async fn complete_onboarding_without_pin_still_provisions_owner_for_unlocked_mode() {
        let pool = test_pool().await;
        let req = sample_onboarding_request(false);

        let status = do_complete_onboarding(&pool, req).await.unwrap();

        assert!(status.setup_completed);
        assert!(!status.app_lock_enabled);
        assert!(status.current_user.is_some());

        let owner_setting = crate::commands::settings::do_get_settings(&pool, "default_user_id")
            .await
            .unwrap();
        assert!(owner_setting.is_some());
    }

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
        assert_eq!(hydrated.as_ref().map(|user| user.id.as_str()), Some("owner"));
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
