use serde::{Deserialize, Serialize};
use serde_json::Map;
use serde_json::Value;

use crate::app_identity;
use crate::command_failure_log::{self, CommandFailureRecord};
use crate::db_error_monitoring::DbErrorGroup;
use crate::support_log::{self, SupportErrorRecord};

pub mod codes {
    pub const AUTH_INVALID_PIN: &str = "AUTH_INVALID_PIN";
    pub const AUTH_NOT_AUTHENTICATED: &str = "AUTH_NOT_AUTHENTICATED";
    pub const AUTH_FORBIDDEN: &str = "AUTH_FORBIDDEN";
    pub const ROOM_ALREADY_EXISTS: &str = "ROOM_ALREADY_EXISTS";
    pub const ROOM_NOT_FOUND: &str = "ROOM_NOT_FOUND";
    pub const ROOM_DELETE_OCCUPIED: &str = "ROOM_DELETE_OCCUPIED";
    pub const ROOM_DELETE_ACTIVE_BOOKING: &str = "ROOM_DELETE_ACTIVE_BOOKING";
    pub const ROOM_TYPE_ALREADY_EXISTS: &str = "ROOM_TYPE_ALREADY_EXISTS";
    pub const ROOM_TYPE_NOT_FOUND: &str = "ROOM_TYPE_NOT_FOUND";
    pub const ROOM_TYPE_IN_USE: &str = "ROOM_TYPE_IN_USE";
    pub const GROUP_INVALID_ROOM_COUNT: &str = "GROUP_INVALID_ROOM_COUNT";
    pub const GROUP_NOT_ENOUGH_VACANT_ROOMS: &str = "GROUP_NOT_ENOUGH_VACANT_ROOMS";
    pub const GROUP_NOT_FOUND: &str = "GROUP_NOT_FOUND";
    pub const GROUP_CHECKOUT_SELECTION_REQUIRED: &str = "GROUP_CHECKOUT_SELECTION_REQUIRED";
    pub const BOOKING_NOT_FOUND: &str = "BOOKING_NOT_FOUND";
    pub const BOOKING_INVALID_STATE: &str = "BOOKING_INVALID_STATE";
    pub const BOOKING_GUEST_REQUIRED: &str = "BOOKING_GUEST_REQUIRED";
    pub const BOOKING_INVALID_NIGHTS: &str = "BOOKING_INVALID_NIGHTS";
    pub const BOOKING_INVALID_SETTLEMENT_TOTAL: &str = "BOOKING_INVALID_SETTLEMENT_TOTAL";
    pub const AUDIT_INVALID_DATE: &str = "AUDIT_INVALID_DATE";
    pub const AUDIT_DATE_ALREADY_RUN: &str = "AUDIT_DATE_ALREADY_RUN";
    pub const WRITE_TOOL_DISABLED: &str = "WRITE_TOOL_DISABLED";
    pub const APPROVAL_REQUIRED: &str = "APPROVAL_REQUIRED";
    pub const DB_LOCKED_RETRYABLE: &str = "DB_LOCKED_RETRYABLE";
    pub const CONFLICT_ROOM_UNAVAILABLE: &str = "CONFLICT_ROOM_UNAVAILABLE";
    pub const CONFLICT_IDEMPOTENCY_HASH_MISMATCH: &str = "CONFLICT_IDEMPOTENCY_HASH_MISMATCH";
    pub const CONFLICT_DUPLICATE_IN_FLIGHT: &str = "CONFLICT_DUPLICATE_IN_FLIGHT";
    pub const SYSTEM_INTERNAL_ERROR: &str = "SYSTEM_INTERNAL_ERROR";

