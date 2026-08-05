//! Lệnh của trợ lý quầy, gồm cả lượt chat và việc ghi lượt ấy vào sổ hội thoại.
//!
//! Không một dòng SQL nào ở đây — `architecture_guard::COMMANDS_STILL_HOLDING_SQL`
//! là mảng rỗng và phải giữ nguyên như vậy.
//!
//! **Việc ghi sổ nằm ở tầng này, không nằm trong `run_assistant_turn`.**
//! `agent/assistant/mod.rs` đã cầm sẵn `pool` nên là chỗ tiện tay nhất để nhét
//! lệnh ghi vào — mà làm thế thì chuỗi thành `command → agent → services`, lệch
//! khỏi chiều đã chốt, và `architecture_guard` **không bắt được** (nó chỉ cấm
//! tầng trong import `commands`). Thêm nữa, mã sản xuất trong `agent/assistant/`
//! hiện không chạy một câu SQL nào; đặt lệnh ghi vào đó là lần đầu tiên phá tính
//! chất ấy.
//!
//! Thứ tự trong một lượt là bắt buộc, không phải tuỳ ý: **kiểm rỗng và 2000 ký
//! tự → kiểm quyền sở hữu → ghi câu hỏi → gọi nhà cung cấp → ghi câu trả lời →
//! nhấc `updated_at`**. Xem `open_turn_record` và `close_turn_record`.

use super::{get_user, require_admin_user, AppState};
use crate::{
    agent::{
        assistant::{
            config::{
                evaluate_assistant_gate, get_assistant_cloud_data_opt_in, get_assistant_config,
                resolve_assistant_api_key_present, save_assistant_config,
                set_assistant_api_key_present, set_assistant_cloud_data_opt_in,
                validate_assistant_base_url, validate_assistant_model, AssistantConfig,
                AssistantGateStatus, AssistantPreset,
            },
            draft::ProposedAction,
            provider::{build_assistant_provider_client, AssistantProviderClient},
            run_assistant_turn, AssistantTurnRequest, AssistantTurnResponse, MAX_MESSAGE_CHARS,
        },
        secrets::{AgentSecretKind, AgentSecretStore, KeychainSecretStore},
    },
    app_error::{codes, CommandError, CommandResult},
    commands::assistant_conversations::wrap_service_system_error,
    models::User,
    repositories::assistant::conversation_repository,
    services::assistant::conversation_service::{
        assert_can_read, record_turn_question, TurnRecord,
    },
};
use serde::Serialize;
use serde_json::json;
use sqlx::{Pool, Sqlite};
use tauri::State;

const TURN_COMMAND: &str = "assistant_turn";

#[derive(Debug, Clone, Serialize)]
pub struct AssistantSettings {
    pub config: AssistantConfig,
    pub has_api_key: bool,
    pub cloud_data_opt_in: bool,
    pub gate: AssistantGateStatus,
}

fn secret_store() -> KeychainSecretStore {
    KeychainSecretStore
}

/// Chạy mỗi lần mở app (`MainShell` gọi `refreshAssistantSettings` khi mount),
/// nên **không được đọc keychain**. Xem `resolve_assistant_api_key_present`:
/// câu trả lời nằm trong database, keychain chỉ bị đụng đúng một lần trên máy
/// đã nhập khoá từ trước bản vá này.
async fn load_settings(state: &State<'_, AppState>) -> CommandResult<AssistantSettings> {
    let config = get_assistant_config(&state.db).await?;
    let cloud_data_opt_in = get_assistant_cloud_data_opt_in(&state.db).await?;
    let has_api_key = resolve_assistant_api_key_present(&state.db, &secret_store()).await?;
    let gate = evaluate_assistant_gate(&config, has_api_key, cloud_data_opt_in);

    Ok(AssistantSettings {
        config,
        has_api_key,
        cloud_data_opt_in,
        gate,
    })
}

/// Đọc được cho mọi người đăng nhập: frontend cần biết có nên hiện panel không.
///
/// Trả về `gate`, và `gate.ready` là thứ quyết định panel hiện hay không. Chưa
/// đăng nhập thì lệnh lỗi, `useAssistantStore.refreshSettings` bắt lỗi và đặt
/// `settings: null`, nên nút "Trợ lý" lẫn panel đều tự tắt — kể cả khi có ai
/// đó gọi thẳng IPC mà không đi qua shell.
#[tauri::command]
pub async fn get_assistant_settings(
    state: State<'_, AppState>,
) -> CommandResult<AssistantSettings> {
    if get_user(&state).is_none() {
        return Err(CommandError::user(
            codes::AUTH_NOT_AUTHENTICATED,
            "Chưa đăng nhập",
        ));
    }
    load_settings(&state).await
}

