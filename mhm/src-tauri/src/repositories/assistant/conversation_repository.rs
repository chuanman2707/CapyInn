//! Mọi lệnh ghi vào sổ hội thoại của trợ lý quầy.
//!
//! Tầng này không quyết định chính sách: ai được ghi, tên hội thoại cắt ra sao
//! đều đã chốt ở `services::assistant::conversation_service` trước khi tới đây.
//!
//! `repositories` là module riêng của crate, nên tới khi service đó ra đời thì
//! mọi thứ ở đây chỉ có test là caller — đó là lý do các `#[allow(dead_code)]`
//! bên dưới. Bỏ chúng đi khi tầng service gọi vào.

use sqlx::{Pool, Sqlite};

#[allow(dead_code)]
pub async fn insert_conversation(
    pool: &Pool<Sqlite>,
    id: &str,
    user_id: &str,
    title: &str,
    now: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO assistant_conversations (id, user_id, title, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(user_id)
    .bind(title)
    .bind(now)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

#[allow(dead_code)]
pub async fn insert_message(
    pool: &Pool<Sqlite>,
    id: &str,
    conversation_id: &str,
    kind: &str,
    text: &str,
    now: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO assistant_messages (id, conversation_id, kind, text, created_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id)
    .bind(conversation_id)
    .bind(kind)
    .bind(text)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

#[allow(dead_code)]
pub async fn touch_conversation(
    pool: &Pool<Sqlite>,
    id: &str,
    now: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE assistant_conversations SET updated_at = ? WHERE id = ?")
        .bind(now)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Trả số dòng bị xoá để caller phân biệt "đã xoá" với "không có hội thoại đó".
#[allow(dead_code)]
pub async fn delete_conversation(pool: &Pool<Sqlite>, id: &str) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("DELETE FROM assistant_conversations WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

#[allow(dead_code)]
pub async fn delete_all_conversations(pool: &Pool<Sqlite>) -> Result<u64, sqlx::Error> {
    let result = sqlx::query("DELETE FROM assistant_conversations")
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
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
            .expect("mở pool test");
        crate::db::run_migrations(&pool).await.expect("migration");
        pool
    }

    const NOW: &str = "2026-08-04T10:00:00+07:00";

    #[tokio::test]
    async fn insert_and_delete_round_trip() {
        let pool = test_pool().await;

        insert_conversation(&pool, "c1", "u1", "Hỏi phòng", NOW)
            .await
            .expect("tạo hội thoại");
        insert_message(
            &pool,
            "m1",
            "c1",
            "user",
            "Tối nay còn phòng nào trống?",
            NOW,
        )
        .await
        .expect("ghi tin nhắn");

        let removed = delete_conversation(&pool, "c1").await.expect("xoá");
        assert_eq!(removed, 1);

        let removed_again = delete_conversation(&pool, "c1").await.expect("xoá lần hai");
        assert_eq!(removed_again, 0, "xoá id không tồn tại không phải lỗi");
    }

    /// `touch_conversation` là chỗ duy nhất `updated_at` được đổi. Nếu nó ghi
    /// nhầm sang `created_at` thì danh sách lịch sử sắp sai mà không ai thấy.
    #[tokio::test]
    async fn touch_moves_updated_at_and_leaves_created_at_alone() {
        let pool = test_pool().await;
        insert_conversation(&pool, "c1", "u1", "Hỏi phòng", NOW)
            .await
            .expect("tạo");

        touch_conversation(&pool, "c1", "2026-08-04T11:30:00+07:00")
            .await
            .expect("touch");

        let (created, updated): (String, String) = sqlx::query_as(
            "SELECT created_at, updated_at FROM assistant_conversations WHERE id = 'c1'",
        )
        .fetch_one(&pool)
        .await
        .expect("đọc lại");

        assert_eq!(created, NOW, "created_at không được đổi");
        assert_eq!(updated, "2026-08-04T11:30:00+07:00");
    }

    #[tokio::test]
    async fn delete_all_removes_every_conversation() {
        let pool = test_pool().await;
        insert_conversation(&pool, "c1", "u1", "A", NOW)
            .await
            .expect("c1");
        insert_conversation(&pool, "c2", "u2", "B", NOW)
            .await
            .expect("c2");

        let removed = delete_all_conversations(&pool).await.expect("xoá sạch");

        assert_eq!(removed, 2);
    }
}
