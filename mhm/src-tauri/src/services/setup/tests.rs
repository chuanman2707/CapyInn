use super::{complete_setup, read_bootstrap_status};
use crate::models::{
    OnboardingAppLockInput, OnboardingCompleteRequest, OnboardingHotelInfoInput,
    OnboardingRoomInput, OnboardingRoomTypeInput,
};
use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};

async fn test_pool() -> Pool<Sqlite> {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    crate::db::run_migrations(&pool).await.unwrap();
    pool
}

async fn seed_setting(pool: &Pool<Sqlite>, key: &str, value: &str) {
    crate::services::settings_store::save_setting(pool, key, value)
        .await
        .unwrap();
}

async fn seed_default_user(pool: &Pool<Sqlite>, id: &str) {
    sqlx::query(
        "INSERT INTO users (id, name, pin_hash, role, active, created_at)
         VALUES (?, ?, ?, 'admin', 1, ?)",
    )
    .bind(id)
    .bind("Owner")
    .bind("hash")
    .bind("2026-04-15T00:00:00+07:00")
    .execute(pool)
    .await
    .unwrap();
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
            admin_name: if with_pin {
                Some("Owner".to_string())
            } else {
                None
            },
            pin: if with_pin {
                Some("1234".to_string())
            } else {
                None
            },
        },
    }
}

#[tokio::test]
async fn read_bootstrap_status_reports_incomplete_setup_before_setup_is_done() {
    let pool = test_pool().await;

    let status = read_bootstrap_status(&pool).await.unwrap();

    assert!(!status.setup_completed);
    assert!(!status.app_lock_enabled);
    assert!(status.current_user.is_none());
}

#[tokio::test]
async fn read_bootstrap_status_returns_default_user_for_completed_unlocked_setup() {
    let pool = test_pool().await;
    seed_setting(&pool, "setup_completed", "true").await;
    seed_setting(&pool, "app_lock", r#"{"enabled":false}"#).await;
    seed_setting(&pool, "default_user_id", "owner-1").await;
    seed_default_user(&pool, "owner-1").await;

    let status = read_bootstrap_status(&pool).await.unwrap();

    assert!(status.setup_completed);
    assert!(!status.app_lock_enabled);
    let current_user = status.current_user.expect("default user should be loaded");
    assert_eq!(current_user.id, "owner-1");
    assert_eq!(current_user.name, "Owner");
}

#[tokio::test]
async fn read_bootstrap_status_returns_no_current_user_for_completed_locked_setup() {
    let pool = test_pool().await;
    seed_setting(&pool, "setup_completed", "true").await;
    seed_setting(&pool, "app_lock", r#"{"enabled":true}"#).await;
    seed_setting(&pool, "default_user_id", "owner-1").await;
    seed_default_user(&pool, "owner-1").await;

    let status = read_bootstrap_status(&pool).await.unwrap();

    assert!(status.setup_completed);
    assert!(status.app_lock_enabled);
    assert!(status.current_user.is_none());
}

#[tokio::test]
async fn complete_setup_persists_canonical_settings_contract() {
    let pool = test_pool().await;

    let status = complete_setup(&pool, sample_onboarding_request(false))
        .await
        .expect("complete_setup should succeed");

    assert!(status.setup_completed);
    assert!(!status.app_lock_enabled);

    let hotel_info = crate::services::settings_store::get_setting(&pool, "hotel_info")
        .await
        .expect("hotel_info setting should load");
    let hotel_info: serde_json::Value = serde_json::from_str(
        &hotel_info.expect("hotel_info should be stored after successful setup"),
    )
    .expect("hotel_info should be valid json");
    assert_eq!(
        hotel_info,
        serde_json::json!({
            "name": "Sunrise Hotel",
            "address": "12 Tran Hung Dao",
            "phone": "0909123456",
            "rating": "4.8",
        })
    );

    let checkin_rules = crate::services::settings_store::get_setting(&pool, "checkin_rules")
        .await
        .expect("checkin_rules setting should load");
    let checkin_rules: serde_json::Value = serde_json::from_str(
        &checkin_rules.expect("checkin_rules should be stored after successful setup"),
    )
    .expect("checkin_rules should be valid json");
    assert_eq!(
        checkin_rules,
        serde_json::json!({
            "checkin": "14:00",
            "checkout": "12:00",
        })
    );

    let default_user_id = crate::services::settings_store::get_setting(&pool, "default_user_id")
        .await
        .expect("default_user_id should load");
    assert_eq!(
        default_user_id,
        status.current_user.as_ref().map(|user| user.id.clone())
    );

    let setup_completed = crate::services::settings_store::get_setting(&pool, "setup_completed")
        .await
        .expect("setup_completed should load");
    assert_eq!(setup_completed.as_deref(), Some("true"));
}

#[tokio::test]
async fn complete_setup_without_app_lock_creates_a_default_user_ready_for_login() {
    let pool = test_pool().await;

    let status = complete_setup(&pool, sample_onboarding_request(false))
        .await
        .expect("complete_setup should succeed");

    let default_user_id = crate::services::settings_store::get_setting(&pool, "default_user_id")
        .await
        .expect("default_user_id should load")
        .expect("default_user_id should exist");

    let current_user = status.current_user.expect("unlocked setup should hydrate the default user");
    assert_eq!(current_user.id, default_user_id);
    assert_eq!(current_user.name, "Owner");
}

#[tokio::test]
async fn complete_setup_returns_locked_status_when_app_lock_is_enabled() {
    let pool = test_pool().await;

    let status = complete_setup(&pool, sample_onboarding_request(true))
        .await
        .expect("complete_setup should succeed when app lock is enabled");

    assert!(status.setup_completed);
    assert!(status.app_lock_enabled);
    assert!(status.current_user.is_none());

    let default_user_id = crate::services::settings_store::get_setting(&pool, "default_user_id")
        .await
        .expect("default_user_id should load");
    let default_user_id = default_user_id.expect("default_user_id should be stored");

    let owner: (String, String) =
        sqlx::query_as("SELECT id, name FROM users WHERE role = 'admin' LIMIT 1")
            .fetch_one(&pool)
            .await
            .expect("admin user should be created");
    assert_eq!(owner.0, default_user_id);
    assert_eq!(owner.1, "Owner");

    let app_lock = crate::services::settings_store::get_setting(&pool, "app_lock")
        .await
        .expect("app_lock should load");
    let app_lock: serde_json::Value =
        serde_json::from_str(&app_lock.expect("app_lock should be stored after successful setup"))
            .expect("app_lock should be valid json");
    assert_eq!(
        app_lock,
        serde_json::json!({
            "enabled": true,
        })
    );
}

#[tokio::test]
async fn complete_setup_rejects_duplicate_room_ids() {
    let pool = test_pool().await;
    let mut req = sample_onboarding_request(false);
    req.rooms[1].id = req.rooms[0].id.clone();

    let error = complete_setup(&pool, req)
        .await
        .expect_err("duplicate room ids should be rejected");

    assert!(
        error.contains("Mã phòng '1A' bị trùng"),
        "unexpected error: {error}"
    );
}

#[tokio::test]
async fn complete_setup_rejects_repeated_setup() {
    let pool = test_pool().await;

    complete_setup(&pool, sample_onboarding_request(false))
        .await
        .expect("initial setup should succeed");

    let error = complete_setup(&pool, sample_onboarding_request(false))
        .await
        .expect_err("repeated setup should be rejected");

    assert_eq!(error, "Setup đã hoàn thành, không thể thực hiện lại.");
}
