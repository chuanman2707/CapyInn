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
            provider::{build_assistant_provider_client, AssistantProviderClient},
            run_assistant_turn, AssistantTurnRequest, AssistantTurnResponse,
        },
        secrets::{AgentSecretKind, AgentSecretStore, KeychainSecretStore},
    },
    app_error::{codes, CommandError, CommandResult},
};
use serde::Serialize;
use tauri::State;

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

#[tauri::command]
pub async fn assistant_turn(
    state: State<'_, AppState>,
    request: AssistantTurnRequest,
) -> CommandResult<AssistantTurnResponse> {
    if get_user(&state).is_none() {
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
    let provider = AssistantProviderClient::new(build_assistant_provider_client()?);

    run_assistant_turn(
        &state.db,
        &provider,
        &config,
        &api_key,
        request,
        &now_local_date,
    )
    .await
}
