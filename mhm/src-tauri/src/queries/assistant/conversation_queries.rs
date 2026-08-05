//! Mọi lệnh đọc sổ hội thoại của trợ lý quầy.
//!
//! `list_conversations` nhận `ConversationScope` chứ không nhận `Option<&str>`.
//! Đường rộng quyền nhất — đọc hội thoại của *mọi* người, mà bên trong có tên
//! khách và số CCCD — nay phải **gõ ra tận nơi** là `EveryUser`. Nó không thể
//! tới bằng cách đánh rơi một giá trị (`.ok()`, `.and_then()`, một early-return
//! đặt nhầm), và nó grep được. Việc quyết định gọi đường nào nằm ở tầng command,
//! không nằm ở đây.
//!
//! Trần tải do chính module này giữ: `CONVERSATION_PAGE` và `MESSAGE_WINDOW`,
//! không có tham số `limit`. Spec chốt "cứng ở tầng query, không có giao diện
//! phân trang trong lần này", và kẹp một tham số thì vẫn để tầng trên truyền số
//! vào — bỏ hẳn mới làm việc vượt trần trở nên không diễn đạt được. Lý do cụ
//! thể: SQLite coi `LIMIT` âm là "không giới hạn" (đo được: `LIMIT -1` và
//! `LIMIT -5` trả về toàn bộ dòng), nên một số âm lọt xuống từ tầng trên là một
//! cú dump sạch sổ hội thoại, im lặng, không lỗi, không dấu vết.
//!
//! `queries` là module riêng của crate, nên tới khi
//! `services::assistant::conversation_service` gọi vào thì mọi thứ ở đây chỉ có
//! test là caller. Các `#[cfg_attr(not(test), expect(dead_code))]` bên dưới nói
//! đúng điều đó — và nói bằng `expect`, không phải `allow`: ngày Task 3 gọi vào,
//! chính compiler bắn `this lint expectation is unfulfilled` và CI đỏ, không ai
//! phải nhớ dọn. `cfg_attr(not(test), …)` là bắt buộc chứ không phải trang trí:
//! bản dựng test **đã có** caller sẵn (chính `mod tests` bên dưới), nên một
//! `#[expect(dead_code)]` trần làm `cargo clippy --all-targets -- -D warnings`
//! đỏ ngay hôm nay — đã đo, cả 10 chỗ.

use serde::Serialize;
use sqlx::{FromRow, Pool, Sqlite};

/// Danh sách lịch sử: 50 hội thoại mới nhất.
pub const CONVERSATION_PAGE: i64 = 50;

/// Mở một hội thoại: 100 tin nhắn mới nhất. `pub` để tầng trên lấy đúng con số
/// cho dòng "Chỉ 100 tin gần nhất được dùng làm ngữ cảnh" thay vì gõ lại 100.
pub const MESSAGE_WINDOW: i64 = 100;

/// Ai được thấy những hội thoại nào.
///
/// `EveryUser` là đường của admin. Nó là một biến thể phải gõ ra, không phải
/// một `None` rơi vào do sơ suất — đó là toàn bộ lý do kiểu này tồn tại.
#[cfg_attr(not(test), expect(dead_code))]
#[derive(Debug, Clone, Copy)]
pub enum ConversationScope<'a> {
    OwnedBy(&'a str),
    EveryUser,
}

#[cfg_attr(not(test), expect(dead_code))]
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct ConversationSummary {
    pub id: String,
    pub user_id: String,
    pub user_name: String,
    pub title: String,
    pub updated_at: String,
}

#[cfg_attr(not(test), expect(dead_code))]
#[derive(Debug, Clone, Serialize, FromRow)]
pub struct StoredMessage {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub created_at: String,
}

