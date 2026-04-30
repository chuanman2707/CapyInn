use sqlx::{Pool, Sqlite, Transaction};

use crate::models::{
    BootstrapStatus, OnboardingAppLockInput, OnboardingCompleteRequest, OnboardingRoomInput,
    OnboardingRoomTypeInput, User,
};
use crate::services::settings_store::{get_setting, save_setting_tx};

use super::read_bootstrap_status;

pub async fn complete_setup(
    pool: &Pool<Sqlite>,
    req: OnboardingCompleteRequest,
) -> Result<BootstrapStatus, String> {
    if matches!(
        get_setting(pool, "setup_completed").await?,
        Some(ref value) if value == "true"
    ) {
        return Err("Setup đã hoàn thành, không thể thực hiện lại.".to_string());
    }

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
            "name": &req.hotel.name,
            "address": &req.hotel.address,
            "phone": &req.hotel.phone,
            "rating": req.hotel.rating.clone().unwrap_or_else(|| "4.8".to_string()),
        }),
    )
    .await?;
    save_json_setting(
        &mut tx,
        "checkin_rules",
        &serde_json::json!({
            "checkin": &req.hotel.default_checkin_time,
            "checkout": &req.hotel.default_checkout_time,
        }),
    )
    .await?;
    save_json_setting(
        &mut tx,
        "app_lock",
        &serde_json::json!({ "enabled": req.app_lock.enabled }),
    )
    .await?;
    save_setting_tx(&mut tx, "app_locale", &req.hotel.locale).await?;

    let owner = insert_initial_admin(&mut tx, &req.app_lock).await?;
    save_setting_tx(&mut tx, "default_user_id", &owner.id).await?;

    insert_room_types(&mut tx, &req.room_types).await?;
    insert_rooms(&mut tx, &req.rooms).await?;
    insert_pricing_rules(
        &mut tx,
        &req.room_types,
        &req.hotel.default_checkin_time,
        &req.hotel.default_checkout_time,
    )
    .await?;

    save_setting_tx(&mut tx, "setup_completed", "true").await?;
    tx.commit().await.map_err(|e| e.to_string())?;

    read_bootstrap_status(pool).await
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
        if room_type.base_price < 0 || room_type.extra_person_fee < 0 || room_type.max_guests < 1 {
            return Err(format!(
                "Loại phòng '{}' có giá trị không hợp lệ",
                room_type.name
            ));
        }
        let normalized = trimmed.to_lowercase();
        if !room_type_names.insert(normalized) {
            return Err(format!("Loại phòng '{}' bị trùng", room_type.name));
        }
    }

    let valid_room_types: std::collections::HashSet<String> = req
        .room_types
        .iter()
        .map(|room_type| room_type.name.trim().to_lowercase())
        .collect();
    let mut room_ids = std::collections::HashSet::new();
    for room in &req.rooms {
        if room.id.trim().is_empty() || room.name.trim().is_empty() {
            return Err("Mỗi phòng phải có mã và tên".to_string());
        }
        if room.floor < 1 || room.base_price < 0 || room.extra_person_fee < 0 || room.max_guests < 1
        {
            return Err(format!("Phòng '{}' có dữ liệu không hợp lệ", room.id));
        }
        if !room_ids.insert(room.id.trim().to_string()) {
            return Err(format!("Mã phòng '{}' bị trùng", room.id));
        }
        if !valid_room_types.contains(&room.room_type_name.trim().to_lowercase()) {
            return Err(format!(
                "Phòng '{}' tham chiếu loại phòng không tồn tại",
                room.id
            ));
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

async fn save_json_setting(
    tx: &mut Transaction<'_, Sqlite>,
    key: &str,
    value: &serde_json::Value,
) -> Result<(), String> {
    save_setting_tx(tx, key, &value.to_string()).await
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
         VALUES (?, ?, ?, 'admin', 1, ?)",
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
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 'vacant')",
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
    default_checkin_time: &str,
    default_checkout_time: &str,
) -> Result<(), String> {
    let now = chrono::Local::now().to_rfc3339();

    for room_type in room_types {
        sqlx::query(
            "INSERT INTO pricing_rules
             (id, room_type, hourly_rate, overnight_rate, daily_rate,
              overnight_start, overnight_end, daily_checkin, daily_checkout,
              early_checkin_surcharge_pct, late_checkout_surcharge_pct,
              weekend_uplift_pct, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(room_type.name.trim())
        .bind((room_type.base_price / 5).max(0))
        .bind(room_type.base_price)
        .bind(room_type.base_price)
        .bind("22:00")
        .bind("11:00")
        .bind(default_checkin_time)
        .bind(default_checkout_time)
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
