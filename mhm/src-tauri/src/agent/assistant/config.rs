use crate::{
    app_error::{codes, CommandError, CommandResult},
    services::settings_store,
};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};

pub const ASSISTANT_CONFIG_SETTING: &str = "assistant_config";
pub const ASSISTANT_CLOUD_DATA_OPT_IN_SETTING: &str = "assistant_cloud_data_opt_in";

pub const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com/v1/chat/completions";
pub const DEEPSEEK_MODEL: &str = "deepseek-chat";
pub const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1/chat/completions";

const MAX_BASE_URL_CHARS: usize = 2_048;
const MAX_MODEL_CHARS: usize = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantPreset {
    DeepSeek,
    OpenRouter,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantConfig {
    pub preset: AssistantPreset,
    pub base_url: String,
    pub model: String,
}

impl AssistantConfig {
    pub fn default_for(preset: AssistantPreset) -> Self {
        match preset {
            AssistantPreset::DeepSeek => Self {
                preset,
                base_url: DEEPSEEK_BASE_URL.to_string(),
                model: DEEPSEEK_MODEL.to_string(),
            },
            AssistantPreset::OpenRouter => Self {
                preset,
                base_url: OPENROUTER_BASE_URL.to_string(),
                model: String::new(),
            },
            AssistantPreset::Custom => Self {
                preset,
                base_url: String::new(),
                model: String::new(),
            },
        }
    }
}

impl Default for AssistantConfig {
    fn default() -> Self {
        Self::default_for(AssistantPreset::DeepSeek)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantGateMissing {
    ApiKey,
    CloudDataOptIn,
    Model,
    BaseUrl,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssistantGateStatus {
    pub ready: bool,
    pub missing: Vec<AssistantGateMissing>,
}

pub fn evaluate_assistant_gate(
    config: &AssistantConfig,
    has_api_key: bool,
    opt_in: bool,
) -> AssistantGateStatus {
    let mut missing = Vec::new();

    if config.base_url.trim().is_empty() {
        missing.push(AssistantGateMissing::BaseUrl);
    }
    if config.model.trim().is_empty() {
        missing.push(AssistantGateMissing::Model);
    }
    if !has_api_key {
        missing.push(AssistantGateMissing::ApiKey);
    }
    if !opt_in {
        missing.push(AssistantGateMissing::CloudDataOptIn);
    }

    AssistantGateStatus {
        ready: missing.is_empty(),
        missing,
    }
}

/// Cổng thứ nhất, đứng trước `evaluate_assistant_gate`: cờ runtime thí nghiệm.
///
/// `evaluate_assistant_gate` chỉ hỏi "chủ nhà đã cấu hình xong chưa". Cái này
/// hỏi "bản dựng này có được phép chạy trợ lý không" — và câu trả lời phải
/// giống hệt câu mà màn hình cài đặt đã dùng để quyết định hiện hay ẩn tab
/// (`pages/settings/index.tsx` đọc `agentRuntimeEnabled`, tức chính
/// `effective_experimental_agent_runtime_enabled`). Hai bên lệch nhau nghĩa là
/// gỡ cờ đi thì mất tab cài đặt trong khi panel vẫn gọi nhà cung cấp và vẫn
/// đẩy dữ liệu khách lên cloud — mà không còn giao diện nào để tắt cái opt-in
/// đang bật. Đó là ngược hẳn công dụng của cờ.
///
/// Lưu ý: `effective_…` cõng luôn override `CAPYINN_DISABLE_CEO_TELEGRAM`, một
/// cái tên buộc tính năng này vào một tính năng không liên quan. Đã ghi nhận
/// là nợ, cố ý **không** đi nối lại dây ở đây — việc cần làm là hai bên đọc
/// cùng một cờ, không phải đổi cờ.
pub fn ensure_agent_runtime_enabled() -> CommandResult<()> {
    if crate::runtime_config::effective_experimental_agent_runtime_enabled() {
        return Ok(());
    }

    Err(CommandError::user(
        codes::AGENT_RUNTIME_DISABLED,
        "Trợ lý quầy đang tắt trên bản dựng này.",
    ))
}

pub fn validate_assistant_base_url(raw: &str) -> CommandResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(invalid_input("Chưa nhập địa chỉ máy chủ AI."));
    }
    if trimmed.chars().count() > MAX_BASE_URL_CHARS {
        return Err(invalid_input("Địa chỉ máy chủ AI quá dài."));
    }

    // Repo không có crate `url` riêng — dùng lại `reqwest::Url`, đúng như
    // `agent/receptionist_demo.rs:265` đang làm. Đừng thêm dependency thứ hai.
    let url = reqwest::Url::parse(trimmed)
        .map_err(|_| invalid_input("Địa chỉ máy chủ AI không hợp lệ."))?;
    let host = url.host_str().unwrap_or_default();
    let is_loopback = host == "127.0.0.1" || host == "localhost" || host == "::1";

    match url.scheme() {
        "https" => Ok(trimmed.to_string()),
        "http" if is_loopback => Ok(trimmed.to_string()),
        "http" => Err(invalid_input(
            "Địa chỉ ra ngoài máy phải dùng https, không dùng http.",
        )),
        _ => Err(invalid_input("Địa chỉ máy chủ AI phải là http hoặc https.")),
    }
}

pub fn validate_assistant_model(raw: &str) -> CommandResult<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(invalid_input("Chưa nhập tên model."));
    }
    if trimmed.chars().count() > MAX_MODEL_CHARS {
        return Err(invalid_input("Tên model quá dài."));
    }
    if trimmed.chars().any(char::is_control) {
        return Err(invalid_input("Tên model chứa ký tự không hợp lệ."));
    }
    Ok(trimmed.to_string())
}

