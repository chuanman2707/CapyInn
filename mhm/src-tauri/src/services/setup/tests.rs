use super::read_bootstrap_status;
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
