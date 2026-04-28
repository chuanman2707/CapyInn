use serde::Serialize;

use crate::app_error::codes;
use crate::runtime_config;
use crate::write_manifest::LockDeriverId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
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
    fn write_tool_disabled(meta: &WriteToolMeta) -> Self {
        Self {
            ok: false,
            error: McpToolError {
                code: codes::WRITE_TOOL_DISABLED,
                kind: "policy",
                message: format!(
                    "MCP write tool '{}' is disabled. Set CAPYINN_ENABLE_HIGH_RISK_MCP_WRITES=1 to enable high-risk MCP writes.",
                    meta.command_name
                ),
                tool: meta.command_name,
                risk_level: meta.risk_level,
                retryable: false,
                request_id: None,
            },
        }
    }

    pub fn to_json_string(&self) -> String {
        serde_json::to_string_pretty(self).expect("MCP error envelope should serialize")
    }
}

pub fn high_risk_mcp_writes_enabled() -> bool {
    runtime_config::env_flag("CAPYINN_ENABLE_HIGH_RISK_MCP_WRITES")
}

pub fn guard_write_tool(meta: &WriteToolMeta) -> Result<(), McpToolErrorEnvelope> {
    if meta.risk_level == RiskLevel::High && !high_risk_mcp_writes_enabled() {
        return Err(McpToolErrorEnvelope::write_tool_disabled(meta));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
