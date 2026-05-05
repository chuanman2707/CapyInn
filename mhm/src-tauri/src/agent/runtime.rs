use crate::{
    agent::model::{AgentRole, ChannelActor},
    app_error::{codes, CommandError, CommandResult},
};

#[derive(Debug, Clone)]
pub struct AgentRuntimePolicyInput {
    pub role: AgentRole,
    pub channel_actor: ChannelActor,
    pub ceo_cloud_data_opt_in: bool,
    pub contains_ceo_sensitive_data: bool,
}

fn ensure_paired_actor(input: &AgentRuntimePolicyInput) -> CommandResult<()> {
    if input
        .channel_actor
        .stable_actor_id
        .as_deref()
        .is_none_or(str::is_empty)
    {
        return Err(CommandError::user(
            codes::AGENT_CHANNEL_UNPAIRED,
            "Agent channel actor is not paired",
        ));
    }
    Ok(())
}

fn ensure_cloud_opt_in(input: &AgentRuntimePolicyInput) -> CommandResult<()> {
    if input.contains_ceo_sensitive_data && !input.ceo_cloud_data_opt_in {
        return Err(CommandError::user(
            codes::AGENT_CLOUD_DATA_OPT_IN_REQUIRED,
            "CEO cloud-data opt-in is required",
        ));
    }
    Ok(())
}

pub async fn build_provider_request_disabled(input: AgentRuntimePolicyInput) -> CommandResult<()> {
    ensure_paired_actor(&input)?;
    ensure_cloud_opt_in(&input)?;
    Err(CommandError::user(
        codes::AGENT_PROVIDER_DISABLED,
        "Agent provider execution is disabled",
    ))
}

pub async fn handle_agent_message_disabled(input: AgentRuntimePolicyInput) -> CommandResult<()> {
    ensure_paired_actor(&input)?;
    ensure_cloud_opt_in(&input)?;
    Err(CommandError::user(
        codes::AGENT_RUNTIME_DISABLED,
        "Agent runtime is disabled",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::model::{AgentChannel, AgentRole, ChannelActor};

    #[tokio::test]
    async fn unpaired_actor_is_denied_before_prompt_construction() {
        let result = handle_agent_message_disabled(AgentRuntimePolicyInput {
            role: AgentRole::CeoSecretary,
            channel_actor: ChannelActor {
                channel: AgentChannel::Telegram,
                stable_actor_id: None,
                display_name: Some("Unknown".to_string()),
                username: Some("unknown".to_string()),
            },
            ceo_cloud_data_opt_in: true,
            contains_ceo_sensitive_data: true,
        })
        .await
        .expect_err("unpaired actor must be denied");

        assert_eq!(result.code, crate::app_error::codes::AGENT_CHANNEL_UNPAIRED);
    }

    #[tokio::test]
    async fn ceo_sensitive_provider_request_requires_opt_in() {
        let result = build_provider_request_disabled(AgentRuntimePolicyInput {
            role: AgentRole::CeoSecretary,
            channel_actor: ChannelActor {
                channel: AgentChannel::Telegram,
                stable_actor_id: Some("12345".to_string()),
                display_name: None,
                username: None,
            },
            ceo_cloud_data_opt_in: false,
            contains_ceo_sensitive_data: true,
        })
        .await
        .expect_err("missing opt-in must block provider construction");

        assert_eq!(
            result.code,
            crate::app_error::codes::AGENT_CLOUD_DATA_OPT_IN_REQUIRED
        );
    }

    #[tokio::test]
    async fn ceo_sensitive_guest_request_requires_opt_in() {
        let result = build_provider_request_disabled(AgentRuntimePolicyInput {
            role: AgentRole::GuestReceptionist,
            channel_actor: ChannelActor {
                channel: AgentChannel::Telegram,
                stable_actor_id: Some("12345".to_string()),
                display_name: None,
                username: None,
            },
            ceo_cloud_data_opt_in: false,
            contains_ceo_sensitive_data: true,
        })
        .await
        .expect_err("CEO-sensitive data must require opt-in for every role");

        assert_eq!(
            result.code,
            crate::app_error::codes::AGENT_CLOUD_DATA_OPT_IN_REQUIRED
        );
    }

    #[tokio::test]
    async fn runtime_is_disabled_even_after_policy_checks() {
        let result = handle_agent_message_disabled(AgentRuntimePolicyInput {
            role: AgentRole::CeoSecretary,
            channel_actor: ChannelActor {
                channel: AgentChannel::Telegram,
                stable_actor_id: Some("12345".to_string()),
                display_name: None,
                username: None,
            },
            ceo_cloud_data_opt_in: true,
            contains_ceo_sensitive_data: true,
        })
        .await
        .expect_err("runtime remains disabled");

        assert_eq!(result.code, crate::app_error::codes::AGENT_RUNTIME_DISABLED);
    }
}
