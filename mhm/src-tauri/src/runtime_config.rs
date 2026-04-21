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
    fn test_now_parses_rfc3339_timestamp() {
        let _guard = env_lock().lock().unwrap();

        std::env::set_var("CAPYINN_TEST_NOW", "2026-04-21T09:15:00+07:00");
        let parsed = test_now().expect("test timestamp should parse");
        std::env::remove_var("CAPYINN_TEST_NOW");

        assert_eq!(parsed.to_rfc3339(), "2026-04-21T09:15:00+07:00");
    }
}
