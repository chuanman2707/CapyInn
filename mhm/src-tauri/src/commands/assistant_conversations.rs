//! Bốn lệnh cho sổ hội thoại của trợ lý quầy.
//!
//! Không một dòng SQL nào ở đây — `architecture_guard::COMMANDS_STILL_HOLDING_SQL`
//! là mảng rỗng và phải giữ nguyên như vậy. Đọc đi qua `queries`, xoá đi qua
//! `repositories`, còn chính sách "ai được đọc cái gì" nằm ở `services`.
//!
//! Ba luật của tầng này, cả ba đều có test canh ngay bên dưới:
//!
//! 1. **Danh tính chỉ đến từ `get_user(&state)`.** Không lệnh nào nhận `user_id`
//!    làm tham số. Frontend khai được mình là ai thì luật "chỉ thấy hội thoại
//!    của mình" thành đồ trang trí — mà hội thoại chứa tên khách và số CCCD.
//! 2. **Vỏ lệnh làm đúng một việc, và việc đó nằm trong một bảng hard-code.**
//!    Bốn `#[tauri::command]` dưới đây là vỏ một dòng gọi xuống lõi; lõi nhận
//!    `Option<User>` nên test hành vi gọi thẳng vào lõi và **không ai gọi vỏ**.
//!    Vỏ vì thế là chỗ duy nhất trong file sai được mà không test nào thấy —
//!    đo được: đổi vỏ xoá-một thành gọi `delete_every` thì cả 1213 test vẫn
//!    xanh. `every_command_shell_does_exactly_what_the_table_says` đóng cửa đó.
//! 3. **Không đẩy chuỗi lỗi sqlx thô ra frontend.** `appError/format.ts` nối
//!    thẳng `message` ra UI, nên `format!("…: {error}")` là
//!    `"no such table: assistant_conversations"` hiện nguyên văn trên màn hình
//!    quầy. Mọi sự cố hệ thống đi qua `log_system_error`: nguyên nhân gốc vào
//!    support log, ra ngoài chỉ còn câu chung kèm `support_id` tra được.
//!
//! Mỗi lệnh truyền **tên của chính nó** vào `log_system_error`, không dùng chung
//! một chuỗi — mã hỗ trợ lễ tân đọc lại phải chỉ đúng lệnh đã hỏng.

use super::{get_user, require_admin_user, AppState};
use crate::{
    app_error::{codes, log_system_error, AppErrorKind, CommandError, CommandResult},
    models::User,
    queries::assistant::conversation_queries::{
        self, ConversationScope, ConversationSummary, StoredMessage,
    },
    repositories::assistant::conversation_repository,
    services::assistant::conversation_service::assert_can_read,
};
use serde_json::{json, Value};
use sqlx::{Pool, Sqlite};
use tauri::State;

const LIST_COMMAND: &str = "list_assistant_conversations";
const GET_COMMAND: &str = "get_assistant_conversation";
const DELETE_COMMAND: &str = "delete_assistant_conversation";
const DELETE_ALL_COMMAND: &str = "delete_all_assistant_conversations";

/// Danh tính người gọi, rút từ phiên phía Rust. Chưa đăng nhập là lỗi, **không**
/// phải là "coi như admin" hay "coi như không lọc".
fn caller(user: Option<User>) -> CommandResult<(String, bool)> {
    let user =
        user.ok_or_else(|| CommandError::user(codes::AUTH_NOT_AUTHENTICATED, "Chưa đăng nhập"))?;
    let is_admin = user.role == "admin";
    Ok((user.id, is_admin))
}

/// `EveryUser` là đường của admin, và nó chỉ tới được từ đúng một chỗ: nhánh
/// `is_admin` gõ tay ở đây. Không nhánh `_`, không nhánh mặc định, không nhánh
/// khi thiếu dữ liệu — thiếu phiên thì `caller` đã trả lỗi trước khi tới đây.
fn scope_for(user_id: &str, is_admin: bool) -> ConversationScope<'_> {
    if is_admin {
        ConversationScope::EveryUser
    } else {
        ConversationScope::OwnedBy(user_id)
    }
}

