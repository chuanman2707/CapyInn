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
//!
//! Thứ tự ấy phải **nhìn thấy được trong dữ liệu**, không chỉ trong luồng chạy:
//! câu hỏi và câu trả lời mang hai mốc `created_at` khác nhau, tính ở hai thời
//! điểm khác nhau. Xem `close_turn_record`.

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
            draft::{
                ProposedAction, BACKFILL_ACTION_KIND, CHECK_IN_ACTION_KIND, RESERVE_ACTION_KIND,
            },
            provider::{build_assistant_provider_client, AssistantProviderClient},
            run_assistant_turn, AssistantTurnRequest, AssistantTurnResponse, MAX_MESSAGE_CHARS,
        },
        secrets::{AgentSecretKind, AgentSecretStore, KeychainSecretStore},
    },
    app_error::{codes, AppErrorKind, CommandError, CommandResult, GENERIC_SYSTEM_ERROR_MESSAGE},
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
///
/// Dòng đầu nói **đúng loại thẻ**. Một thẻ đặt phòng trước ghi vào sổ thành "Đề
/// xuất nhận phòng" là in lại nguyên văn câu nói dối đã mở ra cả spec này (mục
/// 1: trợ lý báo "Đã nhận phòng xong." trong khi vừa ghi một bản ghi sai) — chỉ
/// khác là lần này nó nằm trong sổ và người đọc lại sẽ tin nó. Cùng luật
/// `ACTION_KIND_COPY` bên `types/assistant.ts`.
///
/// `kind` lạ **không** được gọi tên theo loại nào: dán nhãn sai còn tệ hơn nói
/// thẳng là không biết.
fn summarize_action(action: &ProposedAction) -> String {
    let heading = match action.kind.as_str() {
        CHECK_IN_ACTION_KIND => "Đề xuất nhận phòng:",
        RESERVE_ACTION_KIND => "Đề xuất đặt phòng trước:",
        BACKFILL_ACTION_KIND => "Đề xuất ghi bù:",
        _ => "Đề xuất (loại thẻ không rõ):",
    };
    let mut lines = vec![heading.to_string()];
    for (key, value) in &action.display {
        lines.push(format!("- {key}: {value}"));
    }
    for warning in &action.warnings {
        lines.push(format!("- (cảnh báo) {warning}"));
    }
    lines.join("\n")
}

/// Chữ của một lượt hỏng, ghi vào hàng `kind='error'`.
///
/// Lỗi **người dùng** ghi nguyên văn: "Trợ lý tra cứu quá nhiều vòng mà chưa ra
/// kết quả" là câu viết cho lễ tân đọc, và mở sổ lại phải đọc được đúng câu ấy.
///
/// Lỗi **hệ thống** thì không, và đây là chỗ dễ tin nhầm nhất trong file:
/// `CommandError::system` (`app_error.rs:170-181`) **giữ nguyên** `message` —
/// chỉ `log_system_error` mới thay nó bằng câu chung. Nên `error.message` của
/// một lỗi `System` tới đây vẫn là `format!("…: {error}")` với `error` là
/// `sqlx::Error`. Đường sống có thật, không phải giả định: `draft.rs:311` dựng
/// `CommandError::system(…, format!("Không đọc được trạng thái phòng: {error}"))`
/// → `build_check_in_draft` → `run_assistant_turn` cho đi thẳng ra bằng `?`
/// (`agent/assistant/mod.rs:150`) → `outcome`.
///
/// Ghi thẳng chuỗi ấy là **chôn vĩnh viễn** vào sổ đúng thứ mà Task 3 và Task 4
/// tốn hai vòng duyệt để giữ khỏi màn hình quầy — và Task 6-10 sẽ render lại
/// cái sổ đó. Câu chung kèm mã hỗ trợ viết y như `appError/format.ts:25-26`
/// hiện trên màn hình, để dòng trong sổ khớp với thứ lễ tân đã nhìn thấy.
fn error_text(error: &CommandError) -> String {
    match error.kind {
        AppErrorKind::User => error.message.clone(),
        AppErrorKind::System => match error.support_id.as_deref() {
            Some(support_id) => {
                format!("{GENERIC_SYSTEM_ERROR_MESSAGE} (Mã hỗ trợ: {support_id})")
            }
            None => GENERIC_SYSTEM_ERROR_MESSAGE.to_string(),
        },
    }
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
        // báo lỗi". Nhưng KHÔNG phải bằng `error.message` — xem `error_text`:
        // message của lỗi hệ thống là chuỗi sqlx thô, không phải câu chung.
        Err(error) => Some(("error", error_text(error))),
    }
}

/// Sổ hội thoại của **một lượt**: đang mở, hay đóng riêng cho lượt này.
///
/// Trạng thái thứ ba (`Off`) tồn tại vì `assert_can_read` có thể hỏng vì hai lý
/// do khác hẳn nhau. Bị **từ chối quyền** là câu trả lời dứt khoát của hệ
/// thống, phải nổ ra ngoài. **Không đọc nổi** chủ hội thoại (DB hỏng, thiếu
/// bảng) thì không phải là từ chối gì cả — mà `assert_can_read` tồn tại để bảo
/// vệ đường GHI, không phải để bảo vệ câu trả lời. Không xác minh được quyền
/// thì **không ghi gì cả**, và không ghi thì không có rò chéo người dùng, nên
/// lượt chat vẫn trả lời được an toàn.
///
/// `Off` mang theo id chứ **không** rơi về `None`: `None` là ca 3a, nghĩa là
/// "chưa có hội thoại nào" và frontend sẽ mở hội thoại mới cho lượt sau. Rơi về
/// đó thì mỗi lần bấm gửi trên một DB đang hỏng lại đẻ thêm một hội thoại rác.
#[derive(Debug)]
enum TurnLog {
    /// Sổ đang mở. `TurnRecord` nói ghi được tới đâu.
    On(TurnRecord),
    /// **Lượt này không ghi sổ.** `close_turn_record` không ghi hàng nào và
    /// không nhấc `updated_at`.
    Off { conversation_id: String },
}

