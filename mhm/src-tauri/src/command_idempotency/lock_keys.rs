use super::{stable_json_string, system_error};
use crate::app_error::CommandResult;

pub(super) fn optional_lock_keys_json<I, S>(lock_keys: I) -> CommandResult<String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let lock_keys = canonical_lock_keys(lock_keys);
    stable_json_string(&serde_json::json!(lock_keys))
}

pub(super) fn required_lock_keys_json<I, S>(lock_keys: I) -> CommandResult<String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let lock_keys = canonical_lock_keys(lock_keys);
    if lock_keys.is_empty() {
        return Err(system_error("Resolved idempotency lock keys are required"));
    }

    stable_json_string(&serde_json::json!(lock_keys))
}

fn canonical_lock_keys<I, S>(lock_keys: I) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let mut lock_keys = lock_keys.into_iter().map(Into::into).collect::<Vec<_>>();
    lock_keys.sort();
    lock_keys.dedup();
    lock_keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_error::codes;

    #[test]
    fn optional_lock_keys_json_sorts_and_deduplicates_without_trimming() {
        let lock_keys_json = optional_lock_keys_json([
            "room:R1".to_string(),
            " booking:B1".to_string(),
            "booking:B1".to_string(),
            "room:R1".to_string(),
        ])
        .expect("lock keys serialize");

        assert_eq!(
            lock_keys_json,
            "[\" booking:B1\",\"booking:B1\",\"room:R1\"]"
        );
    }

    #[test]
    fn optional_lock_keys_json_allows_empty_initial_keys() {
        let lock_keys_json =
            optional_lock_keys_json(Vec::<String>::new()).expect("empty keys serialize");

        assert_eq!(lock_keys_json, "[]");
    }

    #[test]
    fn optional_lock_keys_json_preserves_settings_format() {
        let lock_keys_json =
            optional_lock_keys_json(["settings:ceo_cloud_data_opt_in".to_string()])
                .expect("settings lock key serializes");

        assert_eq!(lock_keys_json, "[\"settings:ceo_cloud_data_opt_in\"]");
    }

    #[test]
    fn required_lock_keys_json_rejects_empty_with_current_system_error() {
        let error = required_lock_keys_json(Vec::<String>::new())
            .expect_err("empty resolved keys should fail");

        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
        assert_eq!(error.message, "Resolved idempotency lock keys are required");
    }
}
