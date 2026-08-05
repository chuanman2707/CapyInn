//! Mọi lệnh đọc sổ hội thoại của trợ lý quầy.
//!
//! `list_conversations(pool, None, …)` là đường của admin — trả về của mọi
//! người. `Some(user_id)` là đường của lễ tân. Việc quyết định gọi đường nào
//! nằm ở tầng command, không nằm ở đây.
//!
//! `queries` là module riêng của crate, nên tới khi
//! `services::assistant::conversation_service` gọi vào thì mọi thứ ở đây chỉ
//! có test là caller — đó là lý do các `#[allow(dead_code)]` bên dưới. Bỏ
//! chúng đi khi tầng service ra đời.

use serde::Serialize;
use sqlx::{FromRow, Pool, Sqlite};

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ConversationSummary {
    pub id: String,
    pub user_id: String,
    pub user_name: String,
    pub title: String,
    pub updated_at: String,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct StoredMessage {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub created_at: String,
}

#[allow(dead_code)]
pub async fn list_conversations(
    pool: &Pool<Sqlite>,
    user_id: Option<&str>,
    limit: i64,
) -> Result<Vec<ConversationSummary>, sqlx::Error> {
    // LEFT JOIN chứ không JOIN: tài khoản bị xoá để lại hội thoại mồ côi, và
    // admin vẫn phải thấy chúng để mà xoá.
    sqlx::query_as::<_, ConversationSummary>(
        "SELECT c.id            AS id,
                c.user_id       AS user_id,
                COALESCE(u.name, '(tài khoản đã xoá)') AS user_name,
                c.title         AS title,
                c.updated_at    AS updated_at
         FROM assistant_conversations c
         LEFT JOIN users u ON u.id = c.user_id
         WHERE (?1 IS NULL OR c.user_id = ?1)
         ORDER BY c.updated_at DESC
         LIMIT ?2",
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}

#[allow(dead_code)]
pub async fn get_conversation_owner(
    pool: &Pool<Sqlite>,
    id: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT user_id FROM assistant_conversations WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Lấy `limit` tin **mới nhất** rồi trả về theo thứ tự **cũ trước**. Hai chiều
/// khác nhau nên phải bọc: `ORDER BY … DESC LIMIT` để chọn đúng phần đuôi,
/// rồi đảo lại để đọc xuôi.
#[allow(dead_code)]
pub async fn get_messages(
    pool: &Pool<Sqlite>,
    conversation_id: &str,
    limit: i64,
) -> Result<Vec<StoredMessage>, sqlx::Error> {
    sqlx::query_as::<_, StoredMessage>(
        "SELECT id, kind, text, created_at FROM (
             SELECT id, kind, text, created_at
             FROM assistant_messages
             WHERE conversation_id = ?
             ORDER BY created_at DESC, id DESC
             LIMIT ?
         ) ORDER BY created_at ASC, id ASC",
    )
    .bind(conversation_id)
    .bind(limit)
    .fetch_all(pool)
    .await
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

    async fn seeded_pool() -> Pool<Sqlite> {
        let pool = test_pool().await;

        for (id, name) in [("u1", "Lễ tân A"), ("u2", "Lễ tân B")] {
            sqlx::query(
                "INSERT INTO users (id, name, pin_hash, role, active, created_at)
                 VALUES (?, ?, 'x', 'receptionist', 1, '2026-08-01T00:00:00+07:00')",
            )
            .bind(id)
            .bind(name)
            .execute(&pool)
            .await
            .expect("chèn user");
        }

        sqlx::query(
            "INSERT INTO assistant_conversations (id, user_id, title, created_at, updated_at)
             VALUES ('c1', 'u1', 'Hỏi phòng', '2026-08-04T10:00:00+07:00', '2026-08-04T10:00:00+07:00'),
                    ('c2', 'u2', 'Hỏi giá',  '2026-08-04T09:00:00+07:00', '2026-08-04T12:00:00+07:00')",
        )
        .execute(&pool)
        .await
        .expect("chèn hội thoại");

        sqlx::query(
            "INSERT INTO assistant_messages (id, conversation_id, kind, text, created_at)
             VALUES ('m1', 'c1', 'user', 'câu hỏi', '2026-08-04T10:00:00+07:00'),
                    ('m2', 'c1', 'assistant', 'câu trả lời', '2026-08-04T10:01:00+07:00')",
        )
        .execute(&pool)
        .await
        .expect("chèn tin nhắn");

        pool
    }

    /// Lễ tân chỉ được thấy hội thoại của mình. Hội thoại chứa tên khách và
    /// CCCD, nên đây là luật chống rò chứ không phải luật tiện dụng.
    #[tokio::test]
    async fn listing_for_a_user_hides_everyone_else() {
        let pool = seeded_pool().await;

        let mine = list_conversations(&pool, Some("u1"), 50)
            .await
            .expect("của tôi");

        assert_eq!(mine.len(), 1);
        assert_eq!(mine[0].id, "c1");
    }

    /// Admin nhận cả danh sách, nên mỗi dòng phải mang tên người tạo — không
    /// thì admin bấm xoá mà không biết đang xoá của ai.
    #[tokio::test]
    async fn listing_for_admin_returns_everyone_with_the_owner_name() {
        let pool = seeded_pool().await;

        let all = list_conversations(&pool, None, 50).await.expect("tất cả");

        assert_eq!(all.len(), 2);
        let names: Vec<&str> = all.iter().map(|c| c.user_name.as_str()).collect();
        assert!(
            names.contains(&"Lễ tân A"),
            "thiếu tên người tạo: {names:?}"
        );
        assert!(
            names.contains(&"Lễ tân B"),
            "thiếu tên người tạo: {names:?}"
        );
    }

    #[tokio::test]
    async fn listing_is_newest_first_and_respects_the_limit() {
        let pool = seeded_pool().await;

        let all = list_conversations(&pool, None, 1)
            .await
            .expect("giới hạn 1");

        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "c2", "c2 có updated_at mới hơn");
    }

    #[tokio::test]
    async fn messages_come_back_oldest_first_within_the_limit() {
        let pool = seeded_pool().await;

        let messages = get_messages(&pool, "c1", 100).await.expect("tin nhắn");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].text, "câu hỏi");
        assert_eq!(messages[1].text, "câu trả lời");
    }

    /// Giới hạn cắt phần CŨ, giữ phần MỚI — cắt nhầm đầu kia thì trợ lý đọc
    /// được mở bài rồi mất hết phần vừa nói.
    #[tokio::test]
    async fn message_limit_keeps_the_newest_not_the_oldest() {
        let pool = seeded_pool().await;

        let messages = get_messages(&pool, "c1", 1).await.expect("giới hạn 1");

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].text, "câu trả lời", "phải giữ tin mới nhất");
    }

    #[tokio::test]
    async fn owner_lookup_returns_none_for_a_missing_conversation() {
        let pool = seeded_pool().await;

        assert_eq!(
            get_conversation_owner(&pool, "c1").await.expect("chủ c1"),
            Some("u1".to_string())
        );
        assert_eq!(
            get_conversation_owner(&pool, "khong-co")
                .await
                .expect("chủ id lạ"),
            None
        );
    }
}
