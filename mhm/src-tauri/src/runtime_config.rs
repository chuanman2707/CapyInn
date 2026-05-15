use chrono::{DateTime, FixedOffset};
use std::path::PathBuf;

pub fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name)
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

pub fn experimental_runtime_enabled() -> bool {
    env_flag("CAPYINN_EXPERIMENTAL_RUNTIME")
}

pub fn experimental_gateway_runtime_enabled() -> bool {
    experimental_runtime_enabled() || env_flag("CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME")
}

pub fn experimental_agent_runtime_enabled() -> bool {
    experimental_runtime_enabled() || env_flag("CAPYINN_EXPERIMENTAL_AGENT_RUNTIME")
}

#[allow(dead_code)]
pub fn experimental_peripheral_runtime_enabled() -> bool {
    experimental_runtime_enabled() || env_flag("CAPYINN_EXPERIMENTAL_PERIPHERAL_RUNTIME")
}

pub fn gateway_runtime_disabled_by_override() -> bool {
    env_flag("CAPYINN_DISABLE_GATEWAY")
}

pub fn agent_runtime_disabled_by_override() -> bool {
    env_flag("CAPYINN_DISABLE_CEO_TELEGRAM")
}

pub fn effective_experimental_gateway_runtime_enabled() -> bool {
    experimental_gateway_runtime_enabled() && !gateway_runtime_disabled_by_override()
}

pub fn effective_experimental_agent_runtime_enabled() -> bool {
    experimental_agent_runtime_enabled() && !agent_runtime_disabled_by_override()
}

pub fn runtime_root_override() -> Option<PathBuf> {
    std::env::var_os("CAPYINN_RUNTIME_ROOT")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn test_now() -> Option<DateTime<FixedOffset>> {
    std::env::var("CAPYINN_TEST_NOW")
        .ok()
        .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
}

pub fn smoke_ready_file() -> Option<PathBuf> {
    std::env::var_os("CAPYINN_SMOKE_READY_FILE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
pub fn env_lock() -> &'static std::sync::Mutex<()> {
    use std::sync::{Mutex, OnceLock};

    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_root_override_reads_from_env() {
        let _guard = env_lock().lock().unwrap();

        std::env::set_var("CAPYINN_RUNTIME_ROOT", "/tmp/capyinn-test-suite");
        assert_eq!(
            runtime_root_override().as_deref(),
            Some(std::path::Path::new("/tmp/capyinn-test-suite"))
        );
        std::env::remove_var("CAPYINN_RUNTIME_ROOT");
    }

    #[test]
    fn truthy_flags_enable_runtime_toggles() {
        let _guard = env_lock().lock().unwrap();

        std::env::set_var("CAPYINN_DISABLE_WATCHER", "true");
        std::env::set_var("CAPYINN_DISABLE_GATEWAY", "1");
        assert!(env_flag("CAPYINN_DISABLE_WATCHER"));
        assert!(env_flag("CAPYINN_DISABLE_GATEWAY"));
        std::env::remove_var("CAPYINN_DISABLE_WATCHER");
        std::env::remove_var("CAPYINN_DISABLE_GATEWAY");
    }

    #[test]
    fn experimental_runtime_flags_are_disabled_by_default() {
        let _guard = env_lock().lock().unwrap();

        for name in [
            "CAPYINN_EXPERIMENTAL_RUNTIME",
            "CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME",
            "CAPYINN_EXPERIMENTAL_AGENT_RUNTIME",
            "CAPYINN_EXPERIMENTAL_PERIPHERAL_RUNTIME",
        ] {
            std::env::remove_var(name);
        }

        assert!(!experimental_runtime_enabled());
        assert!(!experimental_gateway_runtime_enabled());
        assert!(!experimental_agent_runtime_enabled());
        assert!(!experimental_peripheral_runtime_enabled());
    }

    #[test]
    fn master_experimental_runtime_flag_enables_all_experimental_surfaces() {
        let _guard = env_lock().lock().unwrap();

        std::env::set_var("CAPYINN_EXPERIMENTAL_RUNTIME", "true");
        std::env::remove_var("CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME");
        std::env::remove_var("CAPYINN_EXPERIMENTAL_AGENT_RUNTIME");
        std::env::remove_var("CAPYINN_EXPERIMENTAL_PERIPHERAL_RUNTIME");

        assert!(experimental_gateway_runtime_enabled());
        assert!(experimental_agent_runtime_enabled());
        assert!(experimental_peripheral_runtime_enabled());

        std::env::remove_var("CAPYINN_EXPERIMENTAL_RUNTIME");
    }

    #[test]
    fn individual_experimental_runtime_flags_enable_only_matching_surfaces() {
        let _guard = env_lock().lock().unwrap();

        for name in [
            "CAPYINN_EXPERIMENTAL_RUNTIME",
            "CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME",
            "CAPYINN_EXPERIMENTAL_AGENT_RUNTIME",
            "CAPYINN_EXPERIMENTAL_PERIPHERAL_RUNTIME",
            "CAPYINN_DISABLE_GATEWAY",
            "CAPYINN_DISABLE_CEO_TELEGRAM",
        ] {
            std::env::remove_var(name);
        }

        std::env::set_var("CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME", "true");
        assert!(experimental_gateway_runtime_enabled());
        assert!(!experimental_agent_runtime_enabled());
        assert!(!experimental_peripheral_runtime_enabled());
        assert!(effective_experimental_gateway_runtime_enabled());
        assert!(!effective_experimental_agent_runtime_enabled());
        std::env::remove_var("CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME");

        std::env::set_var("CAPYINN_EXPERIMENTAL_AGENT_RUNTIME", "true");
        assert!(!experimental_gateway_runtime_enabled());
        assert!(experimental_agent_runtime_enabled());
        assert!(!experimental_peripheral_runtime_enabled());
        assert!(!effective_experimental_gateway_runtime_enabled());
        assert!(effective_experimental_agent_runtime_enabled());
        std::env::remove_var("CAPYINN_EXPERIMENTAL_AGENT_RUNTIME");
    }

    #[test]
    fn disable_flags_override_effective_experimental_runtime_flags() {
        let _guard = env_lock().lock().unwrap();

        std::env::set_var("CAPYINN_EXPERIMENTAL_RUNTIME", "true");
        std::env::set_var("CAPYINN_DISABLE_GATEWAY", "true");
        std::env::set_var("CAPYINN_DISABLE_CEO_TELEGRAM", "true");

        assert!(experimental_gateway_runtime_enabled());
        assert!(experimental_agent_runtime_enabled());
        assert!(gateway_runtime_disabled_by_override());
        assert!(agent_runtime_disabled_by_override());
        assert!(!effective_experimental_gateway_runtime_enabled());
        assert!(!effective_experimental_agent_runtime_enabled());

        std::env::remove_var("CAPYINN_EXPERIMENTAL_RUNTIME");
        std::env::remove_var("CAPYINN_DISABLE_GATEWAY");
        std::env::remove_var("CAPYINN_DISABLE_CEO_TELEGRAM");
    }

    #[test]
    fn test_now_parses_rfc3339_timestamp() {
        let _guard = env_lock().lock().unwrap();

        std::env::set_var("CAPYINN_TEST_NOW", "2026-04-21T09:15:00+07:00");
        let parsed = test_now().expect("test timestamp should parse");
        std::env::remove_var("CAPYINN_TEST_NOW");

        assert_eq!(parsed.to_rfc3339(), "2026-04-21T09:15:00+07:00");
    }
}