pub async fn get_assistant_config(pool: &Pool<Sqlite>) -> CommandResult<AssistantConfig> {
    let raw = settings_store::get_setting(pool, ASSISTANT_CONFIG_SETTING)
        .await
        .map_err(read_settings_error)?;

    let Some(raw) = raw else {
        return Ok(AssistantConfig::default());
    };

    // Cấu hình hỏng không được làm chết trợ lý: rơi về mặc định để người dùng
    // còn vào màn hình cài đặt sửa lại được.
    Ok(serde_json::from_str(&raw).unwrap_or_default())
}

pub async fn save_assistant_config(
    pool: &Pool<Sqlite>,
    config: &AssistantConfig,
) -> CommandResult<()> {
    let encoded = serde_json::to_string(config).map_err(|error| {
        CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            format!("Failed to encode assistant config: {error}"),
        )
    })?;

    settings_store::save_setting(pool, ASSISTANT_CONFIG_SETTING, &encoded)
        .await
        .map_err(write_settings_error)
}

pub async fn get_assistant_cloud_data_opt_in(pool: &Pool<Sqlite>) -> CommandResult<bool> {
    let value = settings_store::get_setting(pool, ASSISTANT_CLOUD_DATA_OPT_IN_SETTING)
        .await
        .map_err(read_settings_error)?;

    Ok(value
        .as_deref()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            normalized == "true" || normalized == "1"
        })
        .unwrap_or(false))
}

pub async fn set_assistant_cloud_data_opt_in(
    pool: &Pool<Sqlite>,
    enabled: bool,
) -> CommandResult<()> {
    settings_store::save_setting(
        pool,
        ASSISTANT_CLOUD_DATA_OPT_IN_SETTING,
        if enabled { "true" } else { "false" },
    )
    .await
    .map_err(write_settings_error)
}

fn invalid_input(message: &str) -> CommandError {
    CommandError::user(codes::VALIDATION_INVALID_INPUT, message)
}

fn read_settings_error(error: String) -> CommandError {
    CommandError::system(
        codes::SYSTEM_INTERNAL_ERROR,
        format!("Failed to read assistant setting: {error}"),
    )
}