#[tauri::command]
pub async fn set_assistant_settings(
    state: State<'_, AppState>,
    preset: AssistantPreset,
    base_url: String,
    model: String,
) -> CommandResult<AssistantSettings> {
    let _admin = require_admin_user(get_user(&state))?;

    let config = AssistantConfig {
        preset,
        base_url: validate_assistant_base_url(&base_url)?,
        model: validate_assistant_model(&model)?,
    };
    save_assistant_config(&state.db, &config).await?;

    load_settings(&state).await
}

#[tauri::command]
pub async fn set_assistant_api_key(
    state: State<'_, AppState>,
    api_key: String,
) -> CommandResult<AssistantSettings> {
    let _admin = require_admin_user(get_user(&state))?;

    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return Err(CommandError::user(
            codes::VALIDATION_INVALID_INPUT,
            "Chưa nhập khoá API.",
        ));
    }
    secret_store().set_secret(AgentSecretKind::AssistantApiKey, trimmed)?;
    // Ghi cờ SAU khi keychain nhận khoá. Ngược lại là nói dối: cờ báo có khoá
    // trong khi keychain từ chối lưu, và trợ lý sẽ hiện panel rồi chết ở lượt
    // chat đầu tiên.
    set_assistant_api_key_present(&state.db, true).await?;

    load_settings(&state).await
}

#[tauri::command]
pub async fn clear_assistant_api_key(
    state: State<'_, AppState>,
) -> CommandResult<AssistantSettings> {
    let _admin = require_admin_user(get_user(&state))?;
    secret_store().clear_secret(AgentSecretKind::AssistantApiKey)?;
    set_assistant_api_key_present(&state.db, false).await?;
    load_settings(&state).await
}

#[tauri::command]
pub async fn set_assistant_cloud_opt_in(
    state: State<'_, AppState>,
    enabled: bool,
) -> CommandResult<AssistantSettings> {
    let _admin = require_admin_user(get_user(&state))?;
    set_assistant_cloud_data_opt_in(&state.db, enabled).await?;
    load_settings(&state).await
}

/// Thẻ xác nhận ghi thành **CHỮ**, không thành dữ liệu dựng lại được. Chỉ
/// `display` và `warnings` — không `payload`, không `built_at_ms`, không
/// `preview`. Xem comment ở `migrate_v27_assistant_conversations`: bảng cố ý
/// không có cột chứa nổi chúng, nên nhét JSON vào `text` là lách đúng hàng rào
/// ấy. `display` là `BTreeMap` nên thứ tự dòng tất định.
fn summarize_action(action: &ProposedAction) -> String {
    let mut lines = vec!["Đề xuất nhận phòng:".to_string()];
    for (key, value) in &action.display {
        lines.push(format!("- {key}: {value}"));
    }
    for warning in &action.warnings {
        lines.push(format!("- (cảnh báo) {warning}"));
    }
    lines.join("\n")
}

/// Một lượt biến thành đúng một hàng trong sổ: `kind` và `text`. `None` khi
/// lượt không sinh ra gì đáng ghi.
///
/// Một hàm chứ không phải hai (`log_kind` + `summarize_turn_for_log` riêng): hai
/// hàm cùng đọc một `outcome` là hai chỗ để lệch nhau, và cái lệch rẻ nhất —
/// hàng `kind='assistant'` mang chữ của thẻ — không test nào nhìn ra ngay.
fn summarize_turn_for_log(
    outcome: &CommandResult<AssistantTurnResponse>,
) -> Option<(&'static str, String)> {
    match outcome {
        Ok(response) => {
            if let Some(reply) = response.reply.as_deref() {
                return Some(("assistant", reply.to_string()));
            }
            response
                .proposed_action
                .as_ref()
                .map(|action| ("action", summarize_action(action)))
        }
        // Lượt hỏng vẫn để lại dấu: mở lại thấy "hôm ấy mình hỏi cái này, trợ lý
        // báo lỗi". `message` của lỗi hệ thống đã là câu chung kèm `support_id`,
        // không phải chuỗi sqlx thô.
        Err(error) => Some(("error", error.message.clone())),
    }
}

