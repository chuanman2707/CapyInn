use sqlx::{Pool, Sqlite, Transaction};

pub async fn get_setting(pool: &Pool<Sqlite>, key: &str) -> Result<Option<String>, String> {
    sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .map_err(|e| e.to_string())
}

pub async fn save_setting(pool: &Pool<Sqlite>, key: &str, value: &str) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    save_setting_tx(&mut tx, key, value).await?;
    tx.commit().await.map_err(|e| e.to_string())
}

pub async fn save_setting_tx(
    tx: &mut Transaction<'_, Sqlite>,
    key: &str,
    value: &str,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(&mut **tx)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> Pool<Sqlite> {
        let database_url = format!(
            "sqlite://file:{}?mode=memory&cache=shared",
            uuid::Uuid::new_v4()
        );

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .expect("failed to open sqlite test pool");

        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("failed to enable foreign keys");

        crate::db::run_migrations(&pool)
            .await
            .expect("failed to run migrations");

        pool
    }

    #[tokio::test]
    async fn save_setting_round_trips_a_value() {
        let pool = test_pool().await;

        save_setting(&pool, "hotel_name", "CapyInn")
            .await
            .expect("save_setting should succeed");

        let value = get_setting(&pool, "hotel_name")
            .await
            .expect("get_setting should succeed");

        assert_eq!(value, Some("CapyInn".to_string()));
    }

    #[tokio::test]
    async fn save_setting_tx_updates_a_value_inside_a_transaction() {
        let pool = test_pool().await;

        save_setting(&pool, "hotel_name", "Before")
            .await
            .expect("seed setting should succeed");

        let mut tx = pool.begin().await.expect("transaction should begin");
        save_setting_tx(&mut tx, "hotel_name", "After")
            .await
            .expect("save_setting_tx should succeed");
        tx.commit().await.expect("transaction should commit");

        let value = get_setting(&pool, "hotel_name")
            .await
            .expect("get_setting should succeed");

        assert_eq!(value, Some("After".to_string()));
    }
}
