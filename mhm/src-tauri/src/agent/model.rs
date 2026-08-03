use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    CeoSecretary,
    GuestReceptionist,
    FrontDeskAssistant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentChannel {
    Desktop,
    Telegram,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentProvider {
    None,
    OpenAi,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MutationRisk {
    ReadOnly,
    LowWrite,
    HighWrite,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataSensitivity {
    PublicHotelInfo,
    GuestScoped,
    StaffOperational,
    CeoSensitive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentToolCapability {
    PmsRead,
    PmsWrite,
    Shell,
    File,
    Browser,
    GenericHttp,
    DynamicMcpDiscovery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct AgentToolMeta {
    pub name: &'static str,
    pub description: &'static str,
    pub mutation_risk: MutationRisk,
    pub data_sensitivity: DataSensitivity,
    pub allowed_roles: &'static [AgentRole],
    pub capability: AgentToolCapability,
}

impl AgentToolMeta {
    pub fn allowed_for(&self, role: AgentRole) -> bool {
        self.allowed_roles.contains(&role)
    }

    pub fn is_safe_for_phase_one_ceo_registry(&self) -> bool {
        self.allowed_for(AgentRole::CeoSecretary)
            && self.mutation_risk == MutationRisk::ReadOnly
            && matches!(self.capability, AgentToolCapability::PmsRead)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelActor {
    pub channel: AgentChannel,
    pub stable_actor_id: Option<String>,
    pub display_name: Option<String>,
    pub username: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRuntimeRequest {
    pub role: AgentRole,
    pub channel_actor: ChannelActor,
    pub contains_ceo_sensitive_data: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_is_not_public_by_default() {
        let meta = AgentToolMeta {
            name: "list_unpaid_balances",
            description: "Unpaid balances",
            mutation_risk: MutationRisk::ReadOnly,
            data_sensitivity: DataSensitivity::CeoSensitive,
            allowed_roles: &[AgentRole::CeoSecretary],
            capability: AgentToolCapability::PmsRead,
        };

        assert_eq!(meta.mutation_risk, MutationRisk::ReadOnly);
        assert_eq!(meta.data_sensitivity, DataSensitivity::CeoSensitive);
        assert!(meta.allowed_for(AgentRole::CeoSecretary));
        assert!(!meta.allowed_for(AgentRole::GuestReceptionist));
    }

    #[test]
    fn ceo_tools_are_not_visible_to_the_front_desk_assistant() {
        let meta = AgentToolMeta {
            name: "get_revenue_snapshot",
            description: "Revenue snapshot",
            mutation_risk: MutationRisk::ReadOnly,
            data_sensitivity: DataSensitivity::CeoSensitive,
            allowed_roles: &[AgentRole::CeoSecretary],
            capability: AgentToolCapability::PmsRead,
        };

        assert!(!meta.allowed_for(AgentRole::FrontDeskAssistant));
    }
}