/// Kiểm câu hỏi, kiểm danh tính, kiểm quyền sở hữu — **rồi mới** ghi câu hỏi.
///
/// Thứ tự là cả nội dung của hàm này:
///
/// 1. Rỗng / quá 2000 ký tự → từ chối. `run_assistant_turn` cũng kiểm, nhưng nó
///    chạy sau; ghi trước rồi để nó từ chối thì một câu 3000 ký tự đã kịp tạo
///    hội thoại, đặt tên và ghi câu hỏi, để lại hội thoại rác mà không ai gõ ra
///    nó. Trùng lặp có chủ ý — và dùng chung hằng số, không gõ lại con số.
/// 2. Quyền sở hữu kiểm **ở đây**, không để `record_turn_question` nuốt thành
///    "ghi hỏng": nó vẫn trả về id cũ (ca 3b) khi bị chặn, nên lượt sẽ chạy tiếp
///    và câu trả lời rơi vào sổ của người khác.
/// 3. Ghi hỏng thì **không** thành lỗi lượt (`lib.rs:136-143`) — nhưng nguyên
///    nhân gốc vẫn phải vào support log.
async fn open_turn_record(
    pool: &Pool<Sqlite>,
    user: Option<User>,
    conversation_id: Option<&str>,
    question: &str,
    now: &str,
) -> CommandResult<TurnRecord> {
    let question = question.trim();
    if question.is_empty() || question.chars().count() > MAX_MESSAGE_CHARS {
        return Err(CommandError::user(
            codes::VALIDATION_INVALID_INPUT,
            "Câu hỏi rỗng hoặc quá dài.",
        ));
    }

    let user =
        user.ok_or_else(|| CommandError::user(codes::AUTH_NOT_AUTHENTICATED, "Chưa đăng nhập"))?;
    let is_admin = user.role == "admin";

    if let Some(existing) = conversation_id {
        assert_can_read(pool, existing, &user.id, is_admin)
            .await
            .map_err(|error| {
                wrap_service_system_error(
                    TURN_COMMAND,
                    error,
                    json!({ "conversation_id": existing }),
                )
            })?;
    }

    let record =
        record_turn_question(pool, conversation_id, &user.id, is_admin, question, now).await;

    if let Some(failure) = record.failure.clone() {
        // Giá trị trả về bị vứt có chủ ý: lượt chat phải chạy tiếp. Cái cần là
        // tác dụng phụ — nguyên nhân gốc vào support log, và bản đã làm sạch thì
        // không đi đâu cả. Không bọc thì `"no such table: assistant_conversations"`
        // hoặc là bốc hơi, hoặc là lọt ra frontend nếu ai đó sau này đổi hàm này
        // thành `?`.
        let _ = wrap_service_system_error(
            TURN_COMMAND,
            failure,
            json!({ "conversation_id": conversation_id, "stage": "record_question" }),
        );
    }

    Ok(record)
}

/// Ghi câu trả lời rồi nhấc `updated_at`. **Không trả lỗi**: sổ chat là tiện
/// ích, trả lời được câu hỏi mới là việc chính.
///
/// `record.persisted || wrote_reply` không phải chi tiết vặt: `updated_at` chỉ
/// được nhấc khi có ít nhất một message vào sổ (spec dòng 445). Danh sách lịch
/// sử sắp theo `updated_at DESC`, nên touch vô điều kiện là đẩy một hội thoại
/// rỗng lên đầu sổ.
async fn close_turn_record(
    pool: &Pool<Sqlite>,
    record: &TurnRecord,
    outcome: &CommandResult<AssistantTurnResponse>,
    now: &str,
) {
    // Ca 3a: không có hội thoại nào để ghi vào thì bỏ qua hoàn toàn, không cố
    // tạo lại giữa chừng.
    let Some(conversation_id) = record.conversation_id.as_deref() else {
        return;
    };

    let mut wrote_reply = false;
    if let Some((kind, text)) = summarize_turn_for_log(outcome) {
        wrote_reply = conversation_repository::insert_message(
            pool,
            &uuid::Uuid::new_v4().to_string(),
            conversation_id,
            kind,
            &text,
            now,
        )
        .await
        .is_ok();
    }

    if record.persisted || wrote_reply {
        let _ = conversation_repository::touch_conversation(pool, conversation_id, now).await;
    }
}