fn write_settings_error(error: String) -> CommandError {
    CommandError::system(
        codes::SYSTEM_INTERNAL_ERROR,
        format!("Failed to save assistant setting: {error}"),
    )
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
        crate::db::run_migrations(&pool)
            .await
            .expect("failed to run migrations");
        pool
    }

    #[tokio::test]
    async fn missing_config_falls_back_to_the_deepseek_preset() {
        let pool = test_pool().await;

        let config = get_assistant_config(&pool).await.expect("read config");

        assert_eq!(config.preset, AssistantPreset::DeepSeek);
        assert_eq!(
            config.base_url,
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(config.model, "deepseek-chat");
    }

    #[tokio::test]
    async fn config_round_trips_through_settings() {
        let pool = test_pool().await;
        let config = AssistantConfig {
            preset: AssistantPreset::OpenRouter,
            base_url: "https://openrouter.ai/api/v1/chat/completions".to_string(),
            model: "deepseek/deepseek-chat".to_string(),
        };

        save_assistant_config(&pool, &config).await.expect("save");

        assert_eq!(get_assistant_config(&pool).await.expect("read"), config);
    }

    #[tokio::test]
    async fn opt_in_defaults_to_false_and_round_trips() {
        let pool = test_pool().await;

        assert!(!get_assistant_cloud_data_opt_in(&pool).await.expect("read"));

        set_assistant_cloud_data_opt_in(&pool, true)
            .await
            .expect("enable");
        assert!(get_assistant_cloud_data_opt_in(&pool).await.expect("read"));

        set_assistant_cloud_data_opt_in(&pool, false)
            .await
            .expect("revoke");
        assert!(!get_assistant_cloud_data_opt_in(&pool).await.expect("read"));
    }

    #[test]
    fn base_url_must_be_https_unless_it_is_loopback() {
        assert!(
            validate_assistant_base_url("https://api.deepseek.com/v1/chat/completions").is_ok()
        );
        assert!(validate_assistant_base_url("http://127.0.0.1:8080/v1/chat/completions").is_ok());
        assert!(validate_assistant_base_url("http://localhost:8080/v1/chat/completions").is_ok());

        let error = validate_assistant_base_url("http://api.deepseek.com/v1/chat/completions")
            .expect_err("http tới host xa phải bị chặn");
        assert_eq!(error.code, codes::VALIDATION_INVALID_INPUT);

        assert!(validate_assistant_base_url("").is_err());
        assert!(validate_assistant_base_url("khong-phai-url").is_err());
        assert!(validate_assistant_base_url("ftp://example.com/x").is_err());
    }

    #[test]
    fn gate_is_closed_until_key_and_opt_in_are_both_present() {
        let config = AssistantConfig::default_for(AssistantPreset::DeepSeek);

        let no_key = evaluate_assistant_gate(&config, false, true);
        assert!(!no_key.ready);
        assert!(no_key.missing.contains(&AssistantGateMissing::ApiKey));

        let no_opt_in = evaluate_assistant_gate(&config, true, false);
        assert!(!no_opt_in.ready);
        assert!(no_opt_in
            .missing
            .contains(&AssistantGateMissing::CloudDataOptIn));

        let ready = evaluate_assistant_gate(&config, true, true);
        assert!(ready.ready);
        assert!(ready.missing.is_empty());
    }

    #[test]
    fn gate_reports_a_blank_model_as_missing() {
        let config = AssistantConfig {
            preset: AssistantPreset::Custom,
            base_url: "https://example.com/v1/chat/completions".to_string(),
            model: "   ".to_string(),
        };

        let status = evaluate_assistant_gate(&config, true, true);

        assert!(!status.ready);
        assert!(status.missing.contains(&AssistantGateMissing::Model));
    }

    /// Cờ `CAPYINN_EXPERIMENTAL_AGENT_RUNTIME` là công tắc tắt của cả tính
    /// năng. Gỡ nó ra mà trợ lý vẫn gọi được nhà cung cấp thì công tắc chỉ giấu
    /// đi màn hình cài đặt — tức lấy mất luôn chỗ để tắt cái opt-in đang bật.
    #[test]
    fn the_assistant_refuses_to_run_when_the_agent_runtime_flag_is_removed() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();

        for name in [
            "CAPYINN_EXPERIMENTAL_RUNTIME",
            "CAPYINN_EXPERIMENTAL_AGENT_RUNTIME",
            "CAPYINN_DISABLE_CEO_TELEGRAM",
        ] {
            std::env::remove_var(name);
        }

        let error = ensure_agent_runtime_enabled().expect_err("cờ tắt thì phải từ chối");
        assert_eq!(error.code, codes::AGENT_RUNTIME_DISABLED);

        std::env::set_var("CAPYINN_EXPERIMENTAL_AGENT_RUNTIME", "true");
        assert!(ensure_agent_runtime_enabled().is_ok());
        std::env::remove_var("CAPYINN_EXPERIMENTAL_AGENT_RUNTIME");
    }

    /// Trợ lý phải đọc **đúng cờ hiệu dụng** mà màn hình cài đặt đã đọc, kể cả
    /// cái override đặt tên nhầm sang tính năng khác
    /// (`CAPYINN_DISABLE_CEO_TELEGRAM`, xem `runtime_config.rs:37`). Không
    /// phải để tán thành cách đặt tên đó — mà để hai bên không bao giờ bất
    /// đồng: cửa sổ cài đặt biến mất trong khi panel vẫn gửi dữ liệu khách lên
    /// cloud đúng là cái lỗ hổng này sinh ra.
    #[test]
    fn the_assistant_honours_the_same_disable_override_the_settings_screen_honours() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();

        std::env::set_var("CAPYINN_EXPERIMENTAL_AGENT_RUNTIME", "true");
        std::env::set_var("CAPYINN_DISABLE_CEO_TELEGRAM", "true");

        let error = ensure_agent_runtime_enabled().expect_err("override tắt thì phải từ chối");
        assert_eq!(error.code, codes::AGENT_RUNTIME_DISABLED);
        assert!(
            !crate::runtime_config::effective_experimental_agent_runtime_enabled(),
            "trợ lý và màn hình cài đặt phải cùng đọc một cờ hiệu dụng"
        );

        std::env::remove_var("CAPYINN_EXPERIMENTAL_AGENT_RUNTIME");
        std::env::remove_var("CAPYINN_DISABLE_CEO_TELEGRAM");
    }

    #[test]
    fn gate_reports_a_blank_base_url_as_missing() {
        let config = AssistantConfig::default_for(AssistantPreset::Custom);

        let status = evaluate_assistant_gate(&config, true, true);

        assert!(!status.ready);
        assert!(status.missing.contains(&AssistantGateMissing::BaseUrl));
        assert!(status.missing.contains(&AssistantGateMissing::Model));
    }
}