/// Sự cố sqlx ở tầng này: ghi nguyên nhân gốc vào support log, trả ra ngoài câu
/// chung kèm `support_id`.
fn system_failure(command_name: &str, root_cause: &sqlx::Error, context: Value) -> CommandError {
    log_system_error(command_name, root_cause.to_string(), context)
}

/// Bọc lỗi do `services::assistant::conversation_service` dựng lên.
///
/// Lỗi *người dùng* (từ chối đọc) đi thẳng ra **nguyên vẹn** — cùng một câu cho
/// "không tồn tại" và "của người khác" (spec dòng 313-314), và không được thêm
/// nhánh phân biệt nào ở đây. Chỉ lỗi *hệ thống* mới bị bọc lại, vì tầng service
/// dựng nó bằng `format!("…: {error}")` — tức chuỗi sqlx thô — còn tầng service
/// thì không biết tên lệnh để ghi support log.
///
/// **Cho Task 5:** `conversation_service.rs` có **HAI** chỗ dựng
/// `CommandError::system(format!(…))`, không phải một:
///
/// - `assert_can_read` (khoảng dòng 68-71), và
/// - `insert_conversation` bên trong `ensure_conversation` (khoảng dòng 117-122).
///
/// Task 5 gọi `ensure_conversation`, nên nó dính **cả hai**; bản vá của Task 4
/// mới chỉ che chỗ thứ nhất. Hàm này `pub(crate)` chính là để Task 5 dùng lại
/// chứ đừng chép sang `commands/assistant.rs` — hai bản sao là hai chỗ để trôi
/// lệch.
pub(crate) fn wrap_service_system_error(
    command_name: &str,
    error: CommandError,
    context: Value,
) -> CommandError {
    match error.kind {
        AppErrorKind::System => log_system_error(command_name, &error.message, context),
        AppErrorKind::User => error,
    }
}

fn conversation_context(conversation_id: &str) -> Value {
    json!({ "conversation_id": conversation_id })
}

async fn list_for(
    pool: &Pool<Sqlite>,
    user: Option<User>,
) -> CommandResult<Vec<ConversationSummary>> {
    let (user_id, is_admin) = caller(user)?;

    // `user_id` là id **nhân viên**, không phải PII khách — `commands/auth.rs:105`
    // đã ghi cả `name` lẫn `role`. Thiếu nó thì lễ tân đọc mã hỗ trợ lên mà
    // support vẫn không biết ai, ca nào. Tuyệt đối không thêm tên khách, số CCCD
    // hay nội dung tin nhắn vào đây.
    conversation_queries::list_conversations(pool, scope_for(&user_id, is_admin))
        .await
        .map_err(|error| {
            system_failure(
                LIST_COMMAND,
                &error,
                json!({ "user_id": &user_id, "is_admin": is_admin }),
            )
        })
}

async fn messages_for(
    pool: &Pool<Sqlite>,
    user: Option<User>,
    conversation_id: &str,
) -> CommandResult<Vec<StoredMessage>> {
    let (user_id, is_admin) = caller(user)?;
    assert_can_read(pool, conversation_id, &user_id, is_admin)
        .await
        .map_err(|error| {
            wrap_service_system_error(GET_COMMAND, error, conversation_context(conversation_id))
        })?;

    conversation_queries::get_messages(pool, conversation_id)
        .await
        .map_err(|error| system_failure(GET_COMMAND, &error, conversation_context(conversation_id)))
}

async fn delete_one(
    pool: &Pool<Sqlite>,
    user: Option<User>,
    conversation_id: &str,
) -> CommandResult<u64> {
    let _admin = require_admin_user(user)?;

    conversation_repository::delete_conversation(pool, conversation_id)
        .await
        .map_err(|error| {
            system_failure(
                DELETE_COMMAND,
                &error,
                conversation_context(conversation_id),
            )
        })
}