    pub const ALL: &[&str] = &[
        AUTH_INVALID_PIN,
        AUTH_NOT_AUTHENTICATED,
        AUTH_FORBIDDEN,
        ROOM_ALREADY_EXISTS,
        ROOM_NOT_FOUND,
        ROOM_DELETE_OCCUPIED,
        ROOM_DELETE_ACTIVE_BOOKING,
        ROOM_TYPE_ALREADY_EXISTS,
        ROOM_TYPE_NOT_FOUND,
        ROOM_TYPE_IN_USE,
        GROUP_INVALID_ROOM_COUNT,
        GROUP_NOT_ENOUGH_VACANT_ROOMS,
        GROUP_NOT_FOUND,
        GROUP_CHECKOUT_SELECTION_REQUIRED,
        BOOKING_NOT_FOUND,
        BOOKING_INVALID_STATE,
        BOOKING_GUEST_REQUIRED,
        BOOKING_INVALID_NIGHTS,
        BOOKING_INVALID_SETTLEMENT_TOTAL,
        AUDIT_INVALID_DATE,
        AUDIT_DATE_ALREADY_RUN,
        WRITE_TOOL_DISABLED,
        APPROVAL_REQUIRED,
        DB_LOCKED_RETRYABLE,
        CONFLICT_ROOM_UNAVAILABLE,
        CONFLICT_IDEMPOTENCY_HASH_MISMATCH,
        CONFLICT_DUPLICATE_IN_FLIGHT,
        SYSTEM_INTERNAL_ERROR,
    ];
}

pub const GENERIC_SYSTEM_ERROR_MESSAGE: &str = "Có lỗi hệ thống, vui lòng thử lại";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppErrorKind {
    User,
    System,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandError {
    pub code: String,
    pub message: String,
    pub kind: AppErrorKind,
    pub support_id: Option<String>,
    pub retryable: bool,
    pub request_id: Option<String>,
}

pub type CommandResult<T> = Result<T, CommandError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorrelationIdSource {
    Frontend,
    MissingFallback,
    InvalidFallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveCorrelationId {
    pub value: String,
    pub source: CorrelationIdSource,
    pub rejected_length: Option<usize>,
}

fn ensure_registered_code(code: &'static str) -> &'static str {
    assert!(
        codes::ALL.contains(&code),
        "unregistered command error code: {code}"
    );
    code
}

impl CommandError {
    pub fn user(code: &'static str, message: impl Into<String>) -> Self {
        let code = ensure_registered_code(code);
        Self {
            code: code.to_string(),
            message: message.into(),
            kind: AppErrorKind::User,
            support_id: None,
            retryable: false,
            request_id: None,
        }
    }

    pub fn system(code: &'static str, message: impl Into<String>) -> Self {
        let code = ensure_registered_code(code);
        Self {
            code: code.to_string(),
            message: message.into(),
            kind: AppErrorKind::System,
            support_id: Some(generate_support_id()),
            retryable: false,
            request_id: None,
        }
    }

    pub fn with_support_id(
        code: &'static str,
        message: impl Into<String>,
        support_id: impl Into<String>,
    ) -> Self {
        let code = ensure_registered_code(code);
        Self {
            code: code.to_string(),
            message: message.into(),
            kind: AppErrorKind::System,
            support_id: Some(support_id.into()),
            retryable: false,
            request_id: None,
        }
    }

    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }
}

pub fn generate_support_id() -> String {
    let raw = uuid::Uuid::new_v4().simple().to_string();
    format!("SUP-{}", raw[..8].to_ascii_uppercase())
}

pub fn generate_correlation_id() -> String {
    let raw = uuid::Uuid::new_v4().simple().to_string();
    format!("COR-{}", raw[..8].to_ascii_uppercase())
}

fn is_valid_correlation_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 12
        && &bytes[..4] == b"COR-"
        && bytes[4..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'A'..=b'F'))
}

pub fn normalize_correlation_id(input: Option<String>) -> EffectiveCorrelationId {
    match input {
        Some(value) if is_valid_correlation_id(&value) => EffectiveCorrelationId {
            value,
            source: CorrelationIdSource::Frontend,
            rejected_length: None,
        },
        Some(value) => EffectiveCorrelationId {
            value: generate_correlation_id(),
            source: CorrelationIdSource::InvalidFallback,
            rejected_length: Some(value.len()),
        },
        None => EffectiveCorrelationId {
            value: generate_correlation_id(),
            source: CorrelationIdSource::MissingFallback,
            rejected_length: None,
        },
    }
}