#[tauri::command]
pub async fn assistant_turn(
    state: State<'_, AppState>,
    request: AssistantTurnRequest,
) -> CommandResult<AssistantTurnResponse> {
    // Danh tính lấy phía Rust và kiểm ngay đây, trước cả cấu hình: người chưa
    // đăng nhập không cần biết trợ lý đã cấu hình hay chưa.
    let user = get_user(&state);
    if user.is_none() {
        return Err(CommandError::user(
            codes::AUTH_NOT_AUTHENTICATED,
            "Chưa đăng nhập",
        ));
    }

    let config = get_assistant_config(&state.db).await?;
    let opt_in = get_assistant_cloud_data_opt_in(&state.db).await?;
    let has_api_key = resolve_assistant_api_key_present(&state.db, &secret_store()).await?;

    // Fail closed TRƯỚC khi dựng prompt. Không có prompt nào được tạo, không có
    // request nào bay ra khi cổng chưa mở.
    let gate = evaluate_assistant_gate(&config, has_api_key, opt_in);
    if !gate.ready {
        if !opt_in {
            return Err(CommandError::user(
                codes::AGENT_CLOUD_DATA_OPT_IN_REQUIRED,
                "Chưa bật đồng ý gửi dữ liệu lên máy chủ AI. Vào Cài đặt → Trợ lý quầy để bật.",
            ));
        }
        return Err(CommandError::user(
            codes::AGENT_RUNTIME_NOT_CONFIGURED,
            "Trợ lý chưa được cấu hình. Vào Cài đặt → Trợ lý quầy.",
        ));
    }

    // Chỗ duy nhất còn đọc keychain trong luồng thường — và chỉ tới đây, khi
    // đã chắc là sắp gọi nhà cung cấp thật. Nếu ai đó xoá mục trong Keychain
    // Access thì cờ trong database lệch, và lệch sẽ lộ ra đúng ở đây thay vì
    // hỏng âm thầm.
    let api_key = secret_store()
        .get_secret(AgentSecretKind::AssistantApiKey)?
        .ok_or_else(|| CommandError::user(codes::AGENT_SECRET_MISSING, "Chưa có khoá API."))?;
    let now_local_date = chrono::Local::now().format("%Y-%m-%d").to_string();
    let now = chrono::Local::now().to_rfc3339();
    let provider = AssistantProviderClient::new(build_assistant_provider_client()?);

    let record = open_turn_record(
        &state.db,
        user,
        request.conversation_id.as_deref(),
        &request.message,
        &now,
    )
    .await?;

    let outcome = run_assistant_turn(
        &state.db,
        &provider,
        &config,
        &api_key,
        request,
        &now_local_date,
    )
    .await;

    close_turn_record(&state.db, &record, &outcome, &now).await;

    let mut response = outcome?;
    response.conversation_id = record.conversation_id;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::assistant::draft::{ProposedAction, CHECK_IN_ACTION_KIND};
    use crate::app_error::GENERIC_SYSTEM_ERROR_MESSAGE;
    use crate::commands::assistant_conversations::tests::{commands_in, CommandShell};
    use crate::models::{CheckInRequest, User};
    use sqlx::sqlite::SqlitePoolOptions;
    use std::collections::BTreeMap;

    const NOW: &str = "2026-08-04T10:00:00+07:00";
    const LATER: &str = "2026-08-04T18:30:00+07:00";

    fn receptionist(id: &str) -> Option<User> {
        Some(User {
            id: id.to_string(),
            name: format!("Lễ tân {id}"),
            role: "receptionist".to_string(),
            active: true,
            created_at: "2026-08-01T00:00:00+07:00".to_string(),
        })
    }

    /// Pool KHÔNG chạy migration: mọi câu SQL vào sổ hội thoại đều nổ
    /// `no such table`.
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

    /// `c1` của `u1`, `c2` của `u2`, **không có tin nhắn nào** — nên mọi khẳng
    /// định "không ghi gì" bên dưới đếm từ 0 và có nghĩa.
    async fn seeded_pool() -> Pool<Sqlite> {
        let pool = unmigrated_pool().await;
        crate::db::run_migrations(&pool).await.expect("migration");

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
             VALUES ('c1', 'u1', 'Hỏi phòng', ?, ?), ('c2', 'u2', 'Hỏi giá', ?, ?)",
        )
        .bind(NOW)
        .bind(NOW)
        .bind(NOW)
        .bind(NOW)
        .execute(&pool)
        .await
        .expect("chèn hội thoại");

        pool
    }

    async fn count(pool: &Pool<Sqlite>, table: &str) -> i64 {
        sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(pool)
            .await
            .expect("đếm")
    }

    async fn logged_message(pool: &Pool<Sqlite>, conversation_id: &str) -> (String, String) {
        sqlx::query_as("SELECT kind, text FROM assistant_messages WHERE conversation_id = ?")
            .bind(conversation_id)
            .fetch_one(pool)
            .await
            .expect("đọc lại tin nhắn vừa ghi")
    }

    async fn updated_at(pool: &Pool<Sqlite>, conversation_id: &str) -> String {
        sqlx::query_scalar("SELECT updated_at FROM assistant_conversations WHERE id = ?")
            .bind(conversation_id)
            .fetch_one(pool)
            .await
            .expect("đọc updated_at")
    }

    fn existing_record(conversation_id: &str, persisted: bool) -> TurnRecord {
        TurnRecord {
            conversation_id: Some(conversation_id.to_string()),
            persisted,
            failure: None,
        }
    }

    fn reply_only(text: &str) -> CommandResult<AssistantTurnResponse> {
        Ok(AssistantTurnResponse {
            reply: Some(text.to_string()),
            proposed_action: None,
            history: Vec::new(),
            conversation_id: None,
        })
    }

    fn action_card() -> CommandResult<AssistantTurnResponse> {
        let mut display = BTreeMap::new();
        display.insert("Khách".to_string(), "Nguyễn Văn A".to_string());
        display.insert("Phòng".to_string(), "201".to_string());

        Ok(AssistantTurnResponse {
            reply: None,
            proposed_action: Some(ProposedAction {
                kind: CHECK_IN_ACTION_KIND.to_string(),
                payload: CheckInRequest {
                    room_id: "R201".to_string(),
                    guests: Vec::new(),
                    nights: 2,
                    source: None,
                    notes: None,
                    paid_amount: None,
                    pricing_type: None,
                },
                display,
                preview: serde_json::json!({ "total": 500000 }),
                warnings: vec!["Phòng đang bẩn".to_string()],
                built_at_ms: 1_754_300_000_000,
            }),
            history: Vec::new(),
            conversation_id: None,
        })
    }

    // ─── Mở lượt: kiểm TRƯỚC, ghi SAU ───

    #[tokio::test]
    async fn the_first_question_opens_names_and_records_a_conversation() {
        let pool = seeded_pool().await;

        let record = open_turn_record(
            &pool,
            receptionist("u1"),
            None,
            "  Tối nay còn phòng nào trống?  ",
            NOW,
        )
        .await
        .expect("lượt hợp lệ");

        assert!(record.persisted);
        let id = record.conversation_id.expect("phải có id trả về frontend");

        let title: String =
            sqlx::query_scalar("SELECT title FROM assistant_conversations WHERE id = ?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .expect("đọc tên");
        assert_eq!(
            title, "Tối nay còn phòng nào trống?",
            "tên cắt từ câu đã trim, không mang theo dấu cách thừa"
        );
        assert_eq!(
            logged_message(&pool, &id).await,
            (
                "user".to_string(),
                "Tối nay còn phòng nào trống?".to_string()
            )
        );
    }

    /// Bảng nghiệm thu của spec: `assistant_turn` với `conversation_id` của
    /// người khác → `AUTH_FORBIDDEN`, **và không ghi gì**.
    ///
    /// Vế "không ghi gì" mới là vế đắt. `record_turn_question` một mình vẫn trả
    /// về id cũ (ca 3b) khi bị chặn quyền, nên bỏ `assert_can_read` ở đây thì
    /// lượt chạy tiếp và câu trả lời rơi vào sổ của người ta.
    #[tokio::test]
    async fn writing_into_someone_elses_conversation_is_refused_before_any_write() {
        let pool = seeded_pool().await;

        let error = open_turn_record(&pool, receptionist("u2"), Some("c1"), "chen ngang", NOW)
            .await
            .expect_err("không được ghi vào hội thoại người khác");

        assert_eq!(error.code, codes::AUTH_FORBIDDEN);
        assert_eq!(
            count(&pool, "assistant_messages").await,
            0,
            "bị chặn quyền thì không một dòng nào được ghi"
        );
    }

    /// Spec dòng 313-314: không tiết lộ hội thoại đó có tồn tại hay không.
    #[tokio::test]
    async fn a_missing_conversation_looks_exactly_like_someone_elses() {
        let pool = seeded_pool().await;

        let others = open_turn_record(&pool, receptionist("u2"), Some("c1"), "chen ngang", NOW)
            .await
            .expect_err("của người khác");
        let missing = open_turn_record(&pool, receptionist("u1"), Some("khong-co"), "id lạ", NOW)
            .await
            .expect_err("id lạ");

        assert_eq!(others.code, missing.code);
        assert_eq!(others.message, missing.message);
    }

    /// `run_assistant_turn` cũng kiểm rỗng và 2000 ký tự — nhưng nó chạy SAU.
    /// Ghi trước rồi để nó từ chối thì một câu 3000 ký tự đã kịp tạo hội thoại,
    /// đặt tên và ghi câu hỏi, để lại hội thoại rác mà không ai gõ ra nó.
    #[tokio::test]
    async fn an_empty_or_oversized_question_is_refused_before_any_write() {
        let pool = seeded_pool().await;
        let too_long = "à".repeat(MAX_MESSAGE_CHARS + 1);

        for question in ["   ", too_long.as_str()] {
            let error = open_turn_record(&pool, receptionist("u1"), None, question, NOW)
                .await
                .expect_err("câu hỏi rỗng hoặc quá dài phải bị chặn");
            assert_eq!(error.code, codes::VALIDATION_INVALID_INPUT);
        }

        assert_eq!(
            count(&pool, "assistant_conversations").await,
            2,
            "không được đẻ hội thoại rác"
        );
        assert_eq!(count(&pool, "assistant_messages").await, 0);
    }

    #[tokio::test]
    async fn a_turn_without_a_session_is_refused_and_writes_nothing() {
        let pool = seeded_pool().await;

        let error = open_turn_record(&pool, None, None, "câu hỏi", NOW)
            .await
            .expect_err("chưa đăng nhập");

        assert_eq!(error.code, codes::AUTH_NOT_AUTHENTICATED);
        assert_eq!(count(&pool, "assistant_conversations").await, 2);
    }

    // ─── Đóng lượt: ghi câu trả lời rồi mới nhấc `updated_at` ───

    /// Spec dòng 445. Danh sách lịch sử sắp theo `updated_at DESC`, nên nhấc
    /// mốc cho một hội thoại chẳng ghi được gì là đẩy một hội thoại rỗng lên
    /// đầu sổ.
    #[tokio::test]
    async fn updated_at_stays_put_when_no_message_could_be_written() {
        let pool = seeded_pool().await;
        sqlx::query("DROP TABLE assistant_messages")
            .execute(&pool)
            .await
            .expect("bỏ bảng tin nhắn để ép mọi lệnh ghi message hỏng");

        close_turn_record(
            &pool,
            &existing_record("c1", false),
            &reply_only("Còn phòng 101."),
            LATER,
        )
        .await;

        assert_eq!(
            updated_at(&pool, "c1").await,
            NOW,
            "không message nào vào sổ thì không được nhấc updated_at"
        );
    }

    #[tokio::test]
    async fn updated_at_moves_as_soon_as_one_message_lands() {
        let pool = seeded_pool().await;

        close_turn_record(
            &pool,
            &existing_record("c1", true),
            &reply_only("Còn phòng 101."),
            LATER,
        )
        .await;

        assert_eq!(updated_at(&pool, "c1").await, LATER);
        assert_eq!(
            logged_message(&pool, "c1").await,
            ("assistant".to_string(), "Còn phòng 101.".to_string())
        );
    }

    /// Ca 3a: không có hội thoại nào để ghi vào thì bước 5 bỏ qua **hoàn toàn**,
    /// không cố tạo lại giữa chừng.
    #[tokio::test]
    async fn a_turn_without_a_conversation_logs_nothing() {
        let pool = seeded_pool().await;
        let record = TurnRecord {
            conversation_id: None,
            persisted: false,
            failure: None,
        };

        close_turn_record(&pool, &record, &reply_only("Còn phòng 101."), LATER).await;

        assert_eq!(count(&pool, "assistant_messages").await, 0);
        assert_eq!(count(&pool, "assistant_conversations").await, 2);
    }

    /// Hàng `kind='action'` là CHỮ, không phải dữ liệu dựng lại được. Bảng cố ý
    /// không có cột chứa nổi `payload` hay `built_at_ms`, và nhét JSON vào
    /// `text` là lách đúng hàng rào ấy: `approve()` cần `action.payload` để bắn
    /// `check_in` — tiền thật.
    #[tokio::test]
    async fn an_action_card_is_logged_as_readable_text_only() {
        let pool = seeded_pool().await;

        close_turn_record(&pool, &existing_record("c1", true), &action_card(), LATER).await;

        let (kind, text) = logged_message(&pool, "c1").await;
        assert_eq!(kind, "action");
        assert_eq!(
            text,
            "Đề xuất nhận phòng:\n- Khách: Nguyễn Văn A\n- Phòng: 201\n- (cảnh báo) Phòng đang bẩn"
        );
        assert!(
            !text.contains("built_at_ms"),
            "không được lưu thứ dựng lại được: {text}"
        );
        assert!(
            !text.contains("R201"),
            "`payload.room_id` không được lọt vào text dưới bất kỳ dạng nào: {text}"
        );
    }

    /// Lượt hỏng vẫn để lại dấu: mở lại thấy "hôm ấy mình hỏi cái này, trợ lý
    /// báo lỗi". Đúng hơn là giả vờ nó chưa từng xảy ra.
    #[tokio::test]
    async fn a_failed_turn_still_leaves_a_trace() {
        let pool = seeded_pool().await;
        let outcome = Err(CommandError::user(
            codes::AGENT_TOOL_LOOP_LIMIT,
            "Trợ lý tra cứu quá nhiều vòng mà chưa ra kết quả.",
        ));

        close_turn_record(&pool, &existing_record("c1", true), &outcome, LATER).await;

        assert_eq!(
            logged_message(&pool, "c1").await,
            (
                "error".to_string(),
                "Trợ lý tra cứu quá nhiều vòng mà chưa ra kết quả.".to_string()
            )
        );
    }

    // ─── Lỗi hệ thống: chuỗi sqlx không được ra tới frontend ───

    /// `conversation_service` dựng `CommandError::system(format!("…: {error}"))`
    /// ở **hai** chỗ, và Task 5 chạm cả hai: `assert_can_read` (lỗi đi thẳng ra
    /// frontend qua `?`) và `insert_conversation` bên trong `ensure_conversation`
    /// (lỗi không ra ngoài, nhưng phải vào support log chứ không bốc hơi).
    /// `mhm/src/lib/appError/format.ts` nối thẳng `message` ra UI.
    // `env_lock` là `std::sync::Mutex` và phải giữ suốt cả test — khuôn có sẵn
    // trong repo, xem `agent/supervisor.rs`.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn a_database_failure_never_hands_the_raw_sqlx_message_to_the_frontend() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();
        let runtime_root =
            std::env::temp_dir().join(format!("capyinn-assistant-turn-{}", uuid::Uuid::new_v4()));
        std::env::set_var("CAPYINN_RUNTIME_ROOT", &runtime_root);

        let pool = unmigrated_pool().await;
        let refused = open_turn_record(&pool, receptionist("u1"), Some("c1"), "câu hỏi", NOW)
            .await
            .err();
        let opened = open_turn_record(&pool, receptionist("u1"), None, "câu hỏi", NOW).await;

        std::env::remove_var("CAPYINN_RUNTIME_ROOT");

        let refused = refused.expect("bảng không tồn tại thì đường kiểm quyền phải lỗi");
        assert_eq!(
            refused.message, GENERIC_SYSTEM_ERROR_MESSAGE,
            "đẩy chuỗi lạ ra frontend: {}",
            refused.message
        );
        assert!(!refused.message.contains("no such table"));
        let support_id = refused.support_id.as_deref().expect("thiếu mã hỗ trợ");

        let opened = opened.expect("ghi hỏng không được biến thành lỗi lượt");
        assert_eq!(opened.conversation_id, None, "ca 3a");
        assert!(!opened.persisted);

        let log = std::fs::read_to_string(
            runtime_root
                .join("diagnostics")
                .join("support-errors.jsonl"),
        )
        .expect("support log phải được ghi");
        assert!(
            log.contains(support_id),
            "mã {support_id} không tra được trong support log"
        );
        assert!(
            log.contains("assistant_turn"),
            "lệnh phải tự khai tên mình, không dùng chung một chuỗi"
        );
        assert_eq!(
            log.lines()
                .filter(|line| line.contains("no such table"))
                .count(),
            2,
            "cả hai chỗ dựng lỗi hệ thống đều phải để lại nguyên nhân gốc trong \
             support log — kể cả chỗ không ra tới frontend"
        );

        let _ = std::fs::remove_dir_all(&runtime_root);
    }

    // ─── Hình dạng chữ ký: danh tính không bao giờ tới từ frontend ───

    /// Bảng **mỗi lệnh một tập tham số**, chép nguyên văn từ trên đĩa. File này
    /// không dùng được một allowlist chung như `assistant_conversations.rs`: sáu
    /// lệnh ở đây nhận sáu bộ tham số khác nhau (`preset`, `base_url`, `model`,
    /// `api_key`, `enabled`, `request`), nên một allowlist gộp sẽ cho `api_key`
    /// đi lạc sang lệnh chat.
    ///
    /// So bằng **đúng dãy**, không phải "nằm trong danh sách": so-nằm-trong vẫn
    /// xanh khi bộ đọc trả về dãy rỗng, mà bộ đọc què là guard xanh giả — đúng
    /// chế độ hỏng Task 4 đo được. Đổi chữ ký có chủ ý thì sửa bảng; bảng cố
    /// tình bắt bạn nhìn thấy nó.
    const PARAMETERS_EACH_COMMAND_TAKES: [(&str, &[&str]); 6] = [
        ("get_assistant_settings", &["state"]),
        (
            "set_assistant_settings",
            &["state", "preset", "base_url", "model"],
        ),
        ("set_assistant_api_key", &["state", "api_key"]),
        ("clear_assistant_api_key", &["state"]),
        ("set_assistant_cloud_opt_in", &["state", "enabled"]),
        ("assistant_turn", &["state", "request"]),
    ];

    /// Số vỏ đọc được phải khớp bảng. Đọc hụt là guard xanh giả.
    fn shells_of_this_file() -> Vec<CommandShell> {
        let shells = commands_in(include_str!("assistant.rs"));

        assert_eq!(
            shells.len(),
            PARAMETERS_EACH_COMMAND_TAKES.len(),
            "đọc hụt (hoặc đọc dư) vỏ lệnh — bộ đọc hỏng thì cửa này thành cửa \
             mở: {:?}",
            shells.iter().map(|shell| &shell.name).collect::<Vec<_>>()
        );

        shells
    }

    /// Hàng rào của Task 4 đọc bằng `include_str!("assistant_conversations.rs")`
    /// — **chỉ file đó**. Một `#[tauri::command]` ở đây nhận `user_id: String`
    /// rồi đọc hội thoại thì trước Task 5 hoàn toàn không ai canh, mà hội thoại
    /// chứa tên khách và số CCCD.
    #[test]
    fn no_command_here_takes_an_identity_from_the_frontend() {
        let shells = shells_of_this_file();

        let mut violations = Vec::new();
        for (name, expected_parameters) in PARAMETERS_EACH_COMMAND_TAKES {
            let Some(shell) = shells.iter().find(|shell| shell.name == name) else {
                violations.push(format!("thiếu hẳn vỏ `{name}`"));
                continue;
            };
            if shell.parameters != expected_parameters {
                violations.push(format!(
                    "`{name}`\n  bảng nói: {expected_parameters:?}\n  trên đĩa: {:?}",
                    shell.parameters
                ));
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

    /// Tên trường của một `struct`, đọc từ mã nguồn. Bỏ dòng thuộc tính
    /// (`#[serde(default)]`) và dòng chú thích; kết thúc ở dòng `}` cột 0, đúng
    /// khuôn rustfmt mà bộ đọc vỏ lệnh đã dựa vào.
    fn struct_fields(source: &str, name: &str) -> Vec<String> {
        let mut lines = source.lines().skip_while(|line| {
            !line
                .trim_start()
                .starts_with(&format!("pub struct {name} {{"))
        });
        lines.next();

        lines
            .take_while(|line| *line != "}")
            .map(|line| line.trim())
            .filter(|line| !line.starts_with('#') && !line.starts_with("//"))
            .filter_map(|line| line.split_once(':'))
            .map(|(field, _)| field.trim_start_matches("pub ").trim().to_string())
            .collect()
    }

    /// Bảng tham số bên trên canh **chữ ký**, và `assistant_turn` nhận đúng một
    /// `request: AssistantTurnRequest`. Thêm `user_id` vào *bên trong* struct đó
    /// là đi vòng qua bảng: chữ ký không đổi, mà danh tính vẫn từ frontend vào.
    ///
    /// `AssistantTurnResponse` ở đây là đối chứng cho chính bộ đọc: nếu
    /// `struct_fields` trả về rỗng thì khẳng định thứ hai đỏ.
    #[test]
    fn the_turn_request_carries_no_identity_field() {
        let source = include_str!("../agent/assistant/mod.rs");

        assert_eq!(
            struct_fields(source, "AssistantTurnRequest"),
            ["message", "screen_context", "history", "conversation_id"],
            "`conversation_id` là một id, không phải một lời khai về mình — mọi \
             trường khác phải qua cửa này trước"
        );
        assert_eq!(
            struct_fields(source, "AssistantTurnResponse"),
            ["reply", "proposed_action", "history", "conversation_id"]
        );
    }
}