/// LEFT JOIN chứ không JOIN: tài khoản bị xoá để lại hội thoại mồ côi, và admin
/// vẫn phải thấy chúng để mà xoá.
///
/// Hai thân SQL rời nhau chứ không phải một câu với `WHERE (?1 IS NULL OR …)`.
/// Dạng gộp ấy vô hiệu hoá đúng cái index mà migration v27 dựng riêng cho đường
/// này — đo bằng `EXPLAIN QUERY PLAN`:
///
/// - gộp: `SCAN c USING INDEX …` + `USE TEMP B-TREE FOR ORDER BY`
/// - tách, đường `OwnedBy`: `SEARCH c USING INDEX … (user_id=?)`, hết sort
///
/// Chính sách lưu trữ là "không tự xoá", nên bảng chỉ lớn lên.
#[cfg_attr(not(test), expect(dead_code))]
pub async fn list_conversations(
    pool: &Pool<Sqlite>,
    scope: ConversationScope<'_>,
) -> Result<Vec<ConversationSummary>, sqlx::Error> {
    match scope {
        ConversationScope::OwnedBy(user_id) => {
            sqlx::query_as::<_, ConversationSummary>(
                "SELECT c.id            AS id,
                        c.user_id       AS user_id,
                        COALESCE(u.name, '(tài khoản đã xoá)') AS user_name,
                        c.title         AS title,
                        c.updated_at    AS updated_at
                 FROM assistant_conversations c
                 LEFT JOIN users u ON u.id = c.user_id
                 WHERE c.user_id = ?
                 ORDER BY c.updated_at DESC
                 LIMIT ?",
            )
            .bind(user_id)
            .bind(CONVERSATION_PAGE)
            .fetch_all(pool)
            .await
        }
        ConversationScope::EveryUser => {
            sqlx::query_as::<_, ConversationSummary>(
                "SELECT c.id            AS id,
                        c.user_id       AS user_id,
                        COALESCE(u.name, '(tài khoản đã xoá)') AS user_name,
                        c.title         AS title,
                        c.updated_at    AS updated_at
                 FROM assistant_conversations c
                 LEFT JOIN users u ON u.id = c.user_id
                 ORDER BY c.updated_at DESC
                 LIMIT ?",
            )
            .bind(CONVERSATION_PAGE)
            .fetch_all(pool)
            .await
        }
    }
}

#[cfg_attr(not(test), expect(dead_code))]
pub async fn get_conversation_owner(
    pool: &Pool<Sqlite>,
    id: &str,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT user_id FROM assistant_conversations WHERE id = ?")
        .bind(id)
        .fetch_optional(pool)
        .await
}