pub fn correlation_context(correlation_id: &str, context: Value) -> Value {
    match context {
        Value::Object(mut object) => {
            object.insert(
                "correlation_id".to_string(),
                Value::String(correlation_id.to_string()),
            );
            Value::Object(object)
        }
        other => {
            let mut object = Map::new();
            object.insert(
                "correlation_id".to_string(),
                Value::String(correlation_id.to_string()),
            );
            object.insert("context".to_string(), other);
            Value::Object(object)
        }
    }
}

pub fn record_command_failure(
    command_name: &str,
    error: &CommandError,
    correlation_id: &str,
    context: Value,
) {
    record_command_failure_with_db_group(command_name, error, correlation_id, None, context);
}

pub fn record_command_failure_with_db_group(
    command_name: &str,
    error: &CommandError,
    correlation_id: &str,
    db_error_group: Option<DbErrorGroup>,
    context: Value,
) {
    let record = CommandFailureRecord::new(
        command_name,
        error.code.clone(),
        error.kind,
        correlation_id,
        error.support_id.clone(),
        db_error_group,
        context,
    );

    if let Some(runtime_root) = app_identity::runtime_root_opt() {
        if let Err(error) =
            command_failure_log::append_command_failure_record(&runtime_root, &record)
        {
            log::error!(
                "failed to write command failure record for {}: {}",
                command_name,
                error
            );
        }
    } else {
        log::error!(
            "unable to resolve runtime root for command failure record from {}",
            command_name
        );
    }
}