async fn delete_every(pool: &Pool<Sqlite>, user: Option<User>) -> CommandResult<u64> {
    let _admin = require_admin_user(user)?;

    conversation_repository::delete_all_conversations(pool)
        .await
        .map_err(|error| system_failure(DELETE_ALL_COMMAND, &error, json!({})))
}

/// Lễ tân nhận hội thoại của chính mình; admin nhận của mọi người.
#[tauri::command]
pub async fn list_assistant_conversations(
    state: State<'_, AppState>,
) -> CommandResult<Vec<ConversationSummary>> {
    list_for(&state.db, get_user(&state)).await
}

/// Lọc ở danh sách là chưa đủ: ai biết một `conversation_id` là đọc được nội
/// dung, nên đường theo id tự kiểm quyền qua `assert_can_read`.
#[tauri::command]
pub async fn get_assistant_conversation(
    state: State<'_, AppState>,
    conversation_id: String,
) -> CommandResult<Vec<StoredMessage>> {
    messages_for(&state.db, get_user(&state), &conversation_id).await
}

/// Xoá là quyền admin.
#[tauri::command]
pub async fn delete_assistant_conversation(
    state: State<'_, AppState>,
    conversation_id: String,
) -> CommandResult<u64> {
    delete_one(&state.db, get_user(&state), &conversation_id).await
}

