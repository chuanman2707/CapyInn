use serde::Serialize;

use crate::app_error::codes;
use crate::write_manifest::LockDeriverId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum McpAutonomyLevel {
    ReadOnly,
    Supervised,
    Full,
}

impl McpAutonomyLevel {
    fn parse(value: &str) -> Self {
        match value {
            "read_only" | "readonly" | "read-only" => Self::ReadOnly,
            "supervised" => Self::Supervised,
            "full" => Self::Full,
            _ => Self::Supervised,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct WriteToolMeta {
    pub command_name: &'static str,
    pub risk_level: RiskLevel,
    pub requires_approval: bool,
    pub requires_idempotency_key: bool,
    pub lock_deriver: &'static str,
    pub description: &'static str,
}

pub const CREATE_RESERVATION_META: WriteToolMeta = WriteToolMeta {
    command_name: "create_reservation",
    risk_level: RiskLevel::High,
    requires_approval: true,
    requires_idempotency_key: true,
    lock_deriver: LockDeriverId::RoomFromRequest.policy_name(),
    description: "Creates a reservation booking row and reserves room calendar dates.",
};

pub const CANCEL_RESERVATION_META: WriteToolMeta = WriteToolMeta {
    command_name: "cancel_reservation",
    risk_level: RiskLevel::High,
    requires_approval: true,
    requires_idempotency_key: true,
    lock_deriver: LockDeriverId::ReservationBookingAndRoom.policy_name(),
    description: "Cancels an existing booked reservation.",
};

pub const MODIFY_RESERVATION_META: WriteToolMeta = WriteToolMeta {
    command_name: "modify_reservation",
    risk_level: RiskLevel::High,
    requires_approval: true,
    requires_idempotency_key: true,
    lock_deriver: LockDeriverId::ReservationBookingAndRoom.policy_name(),
    description: "Changes an existing booked reservation's scheduled dates.",
};

pub const WRITE_TOOL_MANIFEST: [WriteToolMeta; 3] = [
    CREATE_RESERVATION_META,
    CANCEL_RESERVATION_META,
    MODIFY_RESERVATION_META,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpToolErrorEnvelope {
    pub ok: bool,
    pub error: McpToolError,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct McpToolError {
    pub code: &'static str,
    pub kind: &'static str,
    pub message: String,
    pub tool: &'static str,
    pub risk_level: RiskLevel,
    pub retryable: bool,
    pub request_id: Option<String>,
}

impl McpToolErrorEnvelope {
    fn write_tool_disabled(meta: &WriteToolMeta, request_id: Option<String>) -> Self {
        Self {
            ok: false,
            error: McpToolError {
                code: codes::WRITE_TOOL_DISABLED,
                kind: "policy",
                message: format!(
                    "MCP write tool '{}' is disabled by the current autonomy policy.",
                    meta.command_name
                ),
                tool: meta.command_name,
                risk_level: meta.risk_level,
                retryable: false,
                request_id,
            },
        }
    }

    fn approval_required(meta: &WriteToolMeta, request_id: Option<String>) -> Self {
        Self {
            ok: false,
            error: McpToolError {
                code: codes::APPROVAL_REQUIRED,
                kind: "policy",
                message: format!(
                    "MCP write tool '{}' requires human approval in supervised mode.",
                    meta.command_name
                ),
                tool: meta.command_name,
                risk_level: meta.risk_level,
                retryable: false,
                request_id,
            },
        }
    }

    pub fn to_json_string(&self) -> String {
        serde_json::to_string_pretty(self).expect("MCP error envelope should serialize")
    }
}

pub fn configured_mcp_autonomy_level() -> McpAutonomyLevel {
    std::env::var("CAPYINN_MCP_AUTONOMY")
        .map(|value| McpAutonomyLevel::parse(&value))
        .unwrap_or(McpAutonomyLevel::Supervised)
}

pub fn guard_write_tool(
    meta: &WriteToolMeta,
    request_id: Option<String>,
) -> Result<(), McpToolErrorEnvelope> {
    if meta.risk_level != RiskLevel::High {
        return Ok(());
    }

    match configured_mcp_autonomy_level() {
        McpAutonomyLevel::ReadOnly | McpAutonomyLevel::Full => {
            Err(McpToolErrorEnvelope::write_tool_disabled(meta, request_id))
        }
        McpAutonomyLevel::Supervised => {
            Err(McpToolErrorEnvelope::approval_required(meta, request_id))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<String>,
    }

    impl EnvVarGuard {
        fn unset(key: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key, previous }
        }

        fn set(key: &'static str, value: &'static str) -> Self {
            let previous = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(value) = &self.previous {
                std::env::set_var(self.key, value);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn write_tool_manifest_carries_required_policy_metadata() {
        assert_eq!(WRITE_TOOL_MANIFEST.len(), 3);

        for meta in WRITE_TOOL_MANIFEST {
            assert!(!meta.command_name.is_empty());
            assert_eq!(meta.risk_level, RiskLevel::High);
            assert!(meta.requires_approval);
            assert!(meta.requires_idempotency_key);
            assert!(!meta.lock_deriver.is_empty());
        }
    }

    #[test]
    fn write_tool_manifest_lock_derivers_match_write_manifest() {
        for meta in WRITE_TOOL_MANIFEST {
            let write_meta = crate::write_manifest::meta_for(meta.command_name)
                .expect("gateway write tool must exist in write manifest");

            assert_eq!(
                meta.lock_deriver,
                write_meta.lock_deriver.policy_name(),
                "lock deriver mismatch for {}",
                meta.command_name
            );
        }
    }

    #[test]
    fn default_autonomy_is_supervised_when_autonomy_and_legacy_env_are_absent() {
        let _lock = env_lock().lock().expect("env lock poisoned");
        let _autonomy = EnvVarGuard::unset("CAPYINN_MCP_AUTONOMY");
        let _legacy = EnvVarGuard::unset("CAPYINN_ENABLE_HIGH_RISK_MCP_WRITES");

        assert_eq!(
            configured_mcp_autonomy_level(),
            McpAutonomyLevel::Supervised
        );
    }

    #[test]
    fn read_only_high_risk_write_returns_disabled_with_request_id() {
        let _lock = env_lock().lock().expect("env lock poisoned");
        let _autonomy = EnvVarGuard::set("CAPYINN_MCP_AUTONOMY", "read_only");
        let request_id = Some("req-read-only".to_string());

        let err = guard_write_tool(&CREATE_RESERVATION_META, request_id.clone())
            .expect_err("read-only mode should disable high-risk writes");

        assert_eq!(err.error.code, codes::WRITE_TOOL_DISABLED);
        assert_eq!(err.error.request_id, request_id);
    }

    #[test]
    fn supervised_high_risk_write_returns_approval_required_with_request_id() {
        let _lock = env_lock().lock().expect("env lock poisoned");
        let _autonomy = EnvVarGuard::set("CAPYINN_MCP_AUTONOMY", "supervised");
        let request_id = Some("req-supervised".to_string());

        let err = guard_write_tool(&CREATE_RESERVATION_META, request_id.clone())
            .expect_err("supervised mode should require approval for high-risk writes");

        assert_eq!(err.error.code, codes::APPROVAL_REQUIRED);
        assert_eq!(err.error.request_id, request_id);
    }

    #[test]
    fn paired_future_client_context_still_requires_policy_approval() {
        let _lock = env_lock().lock().expect("env lock poisoned");
        let _autonomy = EnvVarGuard::set("CAPYINN_MCP_AUTONOMY", "supervised");
        let request_id = Some("paired:telegram-accounting-agent:req-1".to_string());

        let err = guard_write_tool(&CREATE_RESERVATION_META, request_id.clone())
            .expect_err("paired future clients must not bypass high-risk write policy");

        assert_eq!(err.error.code, codes::APPROVAL_REQUIRED);
        assert_eq!(err.error.kind, "policy");
        assert_eq!(err.error.request_id, request_id);
        assert!(!err.error.retryable);
    }

    #[test]
    fn full_autonomy_is_represented_but_high_risk_writes_remain_disabled() {
        let _lock = env_lock().lock().expect("env lock poisoned");
        let _autonomy = EnvVarGuard::set("CAPYINN_MCP_AUTONOMY", "full");
        let request_id = Some("req-full".to_string());

        assert_eq!(configured_mcp_autonomy_level(), McpAutonomyLevel::Full);

        let err = guard_write_tool(&CREATE_RESERVATION_META, request_id.clone())
            .expect_err("full mode is represented but not launched for high-risk writes");

        assert_eq!(err.error.code, codes::WRITE_TOOL_DISABLED);
        assert_eq!(err.error.request_id, request_id);
    }

    #[test]
    fn legacy_high_risk_env_does_not_enable_high_risk_writes() {
        let _lock = env_lock().lock().expect("env lock poisoned");
        let _autonomy = EnvVarGuard::unset("CAPYINN_MCP_AUTONOMY");
        let _legacy = EnvVarGuard::set("CAPYINN_ENABLE_HIGH_RISK_MCP_WRITES", "1");
        let request_id = Some("req-legacy".to_string());

        assert_eq!(
            configured_mcp_autonomy_level(),
            McpAutonomyLevel::Supervised
        );

        let err = guard_write_tool(&CREATE_RESERVATION_META, request_id.clone())
            .expect_err("legacy env must not enable high-risk writes");

        assert_eq!(err.error.code, codes::APPROVAL_REQUIRED);
        assert_eq!(err.error.request_id, request_id);
    }

    #[test]
    fn invalid_autonomy_config_falls_back_to_supervised() {
        let _lock = env_lock().lock().expect("env lock poisoned");
        let _autonomy = EnvVarGuard::set("CAPYINN_MCP_AUTONOMY", "invalid");

        assert_eq!(
            configured_mcp_autonomy_level(),
            McpAutonomyLevel::Supervised
        );
    }
}
