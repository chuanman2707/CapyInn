//! Chính sách của sổ hội thoại: đặt tên hội thoại và canh quyền sở hữu.
//!
//! Tầng này không có SQL. Nó đọc chủ hội thoại qua
//! `queries::assistant::conversation_queries` và ghi qua
//! `repositories::assistant::conversation_repository`; việc quyết định *có
//! được đọc hay không* thì chỉ nằm ở đây.
//!
//! Trần tải không được khai lại ở đây. Spec chốt "cứng ở tầng query", và
//! `conversation_queries::CONVERSATION_PAGE` / `MESSAGE_WINDOW` là nguồn sự
//! thật duy nhất cho hai con số ấy — chép lại xuống service là dựng nguồn thứ
//! hai để trôi lệch.

use sqlx::{Pool, Sqlite};

use crate::app_error::{codes, CommandError, CommandResult};
use crate::queries::assistant::conversation_queries;
use crate::repositories::assistant::conversation_repository;

const MAX_TITLE_CHARS: usize = 60;
const DEFAULT_TITLE: &str = "Hội thoại mới";

/// Cắt câu hỏi đầu tiên thành tên hội thoại.
///
/// Đếm theo **ký tự Unicode**, không phải byte: tiếng Việt đa byte nên
/// `&s[..60]` sẽ panic giữa ký tự. Cắt ở dấu cách cuối cùng trong 60 ký tự
/// đầu; nếu cả 60 ký tự không có dấu cách nào (một đoạn dán dài) thì cắt cứng
/// — thà tên xấu còn hơn tên rỗng.
pub fn derive_title(first_message: &str) -> String {
    let trimmed = first_message.trim();
    if trimmed.is_empty() {
        return DEFAULT_TITLE.to_string();
    }

    let chars: Vec<char> = trimmed.chars().collect();
    if chars.len() <= MAX_TITLE_CHARS {
        return trimmed.to_string();
    }

    let head: String = chars[..MAX_TITLE_CHARS].iter().collect();
    let cut = match head.rfind(' ') {
        Some(index) if index > 0 => head[..index].to_string(),
        _ => head,
    };

    let cut = cut.trim_end().to_string();
    if cut.is_empty() {
        return DEFAULT_TITLE.to_string();
    }

    format!("{cut}…")
}

fn forbidden() -> CommandError {
    // Cùng một câu cho "không có" và "của người khác": tiết lộ hội thoại đó có
    // tồn tại hay không cũng là tiết lộ.
    CommandError::user(codes::AUTH_FORBIDDEN, "Không mở được hội thoại này.")
}

pub async fn assert_can_read(
    pool: &Pool<Sqlite>,
    conversation_id: &str,
    user_id: &str,
    is_admin: bool,
) -> CommandResult<()> {
    let owner = conversation_queries::get_conversation_owner(pool, conversation_id)
        .await
        .map_err(|error| {
            CommandError::system(
                codes::SYSTEM_INTERNAL_ERROR,
                format!("Không đọc được chủ hội thoại: {error}"),
            )
        })?;

    match owner {
        Some(owner) if is_admin || owner == user_id => Ok(()),
        _ => Err(forbidden()),
    }
}

