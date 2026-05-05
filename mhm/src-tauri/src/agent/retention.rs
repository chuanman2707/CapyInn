use crate::app_error::{codes, CommandError, CommandResult};

pub const RETENTION_NOT_STORED: &str = "not_stored";
pub const RAW_PROMPT_RETENTION: &str = RETENTION_NOT_STORED;
pub const RAW_RESPONSE_RETENTION: &str = RETENTION_NOT_STORED;
pub const RAW_TOOL_OUTPUT_RETENTION: &str = RETENTION_NOT_STORED;
pub const RAW_PROVIDER_ERROR_RETENTION: &str = RETENTION_NOT_STORED;

pub const SESSION_RETENTION_METADATA_ONLY: &str = "metadata_only_v1";
pub const SESSION_METADATA_RETENTION: &str = "local_metadata_until_operator_cleanup_v1";
pub const AUDIT_METADATA_RETENTION: &str = "local_metadata_until_operator_cleanup_v1";
pub const MEMORY_RETENTION: &str = "local_non_authoritative_until_operator_cleanup_v1";

pub fn validate_session_retention_policy(value: &str) -> CommandResult<()> {
    if value == SESSION_RETENTION_METADATA_ONLY {
        Ok(())
    } else {
        Err(CommandError::user(
            codes::VALIDATION_INVALID_INPUT,
            "Unknown agent session retention policy",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_agent_data_retention_is_not_stored() {
        assert_eq!(RAW_PROMPT_RETENTION, RETENTION_NOT_STORED);
        assert_eq!(RAW_RESPONSE_RETENTION, RETENTION_NOT_STORED);
        assert_eq!(RAW_TOOL_OUTPUT_RETENTION, RETENTION_NOT_STORED);
        assert_eq!(RAW_PROVIDER_ERROR_RETENTION, RETENTION_NOT_STORED);
    }

    #[test]
    fn only_metadata_policy_is_accepted_for_sessions() {
        assert!(validate_session_retention_policy(SESSION_RETENTION_METADATA_ONLY).is_ok());
        assert!(validate_session_retention_policy("raw_prompt_30_days").is_err());
    }
}