impl TurnLog {
    /// Id trả về frontend. `Off` vẫn trả id cũ — hội thoại ấy có thật, chỉ lượt
    /// này không vào được sổ.
    fn conversation_id(&self) -> Option<String> {
        match self {
            TurnLog::On(record) => record.conversation_id.clone(),
            TurnLog::Off { conversation_id } => Some(conversation_id.clone()),
        }
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
///    và câu trả lời rơi vào sổ của người khác. Lỗi `User` (từ chối quyền) nổ ra
///    ngoài bằng `Err`; lỗi `System` (không đọc nổi) chuyển sang `TurnLog::Off`
///    — xem `TurnLog`.
/// 3. Ghi hỏng thì **không** thành lỗi lượt (`lib.rs:136-143`) — nhưng nguyên
///    nhân gốc vẫn phải vào support log.
async fn open_turn_record(
    pool: &Pool<Sqlite>,
    user: Option<User>,
    conversation_id: Option<&str>,
    question: &str,
    now: &str,
) -> CommandResult<TurnLog> {
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
        if let Err(error) = assert_can_read(pool, existing, &user.id, is_admin).await {
            let error = wrap_service_system_error(
                TURN_COMMAND,
                error,
                json!({ "conversation_id": existing }),
            );

            // Từ chối quyền là câu trả lời dứt khoát: nổ ra ngoài. Nuốt nó
            // thành "ghi hỏng" rồi chạy tiếp là để câu trả lời rơi vào sổ của
            // người khác.
            if error.kind == AppErrorKind::User {
                return Err(error);
            }

            // Không đọc nổi chủ hội thoại thì đóng sổ cho riêng lượt này, giữ
            // nguyên id. Nguyên nhân gốc đã vào support log ở dòng trên.
            return Ok(TurnLog::Off {
                conversation_id: existing.to_string(),
            });
        }
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

    Ok(TurnLog::On(record))
}

/// Ghi câu trả lời rồi nhấc `updated_at`. **Không trả lỗi**: sổ chat là tiện
/// ích, trả lời được câu hỏi mới là việc chính.
///
/// **Mốc thời gian tính ở đây, không nhận từ caller — và đó là cả một sửa lỗi.**
/// Bản cũ nhận `now: &str`, còn `assistant_turn` tính `now` đúng một lần rồi
/// đưa cho cả câu hỏi lẫn câu trả lời. `conversation_queries::get_messages` sắp
/// `ORDER BY created_at ASC, id ASC` mà `id` là UUIDv4 ngẫu nhiên, nên hoà
/// `created_at` là để thứ tự cho bốc thăm: đo bằng `sqlite3` trên đúng câu SQL
/// ấy, 2000 lượt, **993 lần (49,6%) câu trả lời hiện TRƯỚC câu hỏi**. Sổ hội
/// thoại đọc lộn ngược một nửa số lượt, và không test nào bắt được vì fixture
/// nào cũng chỉ ghi một message.
///
/// Bỏ hẳn tham số chứ không chỉ "nhớ truyền mốc thứ hai": còn tham số thì còn
/// truyền lại được mốc của câu hỏi, và chế độ hỏng ấy im lặng 50% số lần. Ở đây
/// mốc chỉ có thể là **sau** khi có `outcome` — tức sau khi trợ lý đã trả lời —
/// nên nó cũng làm `updated_at` trung thực hơn: bản cũ nhấc `updated_at` lên
/// mốc *trước* lúc trợ lý trả lời.
///
/// `record.persisted || wrote_reply` không phải chi tiết vặt: `updated_at` chỉ
/// được nhấc khi có ít nhất một message vào sổ (spec dòng 445). Danh sách lịch
/// sử sắp theo `updated_at DESC`, nên touch vô điều kiện là đẩy một hội thoại
/// rỗng lên đầu sổ.
///
/// **Trả về `AssistantTurnResponse::turn_saved`** — `true` ⟺ lượt này vào sổ
/// TRỌN VẸN. Đây là chỗ duy nhất trong cả luồng biết được điều đó: `record`
/// nói câu hỏi có vào không, còn câu trả lời thì chỉ hàm này ghi. Trước bản
/// này bit ấy chết ngay trong `close_turn_record` (chỉ dùng để quyết định bump
/// `updated_at`) và không bao giờ ra tới biên — xem doc của trường `turn_saved`.
///
/// Ba đường trả `false` và cả ba đều là mất dữ liệu thật, không phải trạng thái
/// trung tính: sổ đóng (`Off`), ca 3a (không có hội thoại để ghi vào), và ca 3b
/// / bước-5-hỏng (ghi trượt một trong hai hàng).
async fn close_turn_record(
    pool: &Pool<Sqlite>,
    log: &TurnLog,
    outcome: &CommandResult<AssistantTurnResponse>,
) -> bool {
    // Sổ đóng cho riêng lượt này (không xác minh được quyền): không ghi hàng
    // nào, không nhấc `updated_at`. Không ghi gì nghĩa là KHÔNG lưu được —
    // hội thoại vẫn còn trên đĩa nhưng lượt vừa rồi thì không vào đó.
    let TurnLog::On(record) = log else {
        return false;
    };

    // Ca 3a: không có hội thoại nào để ghi vào thì bỏ qua hoàn toàn, không cố
    // tạo lại giữa chừng.
    let Some(conversation_id) = record.conversation_id.as_deref() else {
        return false;
    };

    let answered_at = chrono::Local::now().to_rfc3339();

    // Tách "có gì để ghi không" khỏi "ghi có trúng không". Gộp hai câu đó vào
    // một cờ là báo mất dữ liệu cho một lượt chẳng có gì để mất.
    let row_to_write = summarize_turn_for_log(outcome);
    let has_row_to_write = row_to_write.is_some();

    let mut wrote_reply = false;
    if let Some((kind, text)) = row_to_write {
        wrote_reply = conversation_repository::insert_message(
            pool,
            &uuid::Uuid::new_v4().to_string(),
            conversation_id,
            kind,
            &text,
            &answered_at,
        )
        .await
        .is_ok();
    }

    if record.persisted || wrote_reply {
        let _ =
            conversation_repository::touch_conversation(pool, conversation_id, &answered_at).await;
    }

    record.persisted && (wrote_reply || !has_row_to_write)
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

    let log = open_turn_record(
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

    let turn_saved = close_turn_record(&state.db, &log, &outcome).await;

    // ĐƯỜNG LÁCH CÒN ĐỂ NGỎ, ghi lại vì hậu quả là thứ lễ tân nhìn thấy.
    //
    // `?` ở đây vứt mất `conversation_id` của một lượt hỏng. Với lượt thứ hai
    // trở đi thì vô hại — frontend đã cầm id từ lượt trước. Nhưng với lượt
    // **đầu tiên** của một hội thoại, id vừa tạo ra chỉ tồn tại ở đây, và vứt
    // nó đi nghĩa là frontend không có gì để gửi lên cho lần sau.
    //
    // Hậu quả cụ thể khi nhà cung cấp chết: mỗi lần lễ tân bấm gửi lại là một
    // hội thoại rác mới (câu hỏi + hàng `error`), và vì `updated_at` đã bump
    // nên nó nằm **trên đầu** danh sách lịch sử. Thử 5 lần là 5 hội thoại rác
    // đè lên lịch sử thật — mà lễ tân **không xoá được**, spec chỉ cho admin
    // xoá.
    //
    // Cố ý KHÔNG sửa ở đây: mang `conversation_id` ra cùng `Err` là đổi hợp
    // đồng lỗi của lệnh, kéo theo `CommandError` và cả đường lỗi phía frontend
    // — việc của một thay đổi riêng, không phải của bản vá này.
    let mut response = outcome?;
    response.conversation_id = log.conversation_id();
    // Hai dòng, cùng một luật: `run_assistant_turn` không biết gì về sổ hội
    // thoại nên nó trả về hai giá trị giữ chỗ, và tầng này — chỗ DUY NHẤT chạm
    // sổ — ghi đè cả hai bằng giá trị thật.
    response.turn_saved = turn_saved;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::assistant::draft::{
        build_backfill_display, build_check_in_display, build_reserve_display, ActionPayload,
        ProposedAction, BACKFILL_ACTION_KIND, CHECK_IN_ACTION_KIND, RESERVE_ACTION_KIND,
    };
    use crate::commands::assistant_conversations::tests::{commands_in, CommandShell};
    use crate::models::BackfillStayRequest;
    use crate::models::CreateReservationRequest;
    use crate::models::{CheckInRequest, CreateGuestRequest, User};
    use crate::queries::assistant::conversation_queries;
    use sqlx::sqlite::SqlitePoolOptions;

    /// Mốc của câu hỏi. Cố ý nằm trong **quá khứ** so với đồng hồ máy: câu trả
    /// lời lấy mốc thật bằng `chrono::Local::now()` bên trong
    /// `close_turn_record`, nên "sau câu hỏi" là một khẳng định đo được chứ
    /// không phải một hằng số thứ hai do test tự đặt.
    const NOW: &str = "2026-08-04T10:00:00+07:00";

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

    fn existing_record(conversation_id: &str, persisted: bool) -> TurnLog {
        TurnLog::On(TurnRecord {
            conversation_id: Some(conversation_id.to_string()),
            persisted,
            failure: None,
        })
    }

    fn reply_only(text: &str) -> CommandResult<AssistantTurnResponse> {
        Ok(AssistantTurnResponse {
            reply: Some(text.to_string()),
            proposed_action: None,
            history: Vec::new(),
            conversation_id: None,
            // Giá trị giữ chỗ của `run_assistant_turn`, đúng như đường thật:
            // `close_turn_record` nhận `outcome` chứ không nhận cờ, nên fixture
            // đặt gì ở đây cũng không đổi được kết quả nó trả về.
            turn_saved: false,
        })
    }

    /// Thẻ xác nhận dựng bằng **đúng hàm sản xuất** `build_check_in_display`.
    ///
    /// Bản cũ dựng `display` bằng tay với hai dòng vô hại (`Khách`, `Phòng`),
    /// nên khẳng định "`payload.room_id` không lọt vào text" xanh — mà xanh vì
    /// fixture, không vì code. Ngoài đời `build_check_in_display`
    /// (`draft.rs:62`) **luôn** nhét `display["room_id"]`, và mỗi khách một
    /// dòng gồm họ tên · CCCD · SĐT · ngày sinh · địa chỉ · đường dẫn ảnh giấy
    /// tờ. Fixture nào không có những thứ ấy thì đo một cái sổ không tồn tại.
    fn action_card() -> CommandResult<AssistantTurnResponse> {
        let payload = CheckInRequest {
            room_id: "R201".to_string(),
            guests: vec![CreateGuestRequest {
                guest_type: None,
                full_name: "Nguyễn Văn A".to_string(),
                doc_number: "012345678901".to_string(),
                dob: Some("1990-01-02".to_string()),
                gender: None,
                nationality: None,
                address: Some("12 Lê Lợi, Huế".to_string()),
                visa_expiry: None,
                scan_path: Some("/anh-giay-to/cccd-a.jpg".to_string()),
                phone: Some("0905000111".to_string()),
            }],
            nights: 2,
            source: None,
            notes: None,
            paid_amount: None,
            pricing_type: None,
            rate_override_per_night: None,
        };
        let preview = serde_json::json!({ "total": 500000 });
        // Khoảng ngày y như đường thật: `build_check_in_draft` truyền
        // `now_local_date` và ngày trả suy ra từ `nights` (2 đêm từ 04/08).
        let display = build_check_in_display(&payload, &preview, "2026-08-04", "2026-08-06");

        Ok(AssistantTurnResponse {
            reply: None,
            proposed_action: Some(ProposedAction {
                kind: CHECK_IN_ACTION_KIND.to_string(),
                payload: ActionPayload::CheckIn(payload),
                display,
                preview,
                warnings: vec!["Phòng đang ở trạng thái bẩn, chưa dọn.".to_string()],
                built_at_ms: 1_754_300_000_000,
            }),
            history: Vec::new(),
            conversation_id: None,
            turn_saved: false,
        })
    }

    /// Thẻ **đặt phòng trước**, dựng bằng đúng hàm sản xuất
    /// `build_reserve_display` — cùng lý do như fixture nhận phòng ngay trên:
    /// một `display` dựng tay chỉ đo được chính nó.
    fn reserve_action_card() -> CommandResult<AssistantTurnResponse> {
        let payload = CreateReservationRequest {
            room_id: "R201".to_string(),
            guest_name: "Hyungchul Lee".to_string(),
            guest_phone: Some("0905000111".to_string()),
            guest_doc_number: Some("M12345678".to_string()),
            check_in_date: "2026-08-08".to_string(),
            check_out_date: "2026-08-09".to_string(),
            nights: 1,
            deposit_amount: Some(200_000),
            source: Some("phone".to_string()),
            notes: None,
            guests: None,
        };
        let preview = serde_json::json!({ "total": 400000 });
        let display = build_reserve_display(&payload, &preview);

        Ok(AssistantTurnResponse {
            reply: None,
            proposed_action: Some(ProposedAction {
                kind: RESERVE_ACTION_KIND.to_string(),
                payload: ActionPayload::Reserve(payload),
                display,
                preview,
                warnings: Vec::new(),
                built_at_ms: 1_754_300_000_000,
            }),
            history: Vec::new(),
            conversation_id: None,
            turn_saved: false,
        })
    }

    /// Thẻ **ghi bù**, dựng bằng đúng hàm sản xuất `build_backfill_display`.
    ///
    /// Khách **còn ở** (`check_out_date: None`) — nhánh khó của thẻ này, vì nó
    /// là nhánh duy nhất trong cả ba loại thẻ mà ngày trả không phải một ngày.
    fn backfill_action_card() -> CommandResult<AssistantTurnResponse> {
        let payload = BackfillStayRequest {
            room_id: "R201".to_string(),
            guests: vec![CreateGuestRequest {
                guest_type: None,
                full_name: "Trần Thị Bích".to_string(),
                doc_number: "079301005678".to_string(),
                dob: None,
                gender: None,
                nationality: None,
                address: None,
                visa_expiry: None,
                scan_path: None,
                phone: Some("0909000111".to_string()),
            }],
            check_in_date: "2026-08-02".to_string(),
            check_out_date: None,
            expected_checkout_date: Some("2026-08-07".to_string()),
            total_price: 600_000,
            paid_amount: 0,
            source: Some("walk-in".to_string()),
            notes: None,
        };
        let preview = serde_json::json!({ "total": 600000 });
        let display = build_backfill_display(&payload);

        Ok(AssistantTurnResponse {
            reply: None,
            proposed_action: Some(ProposedAction {
                kind: BACKFILL_ACTION_KIND.to_string(),
                payload: ActionPayload::Backfill(payload),
                display,
                preview,
                warnings: Vec::new(),
                built_at_ms: 1_754_300_000_000,
            }),
            history: Vec::new(),
            conversation_id: None,
            turn_saved: false,
        })
    }

    // ─── Mở lượt: kiểm TRƯỚC, ghi SAU ───

    #[tokio::test]
    async fn the_first_question_opens_names_and_records_a_conversation() {
        let pool = seeded_pool().await;

        let log = open_turn_record(
            &pool,
            receptionist("u1"),
            None,
            "  Tối nay còn phòng nào trống?  ",
            NOW,
        )
        .await
        .expect("lượt hợp lệ");

        let TurnLog::On(record) = &log else {
            panic!("đường thường phải mở được sổ");
        };
        assert!(record.persisted);
        let id = log.conversation_id().expect("phải có id trả về frontend");

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
        )
        .await;

        // So bằng mốc đã phân tích, không bằng chuỗi: `close_turn_record` tự
        // tính mốc của mình (xem doc-comment ở đó), nên test không được biết
        // trước chuỗi ấy — chỉ được biết nó phải nằm **sau** câu hỏi.
        let bumped = chrono::DateTime::parse_from_rfc3339(&updated_at(&pool, "c1").await)
            .expect("updated_at phải là rfc3339");
        let asked = chrono::DateTime::parse_from_rfc3339(NOW).expect("mốc câu hỏi");
        assert!(
            bumped > asked,
            "updated_at phải nhích lên sau khi trợ lý trả lời, đo được: {bumped}"
        );
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
        let log = TurnLog::On(TurnRecord {
            conversation_id: None,
            persisted: false,
            failure: None,
        });

        close_turn_record(&pool, &log, &reply_only("Còn phòng 101.")).await;

        assert_eq!(count(&pool, "assistant_messages").await, 0);
        assert_eq!(count(&pool, "assistant_conversations").await, 2);
    }

    /// Trạng thái thứ ba: sổ đóng cho riêng lượt này. Không ghi hàng nào,
    /// **không** nhấc `updated_at` — trên một DB hoàn toàn lành lặn, nên đây là
    /// khẳng định về ý định của code chứ không phải về việc lệnh ghi hỏng.
    #[tokio::test]
    async fn a_closed_log_writes_nothing_and_leaves_updated_at_alone() {
        let pool = seeded_pool().await;
        let log = TurnLog::Off {
            conversation_id: "c1".to_string(),
        };

        close_turn_record(&pool, &log, &reply_only("Còn phòng 101.")).await;

        assert_eq!(
            count(&pool, "assistant_messages").await,
            0,
            "sổ đóng thì không một dòng nào được ghi"
        );
        assert_eq!(
            updated_at(&pool, "c1").await,
            NOW,
            "sổ đóng thì updated_at đứng yên — không thì một hội thoại không ghi \
             được gì vẫn nhảy lên đầu danh sách lịch sử"
        );
        assert_eq!(
            log.conversation_id().as_deref(),
            Some("c1"),
            "id phải đi tiếp về frontend: rơi về None là bảo nó mở hội thoại mới, \
             và mỗi lần bấm gửi lại đẻ thêm một hội thoại rác"
        );
    }

    // ─── `turn_saved`: bit đi ra tới biên ───
    //
    // Spec dòng 446-447 đòi một dòng "Không lưu được hội thoại này". Ca 3b trả
    // **đúng id cũ** — giống hệt một lượt thành công — nên nếu bit này không ra
    // được tới `AssistantTurnResponse` thì frontend không có cách nào phân biệt,
    // và sổ mất tin nhắn hoàn toàn im lặng. Bốn test dưới ghim bốn nhánh của
    // `close_turn_record`, cộng một test canh chiều ngược (đường thường KHÔNG
    // được báo mất) — không có nó thì `false` cứng cũng xanh cả bộ.

    /// Đường thường: hỏi được, trả lời được, cả hai hàng vào sổ → không báo mất.
    #[tokio::test]
    async fn a_clean_turn_reports_itself_as_saved() {
        let pool = seeded_pool().await;

        let saved = close_turn_record(
            &pool,
            &existing_record("c1", true),
            &reply_only("Còn phòng 101."),
        )
        .await;

        assert!(
            saved,
            "lượt vào sổ trọn vẹn mà vẫn báo mất là dạy lễ tân bỏ qua dòng thông \
             báo — nhắc sai thường trực thì người ta thôi đọc"
        );
    }

    /// Ca 3b và ca "bước 5 hỏng" gộp làm một ở đây: hội thoại vẫn còn, vẫn kiểm
    /// quyền được, chỉ đường ghi message gãy. Đây là ca NGUY HIỂM NHẤT — id trả
    /// về giống hệt một lượt thành công.
    ///
    /// **`persisted: true` là cả bài test.** Bản đầu truyền `false`, và như thế
    /// thì `record.persisted &&` một mình đã trả `false` — cùng nhánh mà
    /// `a_turn_that_lost_only_the_question_still_reports_not_saved` ngay dưới
    /// đang ghim, nên hai test đo trùng một vế và vế `wrote_reply` **không ai
    /// canh**. Đo được: rút gọn thân hàm còn đúng `record.persisted` để lại
    /// 36/36 XANH. Câu hỏi đã vào sổ rồi thì `false` ở đây chỉ có thể tới từ lệnh
    /// ghi câu trả lời — tức đây là chỗ duy nhất ca 3b bị bắt.
    #[tokio::test]
    async fn a_turn_whose_rows_could_not_be_written_reports_not_saved() {
        let pool = seeded_pool().await;
        sqlx::query("DROP TABLE assistant_messages")
            .execute(&pool)
            .await
            .expect("bỏ bảng tin nhắn để ép mọi lệnh ghi message hỏng");

        let saved = close_turn_record(
            &pool,
            &existing_record("c1", true),
            &reply_only("Còn phòng 101."),
        )
        .await;

        assert!(
            !saved,
            "DB khoá hay đầy đĩa mà báo là đã lưu thì lễ tân không bao giờ biết \
             sổ đang mất tin nhắn — mở lại về sau chỉ thấy một khoảng trống"
        );
    }

    /// Câu hỏi trượt nhưng câu trả lời vào được: vẫn là **mất một nửa lượt**.
    ///
    /// Ghim riêng vế này vì `record.persisted &&` là chỗ duy nhất canh nó — bỏ
    /// đi thì sổ có câu trả lời mà không có câu hỏi, đọc lại không hiểu trợ lý
    /// đang trả lời cái gì, và không một dòng thông báo nào.
    #[tokio::test]
    async fn a_turn_that_lost_only_the_question_still_reports_not_saved() {
        let pool = seeded_pool().await;

        let saved = close_turn_record(
            &pool,
            &existing_record("c1", false),
            &reply_only("Còn phòng 101."),
        )
        .await;

        assert_eq!(
            logged_message(&pool, "c1").await.0,
            "assistant",
            "fixture phải ghi được câu trả lời, không thì test đo nhầm nhánh"
        );
        assert!(!saved, "mất câu hỏi cũng là mất, không phải nửa thành công");
    }

    /// Ca 3a: không tạo được hội thoại nên không có chỗ nào để ghi.
    #[tokio::test]
    async fn a_turn_without_a_conversation_reports_not_saved() {
        let pool = seeded_pool().await;
        let log = TurnLog::On(TurnRecord {
            conversation_id: None,
            persisted: false,
            failure: None,
        });

        let saved = close_turn_record(&pool, &log, &reply_only("Còn phòng 101.")).await;

        assert!(!saved);
    }

    /// `TurnLog::Off`: sổ đóng cho riêng lượt này. Hội thoại có thật và id vẫn
    /// đi tiếp về frontend, nhưng lượt vừa rồi thì không vào đó — nên vẫn phải
    /// báo mất.
    #[tokio::test]
    async fn a_closed_log_reports_not_saved() {
        let pool = seeded_pool().await;
        let log = TurnLog::Off {
            conversation_id: "c1".to_string(),
        };

        let saved = close_turn_record(&pool, &log, &reply_only("Còn phòng 101.")).await;

        assert!(
            !saved,
            "id đi tiếp KHÔNG có nghĩa là đã lưu — hai câu hỏi khác nhau, và \
             gộp chúng là cách làm mất tin nhắn im lặng"
        );
    }

    /// Hàng `kind='action'` là CHỮ, không phải dữ liệu dựng lại được. Bảng cố ý
    /// không có cột chứa nổi `payload` hay `built_at_ms`, và nhét JSON vào
    /// `text` là lách đúng hàng rào ấy: `approve()` cần `action.payload` để bắn
    /// `check_in` — tiền thật.
    #[tokio::test]
    async fn an_action_card_is_logged_as_readable_text_only() {
        let pool = seeded_pool().await;

        close_turn_record(&pool, &existing_record("c1", true), &action_card()).await;

        // Đối chứng cho chính fixture: bản cũ dựng `display` bằng tay nên
        // khẳng định "`payload.room_id` không lọt vào text" xanh — xanh vì
        // fixture chứ không vì code. Fixture giờ dựng bằng hàm sản xuất, và
        // dòng dưới ghim đúng cái sự thật mà khẳng định kia đã nói dối.
        let display = match &action_card().expect("thẻ").proposed_action {
            Some(action) => action.display.clone(),
            None => panic!("fixture phải là thẻ xác nhận"),
        };
        assert_eq!(
            display.get("room_id").map(String::as_str),
            Some("R201"),
            "`build_check_in_display` LUÔN nhét `room_id` vào thẻ (draft.rs:62)"
        );

        let (kind, text) = logged_message(&pool, "c1").await;
        assert_eq!(kind, "action");
        // Chuỗi kỳ vọng cố ý viết ra đủ: nó là câu trả lời cho "sổ chat chứa
        // những gì?". Có `room_id`, có số CCCD, có địa chỉ, có đường dẫn ảnh
        // giấy tờ — **đúng lựa chọn của chủ nhà**, spec dòng 343-346 chốt "Giữ
        // nguyên văn, kể cả số CCCD. Không tự xoá", đã nêu rõ đánh đổi trước
        // khi chốt. Đây KHÔNG phải lỗ rò cần bịt; bịt nó là sửa spec, không
        // phải sửa code. Điều cấm là thứ khác, ba khẳng định bên dưới mới là
        // nó.
        assert_eq!(
            text,
            "Đề xuất nhận phòng:\n\
             - Khách 1: Nguyễn Văn A · CCCD: 012345678901 · SĐT: 0905000111 · \
             Ngày sinh: 1990-01-02 · Địa chỉ: 12 Lê Lợi, Huế · \
             Ảnh giấy tờ: /anh-giay-to/cccd-a.jpg\n\
             - check_in_date: Hôm nay, 04/08/2026\n\
             - check_out_date: 06/08/2026\n\
             - guests: 1 người\n\
             - nights: 2 đêm\n\
             - notes: —\n\
             - paid_amount: 0 ₫\n\
             - pricing_type: nightly\n\
             - rate_override_per_night: —\n\
             - room_id: R201\n\
             - source: —\n\
             - total: 500.000 ₫\n\
             - (cảnh báo) Phòng đang ở trạng thái bẩn, chưa dọn."
        );
        // Cấm: thứ **dựng lại được**. `approve()` cần `action.payload` để bắn
        // `check_in` — tiền thật — nên sổ không được chứa nguyên liệu dựng lại
        // nó, dù là JSON thô hay mốc `built_at_ms` để so trùng.
        assert!(
            !text.contains("built_at_ms"),
            "mốc dựng thẻ lọt vào sổ: {text}"
        );
        assert!(
            !text.contains("preview"),
            "bảng giá tạm tính lọt vào sổ nguyên khối: {text}"
        );
        assert!(
            !text.contains('{') && !text.contains('}'),
            "JSON thô của payload lọt vào text: {text}"
        );
    }

    /// Sổ hội thoại phải gọi đúng tên việc. Một thẻ **đặt phòng trước** ghi vào
    /// sổ thành "Đề xuất nhận phòng" là in lại nguyên văn câu nói dối đã mở ra
    /// cả spec này — lần này nằm trên đĩa, và người mở lại sổ sẽ tin nó.
    ///
    /// Ghim luôn hai dòng ngày: đây là bất biến mới áp cho **mọi** thẻ, và sổ là
    /// bằng chứng duy nhất còn lại sau khi cái thẻ biến mất khỏi màn hình.
    #[tokio::test]
    async fn a_reservation_card_is_logged_as_a_reservation_not_a_check_in() {
        let pool = seeded_pool().await;

        close_turn_record(&pool, &existing_record("c1", true), &reserve_action_card()).await;

        let (kind, text) = logged_message(&pool, "c1").await;
        assert_eq!(kind, "action");
        assert!(
            text.starts_with("Đề xuất đặt phòng trước:"),
            "thẻ đặt phòng bị ghi vào sổ như một lượt nhận phòng: {text}"
        );
        assert!(
            !text.contains("Đề xuất nhận phòng"),
            "sổ không được gọi một lượt đặt phòng là nhận phòng: {text}"
        );
        assert!(
            text.contains("- check_in_date: 08/08/2026")
                && text.contains("- check_out_date: 09/08/2026"),
            "sổ phải giữ đủ hai ngày của thẻ: {text}"
        );
        // Nhãn "Hôm nay" là của thẻ nhận phòng. Trên một thẻ đặt phòng trước nó
        // luôn sai — thẻ ấy chỉ dựng được cho ngày ở tương lai.
        assert!(!text.contains("Hôm nay"), "{text}");
    }

    /// Cạnh thứ ba của cùng một luật: một thẻ **ghi bù** ghi vào sổ dưới tên
    /// "nhận phòng" hay "đặt phòng trước" là in lại nguyên văn câu nói dối đã mở
    /// ra cả spec này, lần này nằm trên đĩa và người mở lại sổ sẽ tin nó.
    #[tokio::test]
    async fn a_backfill_card_is_logged_as_a_backfill_not_a_check_in() {
        let pool = seeded_pool().await;

        close_turn_record(&pool, &existing_record("c1", true), &backfill_action_card()).await;

        let (kind, text) = logged_message(&pool, "c1").await;
        assert_eq!(kind, "action");
        assert!(
            text.starts_with("Đề xuất ghi bù:"),
            "thẻ ghi bù bị ghi vào sổ dưới một tên khác: {text}"
        );
        assert!(
            !text.contains("Đề xuất nhận phòng") && !text.contains("Đề xuất đặt phòng trước"),
            "sổ gọi một lượt ghi bù bằng tên của loại thẻ khác: {text}"
        );
        // Bất biến "mọi thẻ đều hiện ngày nhận và ngày trả" phải sống sót qua sổ
        // — sổ là bằng chứng duy nhất còn lại sau khi thẻ biến mất khỏi màn hình.
        assert!(
            text.contains("- check_in_date: 02/08/2026")
                && text.contains("- check_out_date: Chưa trả phòng (khách còn ở)")
                && text.contains("- expected_checkout_date: 07/08/2026"),
            "sổ mất một dòng ngày của thẻ: {text}"
        );
        // Tiền phòng của ghi bù là **trường payload**, và nó phải nằm trong sổ:
        // đây là con số duy nhất trong ba loại thẻ mà lệnh nhận thẳng từ thẻ.
        assert!(text.contains("- total_price: 600.000 ₫"), "{text}");
        assert!(!text.contains("Hôm nay"), "{text}");
    }

    /// Câu hỏi phải nằm TRƯỚC câu trả lời khi sổ được đọc lại.
    ///
    /// `get_messages` sắp `ORDER BY created_at ASC, id ASC`, mà `id` là UUIDv4
    /// ngẫu nhiên: hoà `created_at` thì thứ tự do bốc thăm, và đo được là
    /// **49,6% số lượt** hiện câu trả lời trước câu hỏi.
    ///
    /// Test này ép cái bốc thăm ấy thành tất định. Sau khi ghi xong, `id` của
    /// câu trả lời bị đặt thành `"a…"` còn của câu hỏi thành `"z…"`; nếu hai
    /// mốc `created_at` bằng nhau thì `id ASC` **chắc chắn** đẩy câu trả lời
    /// lên đầu và test đỏ — không phải may rủi.
    #[tokio::test]
    async fn the_answer_never_sorts_before_the_question() {
        let pool = seeded_pool().await;

        let log = open_turn_record(
            &pool,
            receptionist("u1"),
            None,
            "Tối nay còn phòng nào trống?",
            NOW,
        )
        .await
        .expect("mở lượt");
        let id = log.conversation_id().expect("phải có hội thoại");

        close_turn_record(&pool, &log, &reply_only("Còn phòng 101.")).await;

        for (kind, forced_id) in [("user", "zzzz"), ("assistant", "aaaa")] {
            let changed = sqlx::query(
                "UPDATE assistant_messages SET id = ? WHERE conversation_id = ? AND kind = ?",
            )
            .bind(forced_id)
            .bind(&id)
            .bind(kind)
            .execute(&pool)
            .await
            .expect("ép id");
            assert_eq!(
                changed.rows_affected(),
                1,
                "phải có đúng một hàng `{kind}` để ép id — không thì phép ép \
                 thành vô nghĩa và test xanh giả"
            );
        }

        let messages = conversation_queries::get_messages(&pool, &id)
            .await
            .expect("đọc lại sổ");

        assert_eq!(messages.len(), 2, "một lượt là hai hàng: hỏi và trả lời");
        assert_eq!(
            (messages[0].kind.as_str(), messages[0].text.as_str()),
            ("user", "Tối nay còn phòng nào trống?"),
            "câu trả lời chen lên trước câu hỏi — sổ hội thoại đọc lộn ngược: \
             {messages:?}"
        );
        assert_eq!(
            (messages[1].kind.as_str(), messages[1].text.as_str()),
            ("assistant", "Còn phòng 101.")
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

        close_turn_record(&pool, &existing_record("c1", true), &outcome).await;

        assert_eq!(
            logged_message(&pool, "c1").await,
            (
                "error".to_string(),
                "Trợ lý tra cứu quá nhiều vòng mà chưa ra kết quả.".to_string()
            ),
            "lỗi NGƯỜI DÙNG ghi nguyên văn — câu này viết cho lễ tân đọc"
        );
    }

    /// Hàng `kind='error'` của một sự cố **hệ thống** không được mang chuỗi
    /// sqlx thô.
    ///
    /// `CommandError::system` **giữ nguyên** `message` — chỉ `log_system_error`
    /// mới thay nó bằng câu chung. Đường sống có thật: `draft.rs:311` dựng
    /// `CommandError::system(…, format!("Không đọc được trạng thái phòng:
    /// {error}"))` với `error` là `sqlx::Error`, `run_assistant_turn` cho nó
    /// đi thẳng ra bằng `?`, và nó rơi vào `outcome`. Ghi thẳng `error.message`
    /// là **chôn vĩnh viễn** vào sổ đúng cái chuỗi mà Task 3 và Task 4 tốn hai
    /// vòng duyệt để giữ khỏi màn hình quầy — sổ này Task 6-10 sẽ render.
    #[tokio::test]
    async fn a_system_failure_is_logged_as_the_generic_sentence_not_the_sqlx_string() {
        let pool = seeded_pool().await;
        let outcome: CommandResult<AssistantTurnResponse> = Err(CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            "Không đọc được trạng thái phòng: no such table: rooms",
        ));
        let support_id = outcome
            .as_ref()
            .expect_err("lỗi hệ thống")
            .support_id
            .clone()
            .expect("lỗi hệ thống luôn có mã hỗ trợ");

        close_turn_record(&pool, &existing_record("c1", true), &outcome).await;

        let (kind, text) = logged_message(&pool, "c1").await;
        assert_eq!(kind, "error");
        assert!(
            !text.contains("no such table"),
            "chuỗi sqlx thô nằm vĩnh viễn trong sổ hội thoại: {text}"
        );
        assert!(
            text.starts_with(GENERIC_SYSTEM_ERROR_MESSAGE),
            "sổ phải ghi đúng câu chung mà lễ tân đã thấy trên màn hình: {text}"
        );
        assert!(
            text.contains(&support_id),
            "mất mã hỗ trợ thì lễ tân mở sổ đọc lên, support vẫn không tra được \
             lượt nào đã hỏng: {text}"
        );
    }

    /// Sự cố **hệ thống** ở `assert_can_read` không được giết cả lượt chat.
    ///
    /// Bất đối xứng mà bản cũ để lại: lượt **đầu tiên** sống sót qua DB hỏng
    /// (ca 3a — `record_turn_question` nuốt lỗi, lượt chạy tiếp), lượt **tiếp
    /// theo** thì không, vì `assert_can_read` lỗi `System` đi thẳng ra bằng
    /// `?`. `assert_can_read` tồn tại để bảo vệ đường GHI, không phải để bảo
    /// vệ câu trả lời: không xác minh được quyền thì **không ghi gì cả**, mà
    /// không ghi thì không có rò chéo người dùng nào để mà chặn.
    ///
    /// Đổi tên bảng chứ không `DROP`: đường đọc chủ hội thoại gãy đúng như DB
    /// hỏng thật, nhưng các hàng cũ vẫn đếm được — nếu không đếm được thì vế
    /// "không đẻ hội thoại rác" không khẳng định được.
    // `env_lock` là `std::sync::Mutex` và phải giữ suốt cả test, xem
    // `agent/supervisor.rs`. Sự cố hệ thống ở đây có ghi support log, nên phải
    // chuyển hướng runtime root đi chỗ khác.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn a_system_failure_on_the_owner_check_never_kills_the_turn() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();
        let runtime_root =
            std::env::temp_dir().join(format!("capyinn-assistant-owner-{}", uuid::Uuid::new_v4()));
        std::env::set_var("CAPYINN_RUNTIME_ROOT", &runtime_root);

        let pool = seeded_pool().await;
        sqlx::query("ALTER TABLE assistant_conversations RENAME TO conversations_moved_away")
            .execute(&pool)
            .await
            .expect("giấu bảng hội thoại đi để ép sự cố hệ thống ở cửa quyền");

        let opened = open_turn_record(&pool, receptionist("u1"), Some("c1"), "câu hỏi", NOW).await;

        std::env::remove_var("CAPYINN_RUNTIME_ROOT");

        let opened = opened.expect(
            "sự cố hệ thống ở cửa quyền không được giết lượt chat — lễ tân vẫn \
             phải nhận được câu trả lời",
        );
        match &opened {
            TurnLog::Off { conversation_id } => assert_eq!(
                conversation_id, "c1",
                "sổ đóng vẫn phải mang id cũ đi tiếp, không rơi về None"
            ),
            TurnLog::On(_) => {
                panic!("không xác minh được quyền thì tuyệt đối không được mở sổ")
            }
        }

        close_turn_record(&pool, &opened, &reply_only("Còn phòng 101.")).await;

        assert_eq!(
            count(&pool, "assistant_messages").await,
            0,
            "không xác minh được quyền thì tuyệt đối không ghi hàng nào"
        );
        assert_eq!(
            count(&pool, "conversations_moved_away").await,
            2,
            "không được đẻ hội thoại rác"
        );

        let _ = std::fs::remove_dir_all(&runtime_root);
    }

    /// Vế còn lại của cùng một cửa: lỗi **`User`** (từ chối quyền) vẫn phải nổ
    /// ra ngoài bằng `Err`, KHÔNG được nuốt thành "lượt này không ghi sổ".
    /// Nuốt nó là để một lượt chen ngang chạy tiếp như thường.
    #[tokio::test]
    async fn a_refused_owner_check_is_never_downgraded_to_a_closed_log() {
        let pool = seeded_pool().await;

        let error = open_turn_record(&pool, receptionist("u2"), Some("c1"), "chen ngang", NOW)
            .await
            .expect_err("từ chối quyền phải là lỗi lượt, không phải sổ đóng");

        assert_eq!(error.code, codes::AUTH_FORBIDDEN);
        assert_eq!(error.kind, AppErrorKind::User);
    }

    // ─── Lỗi hệ thống: chuỗi sqlx không được ra tới frontend ───

    /// `conversation_service` dựng `CommandError::system(format!("…: {error}"))`
    /// ở **hai** chỗ, và Task 5 chạm cả hai: `assert_can_read` và
    /// `insert_conversation` bên trong `ensure_conversation`. Chuỗi sqlx thô
    /// phải vào support log và **chỉ** vào đó — `mhm/src/lib/appError/format.ts`
    /// nối thẳng `message` ra UI.
    ///
    /// Cả hai đường nay đều **không** trả `Err`: đường kiểm quyền đóng sổ cho
    /// riêng lượt này (`TurnLog::Off`), đường tạo hội thoại nuốt lỗi thành ca
    /// 3a. Nên vế "không đẩy chuỗi lạ ra frontend" mạnh hơn trước — chẳng có lỗi
    /// nào ra tới frontend cả — nhưng vế "nguyên nhân gốc không được bốc hơi"
    /// thì vẫn phải tự đứng, và nó là hai dòng trong support log.
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
        let closed = open_turn_record(&pool, receptionist("u1"), Some("c1"), "câu hỏi", NOW).await;
        let opened = open_turn_record(&pool, receptionist("u1"), None, "câu hỏi", NOW).await;

        std::env::remove_var("CAPYINN_RUNTIME_ROOT");

        let closed = closed.expect("DB hỏng ở cửa quyền không được giết lượt chat");
        assert!(
            matches!(closed, TurnLog::Off { .. }),
            "không đọc nổi chủ hội thoại thì phải đóng sổ, không được ghi tiếp"
        );

        let opened = opened.expect("ghi hỏng không được biến thành lỗi lượt");
        let TurnLog::On(record) = &opened else {
            panic!("đường tạo mới không đi qua cửa quyền, phải là sổ mở");
        };
        assert_eq!(record.conversation_id, None, "ca 3a");
        assert!(!record.persisted);

        let log = std::fs::read_to_string(
            runtime_root
                .join("diagnostics")
                .join("support-errors.jsonl"),
        )
        .expect("support log phải được ghi");
        assert!(
            log.contains("assistant_turn"),
            "lệnh phải tự khai tên mình, không dùng chung một chuỗi"
        );
        assert!(
            log.contains("SUP-"),
            "thiếu mã hỗ trợ thì dòng log không tra ngược được"
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
            [
                "reply",
                "proposed_action",
                "history",
                "conversation_id",
                "turn_saved"
            ],
            "`turn_saved` là tín hiệu spec dòng 446-447 đòi; bỏ nó đi là để việc \
             mất tin nhắn trở lại im lặng, và `mhm/src/types/assistant.ts` phải \
             đổi theo cùng lúc"
        );
    }
}