/// Trả về id hội thoại để ghi tiếp. `None` nghĩa là tạo mới; `Some` phải qua
/// cửa quyền sở hữu trước.
///
/// Đây là cửa duy nhất của module chưa có caller sản xuất — `commands::assistant`
/// gọi vào ở **Task 5** (Task 4 chỉ nối `assert_can_read`). Mọi thứ còn lại trong
/// file đều nằm dưới nó nên không cần đánh dấu riêng: rustc coi một item mang
/// `expect(dead_code)` là gốc và vẫn đi tiếp vào thân, nên `derive_title`,
/// `assert_can_read` và hai hàm query/repository nó gọi tới đã sống.
///
/// Dùng `expect` chứ không `allow` — nhưng đừng trông cậy quá vào nó: đã đo được
/// rằng `expect` chỉ bắn `this lint expectation is unfulfilled` khi chuỗi caller
/// có gốc là code sản xuất thật. Chừng nào `generate_handler!` chưa nối tới đây,
/// dấu này nằm im kể cả khi đã thừa. Xem đính chính đầy đủ ở đầu
/// `queries/assistant/conversation_queries.rs`.
#[cfg_attr(not(test), expect(dead_code))]
pub async fn ensure_conversation(
    pool: &Pool<Sqlite>,
    conversation_id: Option<&str>,
    user_id: &str,
    is_admin: bool,
    first_message: &str,
    now: &str,
) -> CommandResult<String> {
    if let Some(existing) = conversation_id {
        assert_can_read(pool, existing, user_id, is_admin).await?;
        return Ok(existing.to_string());
    }

    let id = uuid::Uuid::new_v4().to_string();
    conversation_repository::insert_conversation(
        pool,
        &id,
        user_id,
        &derive_title(first_message),
        now,
    )
    .await
    .map_err(|error| {
        CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            format!("Không tạo được hội thoại: {error}"),
        )
    })?;

    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    const NOW: &str = "2026-08-04T10:00:00+07:00";

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

    /// `c1` là của `u1`. `u2` là một lễ tân khác — người mà mọi test chặn rò
    /// bên dưới đóng vai.
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
             VALUES ('c1', 'u1', 'Hỏi phòng', ?, ?)",
        )
        .bind(NOW)
        .bind(NOW)
        .execute(&pool)
        .await
        .expect("chèn hội thoại");

        pool
    }

    /// Tiếng Việt đa byte: cắt theo byte sẽ panic giữa ký tự. Test này tồn tại
    /// để chặn `&s[..60]`.
    #[test]
    fn title_counts_unicode_characters_not_bytes() {
        let long = "à".repeat(200);

        let title = derive_title(&long);

        assert_eq!(title.chars().count(), 61, "60 ký tự + dấu …");
        assert!(title.ends_with('…'));
    }

    /// Test trên **không** ép ra được cái panic mà comment của `derive_title`
    /// cảnh báo: `"à"` toàn chuỗi thì byte thứ 60 rơi trúng biên ký tự, nên
    /// `&s[..60]` chỉ trả tên cụt chứ không nổ. Trộn một ký tự ASCII vào đầu là
    /// đẩy mọi biên lệch pha đi một byte — giờ byte 60 nằm giữa một `à`, đúng
    /// hình dạng đầu vào làm sập app ngoài đời (câu hỏi lẫn tiếng Anh, số phòng,
    /// tên khách viết không dấu).
    #[test]
    fn title_survives_a_cut_point_that_lands_mid_character() {
        let mixed = format!("a{}", "à".repeat(100));

        let title = derive_title(&mixed);

        assert_eq!(title.chars().count(), 61, "60 ký tự + dấu …");
        assert!(title.ends_with('…'));
    }

    #[test]
    fn title_cuts_at_the_last_space_within_sixty_characters() {
        let input = "Tối nay còn phòng nào trống không em, anh cần hai phòng đôi cho đoàn khách";

        let title = derive_title(input);

        assert!(title.ends_with('…'));
        assert!(
            !title.trim_end_matches('…').ends_with(' '),
            "không để dấu cách lủng lẳng"
        );
        assert!(title.chars().count() <= 61);
        assert!(title.starts_with("Tối nay còn phòng"));
    }

    /// Một đoạn dán dài không có dấu cách nào: phải cắt cứng, không được trả
    /// về rỗng.
    #[test]
    fn title_hard_cuts_when_there_is_no_space_at_all() {
        let input = "a".repeat(200);

        let title = derive_title(&input);

        assert_eq!(title.chars().count(), 61);
        assert!(title.starts_with("aaa"));
    }

    #[test]
    fn short_message_becomes_the_title_unchanged() {
        assert_eq!(
            derive_title("Phòng 201 trống chưa?"),
            "Phòng 201 trống chưa?"
        );
    }

    #[test]
    fn blank_message_falls_back_to_a_default_title() {
        assert_eq!(derive_title("   "), "Hội thoại mới");
    }

    /// Lọc ở danh sách là chưa đủ: ai biết một conversation_id là đọc được nội
    /// dung. Đường theo id phải tự kiểm.
    #[tokio::test]
    async fn reading_someone_elses_conversation_is_forbidden() {
        let pool = seeded_pool().await;

        let error = assert_can_read(&pool, "c1", "u2", false)
            .await
            .expect_err("người khác thì phải bị chặn");

        assert_eq!(error.code, codes::AUTH_FORBIDDEN);
    }

    /// Id không tồn tại trả cùng một lỗi với id của người khác — không tiết lộ
    /// hội thoại đó có tồn tại hay không.
    #[tokio::test]
    async fn a_missing_conversation_looks_the_same_as_someone_elses() {
        let pool = seeded_pool().await;

        let missing = assert_can_read(&pool, "khong-co", "u1", false)
            .await
            .expect_err("id lạ");
        let others = assert_can_read(&pool, "c1", "u2", false)
            .await
            .expect_err("của người khác");

        assert_eq!(missing.code, others.code);
        assert_eq!(missing.message, others.message);
    }

    #[tokio::test]
    async fn owner_and_admin_may_read() {
        let pool = seeded_pool().await;

        assert!(assert_can_read(&pool, "c1", "u1", false).await.is_ok());
        assert!(
            assert_can_read(&pool, "c1", "u2", true).await.is_ok(),
            "admin đọc được của mọi người"
        );
    }

    #[tokio::test]
    async fn ensure_conversation_creates_and_names_on_the_first_message() {
        let pool = seeded_pool().await;

        let id = ensure_conversation(
            &pool,
            None,
            "u1",
            false,
            "Tối nay còn phòng nào trống?",
            NOW,
        )
        .await
        .expect("tạo mới");

        let title: String =
            sqlx::query_scalar("SELECT title FROM assistant_conversations WHERE id = ?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .expect("đọc tên");
        assert_eq!(title, "Tối nay còn phòng nào trống?");

        // Vòng khứ hồi: hàng vừa ghi phải thuộc về ĐÚNG người gọi. Không có hai
        // dòng này thì `insert_conversation(pool, &id, "nguoi-la", …)` vẫn xanh
        // cả bộ — đường ghi duy nhất của task, mà không gì canh chủ sở hữu.
        // Ghi sai chủ thì chính người tạo mất quyền đọc, còn người kia thấy hội
        // thoại đó trong lịch sử, kèm tên khách và số CCCD bên trong.
        assert!(
            assert_can_read(&pool, &id, "u1", false).await.is_ok(),
            "người tạo phải đọc được hội thoại của chính mình"
        );
        assert!(
            assert_can_read(&pool, &id, "u2", false).await.is_err(),
            "người khác không được đọc"
        );
    }

    #[tokio::test]
    async fn ensure_conversation_reuses_an_existing_id_without_renaming() {
        let pool = seeded_pool().await;

        let id = ensure_conversation(&pool, Some("c1"), "u1", false, "câu thứ hai", NOW)
            .await
            .expect("dùng lại");

        assert_eq!(id, "c1");
        let title: String =
            sqlx::query_scalar("SELECT title FROM assistant_conversations WHERE id = 'c1'")
                .fetch_one(&pool)
                .await
                .expect("đọc tên");
        assert_eq!(title, "Hỏi phòng", "tên chỉ đặt một lần, từ câu đầu tiên");
    }

    #[tokio::test]
    async fn ensure_conversation_refuses_someone_elses_id() {
        let pool = seeded_pool().await;

        let error = ensure_conversation(&pool, Some("c1"), "u2", false, "chen ngang", NOW)
            .await
            .expect_err("không được ghi vào hội thoại người khác");

        assert_eq!(error.code, codes::AUTH_FORBIDDEN);
    }
}
