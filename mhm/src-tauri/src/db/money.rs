use sqlx::{Connection, Pool, Sqlite};

use super::{execute_compat_alter, restore_foreign_keys_after_v14_migration, set_schema_version};

pub(super) async fn migrate_v14_integer_vnd_money(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let mut conn = pool.acquire().await?;
    sqlx::query("PRAGMA foreign_keys=OFF")
        .execute(&mut *conn)
        .await?;

    let migration_result = (*conn)
        .transaction(|tx| {
            Box::pin(async move {
                execute_compat_alter(
                    tx,
                    "ALTER TABLE command_idempotency ADD COLUMN legacy_request_hash TEXT",
                )
                .await?;
                crate::money_migration::migrate_integer_vnd_money(tx).await?;
                set_schema_version(tx, 14).await?;
                Ok::<(), sqlx::Error>(())
            })
        })
        .await;

    restore_foreign_keys_after_v14_migration(&mut conn, migration_result).await?;
    Ok(())
}
