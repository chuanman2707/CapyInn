use crate::agent::model::{
    AgentRole, AgentToolCapability, AgentToolMeta, DataSensitivity, MutationRisk,
};

const CEO_ONLY: &[AgentRole] = &[AgentRole::CeoSecretary];

pub const CEO_PHASE_A_TOOLS: &[AgentToolMeta] = &[
    AgentToolMeta {
        name: "get_hotel_status",
        description: "Read current CEO hotel operations status summary from PMS reporting data.",
        mutation_risk: MutationRisk::ReadOnly,
        data_sensitivity: DataSensitivity::CeoSensitive,
        allowed_roles: CEO_ONLY,
        capability: AgentToolCapability::PmsRead,
    },
    AgentToolMeta {
        name: "list_room_status",
        description: "Read room occupancy and availability for CEO review.",
        mutation_risk: MutationRisk::ReadOnly,
        data_sensitivity: DataSensitivity::CeoSensitive,
        allowed_roles: CEO_ONLY,
        capability: AgentToolCapability::PmsRead,
    },
    AgentToolMeta {
        name: "list_today_arrivals",
        description: "Read today's expected arrivals and related operational notes for CEO review.",
        mutation_risk: MutationRisk::ReadOnly,
        data_sensitivity: DataSensitivity::CeoSensitive,
        allowed_roles: CEO_ONLY,
        capability: AgentToolCapability::PmsRead,
    },
    AgentToolMeta {
        name: "list_today_checkouts",
        description: "Read today's expected checkouts and departure workload for CEO review.",
        mutation_risk: MutationRisk::ReadOnly,
        data_sensitivity: DataSensitivity::CeoSensitive,
        allowed_roles: CEO_ONLY,
        capability: AgentToolCapability::PmsRead,
    },
    AgentToolMeta {
        name: "list_unpaid_balances",
        description: "Read outstanding guest and folio balances for CEO financial oversight.",
        mutation_risk: MutationRisk::ReadOnly,
        data_sensitivity: DataSensitivity::CeoSensitive,
        allowed_roles: CEO_ONLY,
        capability: AgentToolCapability::PmsRead,
    },
    AgentToolMeta {
        name: "get_revenue_snapshot",
        description:
            "Read revenue totals, booking value, and payment collection snapshot for CEO review.",
        mutation_risk: MutationRisk::ReadOnly,
        data_sensitivity: DataSensitivity::CeoSensitive,
        allowed_roles: CEO_ONLY,
        capability: AgentToolCapability::PmsRead,
    },
    AgentToolMeta {
        name: "get_audit_readiness",
        description: "Read night-audit readiness indicators and unresolved operational blockers.",
        mutation_risk: MutationRisk::ReadOnly,
        data_sensitivity: DataSensitivity::CeoSensitive,
        allowed_roles: CEO_ONLY,
        capability: AgentToolCapability::PmsRead,
    },
    AgentToolMeta {
        name: "summarize_operational_risks",
        description: "Read PMS-derived operational risk signals for CEO summary generation.",
        mutation_risk: MutationRisk::ReadOnly,
        data_sensitivity: DataSensitivity::CeoSensitive,
        allowed_roles: CEO_ONLY,
        capability: AgentToolCapability::PmsRead,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::model::{AgentRole, AgentToolCapability, DataSensitivity, MutationRisk};

    #[test]
    fn ceo_registry_contains_only_read_only_pms_read_tools() {
        let expected_names = [
            "get_hotel_status",
            "list_room_status",
            "list_today_arrivals",
            "list_today_checkouts",
            "list_unpaid_balances",
            "get_revenue_snapshot",
            "get_audit_readiness",
            "summarize_operational_risks",
        ];

        assert_eq!(CEO_PHASE_A_TOOLS.len(), expected_names.len());
        for expected_name in expected_names {
            assert!(
                CEO_PHASE_A_TOOLS
                    .iter()
                    .any(|tool| tool.name == expected_name),
                "missing {expected_name}"
            );
        }

        for tool in CEO_PHASE_A_TOOLS {
            assert_eq!(tool.mutation_risk, MutationRisk::ReadOnly, "{}", tool.name);
            assert_eq!(
                tool.capability,
                AgentToolCapability::PmsRead,
                "{}",
                tool.name
            );
            assert!(tool.allowed_for(AgentRole::CeoSecretary), "{}", tool.name);
            assert!(
                !tool.allowed_for(AgentRole::GuestReceptionist),
                "{}",
                tool.name
            );
            assert_eq!(
                tool.data_sensitivity,
                DataSensitivity::CeoSensitive,
                "{}",
                tool.name
            );
            assert!(tool.is_safe_for_phase_one_ceo_registry(), "{}", tool.name);
        }
    }

    #[test]
    fn ceo_registry_contains_no_guest_public_sensitive_mismatch() {
        for tool in CEO_PHASE_A_TOOLS {
            if matches!(tool.data_sensitivity, DataSensitivity::PublicHotelInfo) {
                assert!(
                    !tool.name.contains("booking")
                        && !tool.name.contains("invoice")
                        && !tool.name.contains("revenue")
                        && !tool.name.contains("balance")
                        && !tool.name.contains("audit"),
                    "{} must not be public hotel info",
                    tool.name
                );
            }
        }
    }
}