/// Lấy `MESSAGE_WINDOW` tin **mới nhất** rồi trả về theo thứ tự **cũ trước**.
/// Hai chiều khác nhau nên phải bọc: `ORDER BY … DESC LIMIT` để chọn đúng phần
/// đuôi, rồi đảo lại để đọc xuôi.
#[cfg_attr(not(test), expect(dead_code))]
pub async fn get_messages(
    pool: &Pool<Sqlite>,
    conversation_id: &str,
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
    .bind(MESSAGE_WINDOW)
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

    /// Nhãn thời gian tăng dần theo `index`; sắp chuỗi trùng với sắp số.
    fn stamp(index: i64) -> String {
        format!("2026-08-04T{:02}:{:02}:00+07:00", index / 60, index % 60)
    }

    /// `count` hội thoại của `u1`; hội thoại thứ `i` mới hơn hội thoại thứ `i-1`.
    async fn pool_with_many_conversations(count: i64) -> Pool<Sqlite> {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO users (id, name, pin_hash, role, active, created_at)
             VALUES ('u1', 'Lễ tân A', 'x', 'receptionist', 1, '2026-08-01T00:00:00+07:00')",
        )
        .execute(&pool)
        .await
        .expect("chèn user");

        for index in 0..count {
            sqlx::query(
                "INSERT INTO assistant_conversations (id, user_id, title, created_at, updated_at)
                 VALUES (?, 'u1', ?, ?, ?)",
            )
            .bind(format!("c{index:03}"))
            .bind(format!("Hội thoại {index:03}"))
            .bind(stamp(index))
            .bind(stamp(index))
            .execute(&pool)
            .await
            .expect("chèn hội thoại");
        }

        pool
    }

    /// Một hội thoại `c1` mang `count` tin nhắn; tin sau mới hơn tin trước.
    async fn pool_with_many_messages(count: i64) -> Pool<Sqlite> {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO assistant_conversations (id, user_id, title, created_at, updated_at)
             VALUES ('c1', 'u1', 'Hỏi phòng', ?, ?)",
        )
        .bind(stamp(0))
        .bind(stamp(0))
        .execute(&pool)
        .await
        .expect("chèn hội thoại");

        for index in 0..count {
            sqlx::query(
                "INSERT INTO assistant_messages (id, conversation_id, kind, text, created_at)
                 VALUES (?, 'c1', 'user', ?, ?)",
            )
            .bind(format!("m{index:03}"))
            .bind(format!("tin {index:03}"))
            .bind(stamp(index))
            .execute(&pool)
            .await
            .expect("chèn tin nhắn");
        }

        pool
    }

    /// Trần tải thuộc về tầng query, không thuộc về caller: sau khi bỏ tham số
    /// `limit` thì không còn cách nào diễn đạt việc vượt trần. Trước đây `limit`
    /// là tham số, mà SQLite coi `LIMIT` âm là "không giới hạn" — một số âm
    /// đánh rơi từ tầng trên là một cú dump sạch sổ hội thoại, im lặng.
    ///
    /// Cắt phần CŨ, giữ phần MỚI: cắt nhầm đầu kia thì lễ tân mở lịch sử ra chỉ
    /// thấy những hội thoại đã nguội.
    #[tokio::test]
    async fn the_conversation_ceiling_keeps_the_newest_page_and_drops_the_rest() {
        let pool = pool_with_many_conversations(60).await;

        let all = list_conversations(&pool, ConversationScope::EveryUser)
            .await
            .expect("liệt kê");

        assert_eq!(all.len(), CONVERSATION_PAGE as usize, "trần 50 phải giữ");
        assert_eq!(all[0].id, "c059", "dòng đầu là hội thoại mới nhất");
        assert_eq!(all[49].id, "c010", "cắt phần CŨ, giữ phần MỚI");
    }

    /// Hai thân SQL nay rời nhau, nên trần phải được canh ở **cả hai** — không
    /// thì một đường đánh mất `LIMIT` mà chẳng test nào thấy.
    #[tokio::test]
    async fn the_conversation_ceiling_holds_on_the_owned_by_path_too() {
        let pool = pool_with_many_conversations(60).await;

        let mine = list_conversations(&pool, ConversationScope::OwnedBy("u1"))
            .await
            .expect("của tôi");

        assert_eq!(mine.len(), CONVERSATION_PAGE as usize, "trần 50 phải giữ");
        assert_eq!(mine[0].id, "c059", "dòng đầu là hội thoại mới nhất");
    }

    /// Trần tin nhắn cắt phần CŨ, giữ phần MỚI — cắt nhầm đầu kia thì trợ lý
    /// đọc được mở bài rồi mất hết phần vừa nói.
    #[tokio::test]
    async fn the_message_ceiling_keeps_the_newest_window_and_drops_the_rest() {
        let pool = pool_with_many_messages(120).await;

        let messages = get_messages(&pool, "c1").await.expect("tin nhắn");

        assert_eq!(messages.len(), MESSAGE_WINDOW as usize, "trần 100 phải giữ");
        assert_eq!(messages[0].text, "tin 020", "cắt phần CŨ");
        assert_eq!(messages[99].text, "tin 119", "giữ tới tin mới nhất");
    }

    /// Lễ tân chỉ được thấy hội thoại của mình. Hội thoại chứa tên khách và
    /// CCCD, nên đây là luật chống rò chứ không phải luật tiện dụng.
    #[tokio::test]
    async fn listing_for_a_user_hides_everyone_else() {
        let pool = seeded_pool().await;

        let mine = list_conversations(&pool, ConversationScope::OwnedBy("u1"))
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

        let all = list_conversations(&pool, ConversationScope::EveryUser)
            .await
            .expect("tất cả");

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
    async fn listing_is_newest_first() {
        let pool = seeded_pool().await;

        let all = list_conversations(&pool, ConversationScope::EveryUser)
            .await
            .expect("tất cả");

        assert_eq!(all[0].id, "c2", "c2 có updated_at mới hơn");
        assert_eq!(all[1].id, "c1");
    }

    #[tokio::test]
    async fn messages_come_back_oldest_first() {
        let pool = seeded_pool().await;

        let messages = get_messages(&pool, "c1").await.expect("tin nhắn");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].text, "câu hỏi");
        assert_eq!(messages[1].text, "câu trả lời");
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