/// Xoá sạch sổ hội thoại — quyền admin.
#[tauri::command]
pub async fn delete_all_assistant_conversations(state: State<'_, AppState>) -> CommandResult<u64> {
    delete_every(&state.db, get_user(&state)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_error::GENERIC_SYSTEM_ERROR_MESSAGE;
    use sqlx::sqlite::SqlitePoolOptions;

    fn user(id: &str, role: &str) -> User {
        User {
            id: id.to_string(),
            name: format!("Nhân viên {id}"),
            role: role.to_string(),
            active: true,
            created_at: "2026-08-01T00:00:00+07:00".to_string(),
        }
    }

    fn receptionist(id: &str) -> Option<User> {
        Some(user(id, "receptionist"))
    }

    fn admin(id: &str) -> Option<User> {
        Some(user(id, "admin"))
    }

    /// Pool KHÔNG chạy migration: mọi câu SQL vào sổ hội thoại đều nổ
    /// `no such table`. Đây là hình dạng sự cố mà vòng duyệt Task 3 đo được đã
    /// hiện nguyên văn ra màn hình lễ tân.
    async fn unmigrated_pool() -> Pool<Sqlite> {
        let database_url = format!(
            "sqlite://file:{}?mode=memory&cache=shared",
            uuid::Uuid::new_v4()
        );
        SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .expect("mở pool test")
    }

    async fn test_pool() -> Pool<Sqlite> {
        let pool = unmigrated_pool().await;
        crate::db::run_migrations(&pool).await.expect("migration");
        pool
    }

    /// `c1` của `u1`, `c2` của `u2`. Admin `a1` không sở hữu hội thoại nào —
    /// nên nếu admin thấy cả hai, đó chỉ có thể là nhánh `EveryUser`.
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
             VALUES ('m1', 'c1', 'user', 'Khách Nguyễn Văn A, CCCD 001', '2026-08-04T10:00:00+07:00')",
        )
        .execute(&pool)
        .await
        .expect("chèn tin nhắn");

        pool
    }

    // ─── Danh sách: phạm vi theo vai, không theo tham số ───

    /// Bất biến đắt nhất của Task 4. Hội thoại chứa tên khách và số CCCD, nên
    /// một lễ tân thấy sổ của người khác là rò dữ liệu khách, không phải lỗi
    /// tiện dụng.
    #[tokio::test]
    async fn a_receptionist_only_ever_lists_their_own_conversations() {
        let pool = seeded_pool().await;

        let mine = list_for(&pool, receptionist("u1")).await.expect("của tôi");

        assert_eq!(mine.len(), 1, "lễ tân chỉ được thấy sổ của chính mình");
        assert_eq!(mine[0].id, "c1");
    }

    /// `a1` không sở hữu hội thoại nào; thấy cả hai nghĩa là đã đi nhánh
    /// `EveryUser` — nhánh phải được gõ ra có chủ ý.
    #[tokio::test]
    async fn an_admin_lists_every_users_conversations() {
        let pool = seeded_pool().await;

        let all = list_for(&pool, admin("a1")).await.expect("tất cả");

        assert_eq!(all.len(), 2, "admin nhận sổ của mọi người");
    }

    #[tokio::test]
    async fn listing_without_a_session_is_refused() {
        let pool = seeded_pool().await;

        let error = list_for(&pool, None).await.expect_err("chưa đăng nhập");

        assert_eq!(error.code, codes::AUTH_NOT_AUTHENTICATED);
    }

    // ─── Mở một hội thoại ───

    #[tokio::test]
    async fn the_owner_and_an_admin_can_open_a_conversation() {
        let pool = seeded_pool().await;

        assert_eq!(
            messages_for(&pool, receptionist("u1"), "c1")
                .await
                .expect("chủ đọc được")
                .len(),
            1
        );
        assert_eq!(
            messages_for(&pool, admin("a1"), "c1")
                .await
                .expect("admin đọc được")
                .len(),
            1
        );
    }

    /// Lọc ở danh sách là chưa đủ: ai biết một `conversation_id` là đọc được nội
    /// dung. Và "không tồn tại" phải giống hệt "của người khác" — tiết lộ hội
    /// thoại đó có tồn tại hay không cũng là tiết lộ (spec dòng 313-314).
    #[tokio::test]
    async fn opening_someone_elses_conversation_looks_exactly_like_a_missing_one() {
        let pool = seeded_pool().await;

        let others = messages_for(&pool, receptionist("u2"), "c1")
            .await
            .expect_err("của người khác");
        let missing = messages_for(&pool, receptionist("u1"), "khong-co")
            .await
            .expect_err("id lạ");

        assert_eq!(others.code, codes::AUTH_FORBIDDEN);
        assert_eq!(others.code, missing.code);
        assert_eq!(others.message, missing.message);
    }

    #[tokio::test]
    async fn opening_a_conversation_without_a_session_is_refused() {
        let pool = seeded_pool().await;

        let error = messages_for(&pool, None, "c1")
            .await
            .expect_err("chưa đăng nhập");

        assert_eq!(error.code, codes::AUTH_NOT_AUTHENTICATED);
    }

    // ─── Xoá: quyền admin ───

    /// Xoá là quyền admin (spec, bảng quyền dòng 280-290). Kiểm cả hàng còn
    /// nguyên: một lệnh xoá vừa trả lỗi vừa xoá thật là hỏng kiểu tệ nhất.
    #[tokio::test]
    async fn deleting_one_conversation_is_admin_only() {
        let pool = seeded_pool().await;

        let error = delete_one(&pool, receptionist("u1"), "c1")
            .await
            .expect_err("lễ tân không được xoá");
        assert_eq!(error.code, codes::AUTH_FORBIDDEN);

        let still_there: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM assistant_conversations WHERE id = 'c1'")
                .fetch_one(&pool)
                .await
                .expect("đếm lại");
        assert_eq!(still_there, 1, "bị từ chối thì không được xoá gì");

        let removed = delete_one(&pool, admin("a1"), "c1")
            .await
            .expect("admin xoá được");
        assert_eq!(removed, 1);
    }

    #[tokio::test]
    async fn deleting_one_conversation_without_a_session_is_refused() {
        let pool = seeded_pool().await;

        let error = delete_one(&pool, None, "c1")
            .await
            .expect_err("chưa đăng nhập");

        assert_eq!(error.code, codes::AUTH_NOT_AUTHENTICATED);
    }

    #[tokio::test]
    async fn deleting_every_conversation_is_admin_only() {
        let pool = seeded_pool().await;

        let error = delete_every(&pool, receptionist("u1"))
            .await
            .expect_err("lễ tân không được xoá sạch");
        assert_eq!(error.code, codes::AUTH_FORBIDDEN);

        let still_there: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM assistant_conversations")
            .fetch_one(&pool)
            .await
            .expect("đếm lại");
        assert_eq!(still_there, 2, "bị từ chối thì không được xoá gì");

        let removed = delete_every(&pool, admin("a1")).await.expect("admin xoá");
        assert_eq!(removed, 2);
    }

    // ─── Lỗi hệ thống: chuỗi sqlx không được ra tới frontend ───

    /// `mhm/src/lib/appError/format.ts:26` nối thẳng `message` ra UI, nên một
    /// `format!("…: {error}")` là "no such table: assistant_conversations" hiện
    /// nguyên văn trên màn hình quầy. Cả bốn đường phải trả câu chung kèm
    /// `support_id` tra được trong support log.
    // `env_lock` là `std::sync::Mutex` và phải giữ suốt cả test, không thì test
    // song song khác đổi `CAPYINN_RUNTIME_ROOT` ngay giữa chừng. Đây là khuôn có
    // sẵn trong repo — xem `agent/supervisor.rs`.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn a_database_failure_never_hands_the_raw_sqlx_message_to_the_frontend() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();
        let runtime_root = std::env::temp_dir().join(format!(
            "capyinn-assistant-conversations-{}",
            uuid::Uuid::new_v4()
        ));
        std::env::set_var("CAPYINN_RUNTIME_ROOT", &runtime_root);

        let pool = unmigrated_pool().await;
        let failures = vec![
            (
                "list_assistant_conversations",
                list_for(&pool, receptionist("u1")).await.err(),
            ),
            (
                "get_assistant_conversation",
                messages_for(&pool, receptionist("u1"), "c1").await.err(),
            ),
            (
                "delete_assistant_conversation",
                delete_one(&pool, admin("a1"), "c1").await.err(),
            ),
            (
                "delete_all_assistant_conversations",
                delete_every(&pool, admin("a1")).await.err(),
            ),
        ];

        std::env::remove_var("CAPYINN_RUNTIME_ROOT");

        for (command_name, error) in &failures {
            let error = error.as_ref().expect("bảng không tồn tại thì phải lỗi");
            assert_eq!(
                error.message, GENERIC_SYSTEM_ERROR_MESSAGE,
                "{command_name} đẩy chuỗi lạ ra frontend: {}",
                error.message
            );
            assert!(
                !error.message.contains("no such table"),
                "{command_name} rò chuỗi sqlx thô: {}",
                error.message
            );
            assert!(error.support_id.is_some(), "{command_name} thiếu mã hỗ trợ");
        }

        let log = std::fs::read_to_string(
            runtime_root
                .join("diagnostics")
                .join("support-errors.jsonl"),
        )
        .expect("support log phải được ghi");
        for (command_name, error) in &failures {
            let support_id = error
                .as_ref()
                .expect("lỗi")
                .support_id
                .as_deref()
                .expect("mã hỗ trợ");
            assert!(
                log.contains(support_id),
                "{command_name}: mã {support_id} không tra được trong support log"
            );
            assert!(
                log.contains(*command_name),
                "{command_name} phải tự khai tên mình, không dùng chung một chuỗi"
            );
        }
        assert!(
            log.contains("no such table"),
            "nguyên nhân gốc phải nằm trong support log, chỉ là không ra tới frontend"
        );

        let _ = std::fs::remove_dir_all(&runtime_root);
    }

    // ─── Hình dạng chữ ký: danh tính không bao giờ tới từ frontend ───

    /// Một vỏ `#[tauri::command]` đọc ra từ mã nguồn: tên, tên các tham số, và
    /// các dòng thân hàm (đã `trim`, bỏ dòng trống).
    #[derive(Debug)]
    struct CommandShell {
        name: String,
        parameters: Vec<String>,
        body: Vec<String>,
    }

    /// Đọc mọi `#[tauri::command]` trong một file.
    ///
    /// Dựa vào rustfmt ở hai chỗ, cả hai đều có meta-test bên dưới: chữ ký kết
    /// thúc ở dòng đầu tiên kết thúc bằng `{`, và thân hàm cấp cao nhất đóng
    /// bằng một dòng `}` ở cột 0 (nên `}` thụt lề của closure không cắt nhầm).
    ///
    /// `starts_with` chứ **không** so bằng `==`: `#[tauri::command(rename_all =
    /// "snake_case")]` cũng là một cửa IPC thật y hệt. Đo được với bản so bằng:
    /// thêm một lệnh thứ năm khai biến thể đó, mang cả `user_id: String` lẫn
    /// `is_admin: bool`, thì nó lọt hẳn khỏi bộ đọc — số vỏ đọc được vẫn là 4
    /// nên guard vẫn xanh.
    fn commands_in(source: &str) -> Vec<CommandShell> {
        let mut shells = Vec::new();
        let mut lines = source.lines();

        while let Some(line) = lines.next() {
            if !line.trim().starts_with("#[tauri::command") {
                continue;
            }

            let mut signature = String::new();
            for line in lines.by_ref() {
                signature.push_str(line.trim());
                signature.push(' ');
                if line.trim_end().ends_with('{') {
                    break;
                }
            }

            let mut body = Vec::new();
            for line in lines.by_ref() {
                if line == "}" {
                    break;
                }
                if !line.trim().is_empty() {
                    body.push(line.trim().to_string());
                }
            }

            shells.push(CommandShell {
                name: function_name(&signature),
                parameters: parameter_names(&signature),
                body,
            });
        }

        shells
    }

    fn function_name(signature: &str) -> String {
        signature
            .split_once("fn ")
            .map(|(_, rest)| {
                rest.split('(')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            })
            .unwrap_or_default()
    }

    /// Tên các tham số của một chữ ký. Tách ở dấu phẩy **ngoài cùng**: dấu phẩy
    /// trong `State<'_, AppState>` nằm trong ngoặc nhọn nên không được tính, và
    /// tên là phần trước dấu `:` đầu tiên của mỗi tham số.
    fn parameter_names(signature: &str) -> Vec<String> {
        let Some(open) = signature.find('(') else {
            return Vec::new();
        };

        let mut parameters = Vec::new();
        let mut current = String::new();
        let mut parens = 0usize;
        let mut angles = 0usize;

        for character in signature[open + 1..].chars() {
            match character {
                ')' if parens == 0 => break,
                '(' | '[' => {
                    parens += 1;
                    current.push(character);
                }
                ')' | ']' => {
                    parens = parens.saturating_sub(1);
                    current.push(character);
                }
                '<' => {
                    angles += 1;
                    current.push(character);
                }
                '>' => {
                    angles = angles.saturating_sub(1);
                    current.push(character);
                }
                ',' if parens == 0 && angles == 0 => parameters.push(std::mem::take(&mut current)),
                _ => current.push(character),
            }
        }
        parameters.push(current);

        parameters
            .iter()
            .map(|parameter| parameter.trim())
            .filter(|parameter| !parameter.is_empty())
            .map(|parameter| {
                parameter
                    .split(':')
                    .next()
                    .unwrap_or(parameter)
                    .trim()
                    .to_string()
            })
            .collect()
    }

    /// Allowlist, **không** phải blocklist. `state` là nơi danh tính đi vào —
    /// phía Rust; `conversation_id` là một id, không phải một lời khai về mình.
    ///
    /// Bản cũ là blocklist 5 tên (`user_id`, `user_name`, `is_admin`, `role`,
    /// `scope`). Đo được: đổi tên tham số thành `actor: String` rồi tự dựng
    /// `User { role: "admin", … }` từ chuỗi đó thì cả 12 test của module vẫn
    /// xanh, kể cả chính test canh danh tính — blocklist canh *cách gõ tên*, và
    /// `actor`/`uid`/`caller_id`/`owner` đi lọt hết. Allowlist không cần biết
    /// `actor` nghĩa là gì để từ chối nó.
    const PARAMETERS_A_COMMAND_MAY_TAKE: [&str; 2] = ["state", "conversation_id"];

    /// Thân của bốn vỏ, chép nguyên văn từ trên đĩa.
    ///
    /// Bảng này là dữ liệu hard-code và đó là **chủ đích**: sửa vỏ mà quên sửa
    /// bảng thì đỏ, nên ai đổi vỏ buộc phải nhìn thấy bảng. Nó chặn ba cú mà
    /// không test hành vi nào chặn được, vì không test nào gọi vỏ: gọi nhầm lõi
    /// (`delete_one` → `delete_every`), dựng `User` bằng tay thay cho
    /// `get_user(&state)`, và bỏ hẳn `get_user(&state)`.
    const WHAT_EACH_SHELL_MUST_DO: [(&str, &str); 4] = [
        (
            "list_assistant_conversations",
            "list_for(&state.db, get_user(&state)).await",
        ),
        (
            "get_assistant_conversation",
            "messages_for(&state.db, get_user(&state), &conversation_id).await",
        ),
        (
            "delete_assistant_conversation",
            "delete_one(&state.db, get_user(&state), &conversation_id).await",
        ),
        (
            "delete_all_assistant_conversations",
            "delete_every(&state.db, get_user(&state)).await",
        ),
    ];

    /// Số vỏ đọc được phải khớp bảng. Một bộ đọc đọc hụt là một guard xanh giả,
    /// nên cả hai hàng rào dưới đây đều bắt đầu bằng khẳng định này.
    fn shells_of_this_file() -> Vec<CommandShell> {
        let shells = commands_in(include_str!("assistant_conversations.rs"));

        assert_eq!(
            shells.len(),
            WHAT_EACH_SHELL_MUST_DO.len(),
            "đọc hụt (hoặc đọc dư) vỏ lệnh — bộ đọc hỏng thì cửa này thành cửa \
             mở: {:?}",
            shells.iter().map(|shell| &shell.name).collect::<Vec<_>>()
        );

        shells
    }

    /// Hàng rào 1 — cửa IPC. Test hành vi bên trên chạy qua `list_for`/
    /// `messages_for`, tức chúng canh phần *bên trong* lệnh. Chúng vẫn xanh nếu
    /// ai đó thêm một tham số danh tính vào chữ ký lệnh rồi nhét thẳng xuống —
    /// mà đó chính là cú làm luật "chỉ thấy hội thoại của mình" thành đồ trang
    /// trí. Test này canh nửa còn lại.
    #[test]
    fn no_command_here_takes_an_identity_from_the_frontend() {
        let shells = shells_of_this_file();

        let mut violations = Vec::new();
        for shell in &shells {
            for parameter in &shell.parameters {
                if !PARAMETERS_A_COMMAND_MAY_TAKE.contains(&parameter.as_str()) {
                    violations.push(format!(
                        "`{}` nhận `{parameter}` — ngoài allowlist \
                         {PARAMETERS_A_COMMAND_MAY_TAKE:?}",
                        shell.name
                    ));
                }
            }
        }

        assert!(
            violations.is_empty(),
            "danh tính phải lấy từ `get_user(&state)` phía Rust. Frontend truyền \
             lên được thì luật \"chỉ thấy hội thoại của mình\" thành đồ trang \
             trí — mà hội thoại chứa tên khách và số CCCD.\n\n{}",
            violations.join("\n")
        );
    }

    /// Hàng rào 2 — thân vỏ. Lõi nhận `Option<User>` nên mọi test hành vi gọi
    /// thẳng vào lõi và **không ai gọi vỏ**; vỏ là chỗ duy nhất trong file sai
    /// được mà không ai canh. Đo được: đổi vỏ `delete_assistant_conversation`
    /// gọi `delete_every` — "xoá một hội thoại" thành "xoá sạch cả sổ" — mà
    /// 1213/1213 test vẫn xanh.
    #[test]
    fn every_command_shell_does_exactly_what_the_table_says() {
        let shells = shells_of_this_file();

        let mut violations = Vec::new();
        for (name, expected_body) in WHAT_EACH_SHELL_MUST_DO {
            let Some(shell) = shells.iter().find(|shell| shell.name == name) else {
                violations.push(format!("thiếu hẳn vỏ `{name}`"));
                continue;
            };
            if shell.body != [expected_body] {
                violations.push(format!(
                    "`{name}`\n  bảng nói: {expected_body}\n  trên đĩa: {:?}",
                    shell.body
                ));
            }
        }

        assert!(
            violations.is_empty(),
            "thân vỏ lệnh lệch khỏi bảng mong đợi. Nếu bạn đổi vỏ có chủ ý thì \
             sửa `WHAT_EACH_SHELL_MUST_DO` cho khớp — bảng cố tình bắt bạn nhìn \
             thấy nó.\n\n{}",
            violations.join("\n")
        );
    }

    /// Canh chính bộ đọc: một guard đọc hụt là một guard xanh giả.
    #[test]
    fn the_signature_reader_handles_both_layouts_rustfmt_produces() {
        let multi_line = "#[tauri::command]\npub async fn f(\n    state: State<'_, AppState>,\n    user_id: String,\n) -> CommandResult<u64> {\n    Ok(0)\n}\n";
        let read = commands_in(multi_line);
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].name, "f");
        assert_eq!(read[0].parameters, ["state", "user_id"]);
        assert_eq!(read[0].body, ["Ok(0)"]);

        let single_line =
            "#[tauri::command]\npub async fn f(state: State<'_, AppState>) -> CommandResult<u64> {\n    Ok(0)\n}\n";
        let read = commands_in(single_line);
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].parameters, ["state"]);
        assert_eq!(read[0].body, ["Ok(0)"]);

        // Thân hàm không được lẫn vào chữ ký, không thì guard báo động giả và
        // sẽ bị ai đó tắt đi.
        let body_mentions_it = "#[tauri::command]\npub async fn f(state: State<'_, AppState>) -> CommandResult<u64> {\n    let user_id = caller(state)?;\n    Ok(0)\n}\n";
        let read = commands_in(body_mentions_it);
        assert_eq!(read[0].parameters, ["state"]);
        assert_eq!(read[0].body, ["let user_id = caller(state)?;", "Ok(0)"]);

        // `#[tauri::command(rename_all = "…")]` là cửa IPC y hệt. Bản so bằng
        // `==` để lọt biến thể này, và một lệnh lọt là một lệnh không ai canh.
        let renamed = "#[tauri::command(rename_all = \"snake_case\")]\npub async fn f(state: State<'_, AppState>, user_id: String) -> CommandResult<u64> {\n    Ok(0)\n}\n";
        let read = commands_in(renamed);
        assert_eq!(read.len(), 1, "biến thể có tham số cũng phải đọc được");
        assert_eq!(read[0].parameters, ["state", "user_id"]);

        // `}` thụt lề của closure không được cắt thân hàm giữa chừng.
        let closure_inside = "#[tauri::command]\npub async fn f(state: State<'_, AppState>) -> CommandResult<u64> {\n    go().map_err(|error| {\n        wrap(error)\n    })\n}\n";
        assert_eq!(
            commands_in(closure_inside)[0].body,
            ["go().map_err(|error| {", "wrap(error)", "})"]
        );

        assert!(commands_in("fn plain() {}\n").is_empty());
    }
}