pub fn log_system_error(
    command_name: &str,
    root_cause: impl AsRef<str>,
    context: Value,
) -> CommandError {
    let root_cause = root_cause.as_ref();
    let support_id = generate_support_id();
    let record = SupportErrorRecord::new(
        command_name,
        codes::SYSTEM_INTERNAL_ERROR,
        root_cause,
        support_id.clone(),
        context,
    );
    let context_json = serde_json::to_string(&record.context).unwrap_or_else(|_| "{}".to_string());

    log::error!(
        "system error [{}] {}: {} | context={}",
        support_id,
        command_name,
        root_cause,
        context_json
    );

    if let Some(runtime_root) = app_identity::runtime_root_opt() {
        if let Err(error) = support_log::append_support_error_record(&runtime_root, &record) {
            log::error!(
                "failed to write support error record for {}: {}",
                command_name,
                error
            );
        }
    } else {
        log::error!(
            "unable to resolve runtime root for support error record from {}",
            command_name
        );
    }

    CommandError::with_support_id(
        codes::SYSTEM_INTERNAL_ERROR,
        GENERIC_SYSTEM_ERROR_MESSAGE,
        support_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db_error_monitoring::DbErrorGroup;
    use serde_json::json;
    use std::fs;
    use std::path::Path;

    #[test]
    fn user_error_serializes_to_the_expected_contract_shape() {
        let error = CommandError::user(codes::AUTH_INVALID_PIN, "Mã PIN không đúng");
        let serialized = serde_json::to_value(&error).expect("serialize command error");

        assert_eq!(
            serialized,
            json!({
                "code": codes::AUTH_INVALID_PIN,
                "message": "Mã PIN không đúng",
                "kind": "user",
                "support_id": null,
                "retryable": false,
                "request_id": null,
            })
        );
    }

    #[test]
    fn command_error_builders_set_retryable_and_request_id() {
        let error = CommandError::user(codes::DB_LOCKED_RETRYABLE, "Database is locked")
            .retryable(true)
            .with_request_id("REQ-1234");

        assert!(error.retryable);
        assert_eq!(error.request_id.as_deref(), Some("REQ-1234"));
        assert_eq!(error.support_id, None);
    }

    #[test]
    fn system_error_serializes_support_id_in_the_expected_format() {
        let error = CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            "Có lỗi hệ thống, vui lòng thử lại",
        );
        let support_id = error
            .support_id
            .clone()
            .expect("system errors carry support ids");

        assert_eq!(support_id.len(), 12);
        assert!(support_id.starts_with("SUP-"));
        assert!(support_id[4..]
            .chars()
            .all(|character| character.is_ascii_digit() || matches!(character, 'A'..='F')));

        let serialized = serde_json::to_value(&error).expect("serialize command error");
        assert_eq!(serialized["kind"], "system");
        assert_eq!(serialized["support_id"], support_id);
    }

    #[test]
    #[should_panic(expected = "unregistered command error code")]
    fn rejects_unregistered_command_error_code() {
        let _ = CommandError::user("NOT_REGISTERED", "boom");
    }

    #[test]
    fn registry_matches_the_exported_code_constants() {
        #[derive(Deserialize)]
        struct RegistryEntry {
            code: String,
            kind: AppErrorKind,
            #[serde(rename = "defaultMessage")]
            default_message: String,
        }

        let registry_path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../shared/error-codes.json");
        let registry: Vec<RegistryEntry> =
            serde_json::from_str(&fs::read_to_string(&registry_path).expect("read registry"))
                .expect("parse registry");
        let expected = vec![
            (
                codes::AUTH_INVALID_PIN,
                AppErrorKind::User,
                "Mã PIN không đúng",
            ),
            (
                codes::AUTH_NOT_AUTHENTICATED,
                AppErrorKind::User,
                "Chưa đăng nhập",
            ),
            (
                codes::AUTH_FORBIDDEN,
                AppErrorKind::User,
                "Không có quyền thực hiện. Yêu cầu quyền Admin.",
            ),
            (
                codes::ROOM_ALREADY_EXISTS,
                AppErrorKind::User,
                "Phòng đã tồn tại",
            ),
            (
                codes::ROOM_NOT_FOUND,
                AppErrorKind::User,
                "Phòng không tồn tại",
            ),
            (
                codes::ROOM_DELETE_OCCUPIED,
                AppErrorKind::User,
                "Không thể xóa phòng đang có khách",
            ),
            (
                codes::ROOM_DELETE_ACTIVE_BOOKING,
                AppErrorKind::User,
                "Không thể xóa phòng có booking đang hoạt động",
            ),
            (
                codes::ROOM_TYPE_ALREADY_EXISTS,
                AppErrorKind::User,
                "Loại phòng đã tồn tại",
            ),
            (
                codes::ROOM_TYPE_NOT_FOUND,
                AppErrorKind::User,
                "Loại phòng không tồn tại",
            ),
            (
                codes::ROOM_TYPE_IN_USE,
                AppErrorKind::User,
                "Không thể xóa loại phòng đang được sử dụng",
            ),
            (
                codes::GROUP_INVALID_ROOM_COUNT,
                AppErrorKind::User,
                "Số phòng phải lớn hơn 0",
            ),
            (
                codes::GROUP_NOT_ENOUGH_VACANT_ROOMS,
                AppErrorKind::User,
                "Không đủ phòng trống",
            ),
            (
                codes::GROUP_NOT_FOUND,
                AppErrorKind::User,
                "Không tìm thấy đoàn",
            ),
            (
                codes::GROUP_CHECKOUT_SELECTION_REQUIRED,
                AppErrorKind::User,
                "Phải chọn ít nhất 1 phòng để checkout",
            ),
            (
                codes::BOOKING_NOT_FOUND,
                AppErrorKind::User,
                "Không tìm thấy booking",
            ),
            (
                codes::BOOKING_INVALID_STATE,
                AppErrorKind::User,
                "Booking hiện không ở trạng thái hợp lệ cho thao tác này",
            ),
            (
                codes::BOOKING_GUEST_REQUIRED,
                AppErrorKind::User,
                "Phải có ít nhất 1 khách",
            ),
            (
                codes::BOOKING_INVALID_NIGHTS,
                AppErrorKind::User,
                "Số đêm phải lớn hơn 0",
            ),
            (
                codes::BOOKING_INVALID_SETTLEMENT_TOTAL,
                AppErrorKind::User,
                "Tổng quyết toán phải lớn hơn hoặc bằng 0",
            ),
            (
                codes::AUDIT_INVALID_DATE,
                AppErrorKind::User,
                "Ngày kiểm toán không hợp lệ",
            ),
            (
                codes::AUDIT_DATE_ALREADY_RUN,
                AppErrorKind::User,
                "Ngày kiểm toán này đã được chạy",
            ),
            (
                codes::WRITE_TOOL_DISABLED,
                AppErrorKind::User,
                "Thao tác ghi qua MCP đang bị tắt.",
            ),
            (
                codes::APPROVAL_REQUIRED,
                AppErrorKind::User,
                "Thao tác này cần phê duyệt.",
            ),
            (
                codes::DB_LOCKED_RETRYABLE,
                AppErrorKind::System,
                "Database đang bận, vui lòng thử lại.",
            ),
            (
                codes::CONFLICT_ROOM_UNAVAILABLE,
                AppErrorKind::User,
                "Phòng không còn trống trong khoảng ngày đã chọn.",
            ),
            (
                codes::CONFLICT_IDEMPOTENCY_HASH_MISMATCH,
                AppErrorKind::User,
                "Idempotency key đã được dùng cho payload khác.",
            ),
            (
                codes::CONFLICT_DUPLICATE_IN_FLIGHT,
                AppErrorKind::User,
                "Lệnh đang được xử lý.",
            ),
            (
                codes::SYSTEM_INTERNAL_ERROR,
                AppErrorKind::System,
                GENERIC_SYSTEM_ERROR_MESSAGE,
            ),
        ];

        assert_eq!(registry.len(), expected.len());
        for (entry, (code, kind, default_message)) in registry.iter().zip(expected.iter()) {
            assert_eq!(entry.code, *code);
            assert_eq!(entry.kind, *kind);
            assert_eq!(entry.default_message, *default_message);
        }
        assert!(registry
            .iter()
            .all(|entry| !entry.default_message.is_empty()));
    }

    #[test]
    fn log_system_error_returns_generic_system_contract_and_writes_support_record() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();
        let runtime_root =
            std::env::temp_dir().join(format!("capyinn-log-system-error-{}", uuid::Uuid::new_v4()));

        std::env::set_var("CAPYINN_RUNTIME_ROOT", &runtime_root);
        let error = log_system_error(
            "login",
            "database offline",
            json!({ "room_id": "R101", "booking_id": "B202" }),
        );
        std::env::remove_var("CAPYINN_RUNTIME_ROOT");

        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
        assert_eq!(error.message, GENERIC_SYSTEM_ERROR_MESSAGE);
        assert_eq!(error.kind, AppErrorKind::System);
        let support_id = error.support_id.expect("support id");
        assert!(support_id.starts_with("SUP-"));

        let log_path = runtime_root
            .join("diagnostics")
            .join("support-errors.jsonl");
        let contents = fs::read_to_string(&log_path).expect("support log contents");
        let parsed: serde_json::Value =
            serde_json::from_str(contents.trim()).expect("support log json");
        assert_eq!(parsed["command"], "login");
        assert_eq!(parsed["code"], codes::SYSTEM_INTERNAL_ERROR);
        assert_eq!(parsed["root_cause"], "database offline");
        assert_eq!(parsed["context"]["room_id"], "R101");
        assert_eq!(parsed["context"]["booking_id"], "B202");
        assert!(parsed.get("room_id").is_none());
        assert!(parsed.get("booking_id").is_none());
        assert_eq!(parsed["support_id"], support_id);

        let _ = fs::remove_dir_all(&runtime_root);
    }

    #[test]
    fn record_command_failure_writes_normalized_user_failure_record() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();
        let runtime_root = std::env::temp_dir().join(format!(
            "capyinn-command-failure-user-{}",
            uuid::Uuid::new_v4()
        ));

        std::env::set_var("CAPYINN_RUNTIME_ROOT", &runtime_root);
        let error = CommandError::user(codes::BOOKING_GUEST_REQUIRED, "Phải có ít nhất 1 khách");
        record_command_failure(
            "check_in",
            &error,
            "COR-1A2B3C4D",
            json!({
                "room_id": "R101",
                "guest_count": 1,
                "nights": 2,
            }),
        );
        std::env::remove_var("CAPYINN_RUNTIME_ROOT");

        let log_path = runtime_root
            .join("diagnostics")
            .join("command-failures.jsonl");
        let contents = fs::read_to_string(&log_path).expect("command failure log contents");
        let parsed: serde_json::Value =
            serde_json::from_str(contents.trim()).expect("command failure json");

        assert_eq!(parsed["schema_version"], 2);
        assert_eq!(parsed["command"], "check_in");
        assert_eq!(parsed["code"], codes::BOOKING_GUEST_REQUIRED);
        assert_eq!(parsed["kind"], "user");
        assert_eq!(parsed["correlation_id"], "COR-1A2B3C4D");
        assert!(parsed["support_id"].is_null());
        assert!(parsed.get("db_error_group").is_none());
        assert_eq!(parsed["context"]["room_id"], "R101");
        assert_eq!(parsed["context"]["guest_count"], 1);
        assert_eq!(parsed["context"]["nights"], 2);
        assert!(parsed["context"].get("correlation_id").is_none());
        assert!(parsed.get("room_id").is_none());
        assert!(parsed.get("root_cause").is_none());

        let _ = fs::remove_dir_all(&runtime_root);
    }

    #[test]
    fn record_command_failure_preserves_system_support_id() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();
        let runtime_root = std::env::temp_dir().join(format!(
            "capyinn-command-failure-system-{}",
            uuid::Uuid::new_v4()
        ));

        std::env::set_var("CAPYINN_RUNTIME_ROOT", &runtime_root);
        let error = CommandError::system(codes::SYSTEM_INTERNAL_ERROR, "database offline");
        let support_id = error.support_id.clone().expect("system error support id");
        record_command_failure(
            "create_reservation",
            &error,
            "COR-5A6B7C8D",
            json!({
                "room_id": "R202",
                "check_in_date": "2026-04-22",
                "check_out_date": "2026-04-24",
                "nights": 2,
                "deposit_present": false,
                "source": "phone",
                "notes_present": false,
            }),
        );
        std::env::remove_var("CAPYINN_RUNTIME_ROOT");

        let log_path = runtime_root
            .join("diagnostics")
            .join("command-failures.jsonl");
        let contents = fs::read_to_string(&log_path).expect("command failure log contents");
        let parsed: serde_json::Value =
            serde_json::from_str(contents.trim()).expect("command failure json");

        assert_eq!(parsed["schema_version"], 2);
        assert_eq!(parsed["command"], "create_reservation");
        assert_eq!(parsed["code"], codes::SYSTEM_INTERNAL_ERROR);
        assert_eq!(parsed["kind"], "system");
        assert_eq!(parsed["correlation_id"], "COR-5A6B7C8D");
        assert_eq!(parsed["support_id"], support_id);
        assert!(parsed.get("db_error_group").is_none());
        assert_eq!(parsed["context"]["room_id"], "R202");
        assert_eq!(parsed["context"]["check_in_date"], "2026-04-22");
        assert_eq!(parsed["context"]["check_out_date"], "2026-04-24");
        assert_eq!(parsed["context"]["nights"], 2);
        assert_eq!(parsed["context"]["deposit_present"], false);
        assert_eq!(parsed["context"]["source"], "phone");
        assert_eq!(parsed["context"]["notes_present"], false);
        assert!(parsed["context"].get("correlation_id").is_none());
        assert!(parsed.get("room_id").is_none());
        assert!(parsed.get("root_cause").is_none());

        let _ = fs::remove_dir_all(&runtime_root);
    }

    #[test]
    fn record_command_failure_with_db_group_writes_group_without_touching_existing_shape() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();
        let runtime_root = std::env::temp_dir().join(format!(
            "capyinn-command-failure-grouped-{}",
            uuid::Uuid::new_v4()
        ));

        std::env::set_var("CAPYINN_RUNTIME_ROOT", &runtime_root);
        let error = CommandError::system(codes::SYSTEM_INTERNAL_ERROR, "database offline");
        let support_id = error.support_id.clone().expect("system error support id");
        record_command_failure_with_db_group(
            "create_reservation",
            &error,
            "COR-9A8B7C6D",
            Some(DbErrorGroup::Locked),
            json!({
                "room_id": "R303",
                "source": "walk_in",
            }),
        );
        std::env::remove_var("CAPYINN_RUNTIME_ROOT");

        let log_path = runtime_root
            .join("diagnostics")
            .join("command-failures.jsonl");
        let contents = fs::read_to_string(&log_path).expect("command failure log contents");
        let parsed: serde_json::Value =
            serde_json::from_str(contents.trim()).expect("command failure json");

        assert_eq!(parsed["schema_version"], 2);
        assert_eq!(parsed["command"], "create_reservation");
        assert_eq!(parsed["code"], codes::SYSTEM_INTERNAL_ERROR);
        assert_eq!(parsed["kind"], "system");
        assert_eq!(parsed["correlation_id"], "COR-9A8B7C6D");
        assert_eq!(parsed["support_id"], support_id);
        assert_eq!(parsed["db_error_group"], "locked");
        assert_eq!(parsed["context"]["room_id"], "R303");
        assert_eq!(parsed["context"]["source"], "walk_in");
        assert!(parsed.get("room_id").is_none());
        assert!(parsed.get("root_cause").is_none());

        let _ = fs::remove_dir_all(&runtime_root);
    }

    #[test]
    fn normalize_correlation_id_preserves_valid_frontend_value() {
        let effective = normalize_correlation_id(Some("COR-1A2B3C4D".to_string()));

        assert_eq!(effective.value, "COR-1A2B3C4D");
        assert_eq!(effective.source, CorrelationIdSource::Frontend);
        assert_eq!(effective.rejected_length, None);
    }

    #[test]
    fn normalize_correlation_id_generates_fallback_for_missing_value() {
        let effective = normalize_correlation_id(None);

        assert_eq!(effective.source, CorrelationIdSource::MissingFallback);
        assert_eq!(effective.rejected_length, None);
        assert_eq!(effective.value.len(), 12);
        assert!(effective.value.starts_with("COR-"));
        assert!(effective.value[4..]
            .chars()
            .all(|character: char| character.is_ascii_digit() || matches!(character, 'A'..='F')));
    }

    #[test]
    fn normalize_correlation_id_replaces_invalid_input_without_echoing_it() {
        let raw_input = "not-a-real-correlation-id";
        let effective = normalize_correlation_id(Some(raw_input.to_string()));

        assert_eq!(effective.source, CorrelationIdSource::InvalidFallback);
        assert_eq!(effective.rejected_length, Some(raw_input.len()));
        assert_eq!(effective.value.len(), 12);
        assert!(effective.value.starts_with("COR-"));
        assert_ne!(effective.value, raw_input);
        assert!(!effective.value.contains(raw_input));
    }

    #[test]
    fn correlation_context_merges_correlation_id_without_dropping_existing_fields() {
        let merged = correlation_context(
            "COR-1A2B3C4D",
            json!({ "room_id": "R101", "booking_id": "B202" }),
        );

        assert_eq!(
            merged,
            json!({
                "correlation_id": "COR-1A2B3C4D",
                "room_id": "R101",
                "booking_id": "B202",
            })
        );
    }
}
