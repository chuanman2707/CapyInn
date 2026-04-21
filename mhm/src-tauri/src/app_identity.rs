use std::path::PathBuf;

pub const APP_NAME: &str = "CapyInn";
pub const APP_RUNTIME_DIR: &str = "CapyInn";
pub const APP_DATABASE_FILENAME: &str = "capyinn.db";
pub const APP_API_KEY_PREFIX: &str = "capyinn_sk_";
pub const APP_GATEWAY_LOCKFILE: &str = ".gateway-port";
pub const APP_BUNDLE_IDENTIFIER: &str = "io.capyinn.app";

pub fn runtime_root() -> PathBuf {
    runtime_root_opt().expect("Cannot find home directory")
}

pub fn runtime_root_opt() -> Option<PathBuf> {
    crate::runtime_config::runtime_root_override()
        .or_else(|| dirs::home_dir().map(|home| home.join(APP_RUNTIME_DIR)))
}

pub fn database_path() -> PathBuf {
    runtime_root().join(APP_DATABASE_FILENAME)
}

pub fn database_path_opt() -> Option<PathBuf> {
    runtime_root_opt().map(|root| root.join(APP_DATABASE_FILENAME))
}

pub fn scans_dir() -> PathBuf {
    runtime_root().join("Scans")
}

pub fn scans_dir_opt() -> Option<PathBuf> {
    runtime_root_opt().map(|root| root.join("Scans"))
}

pub fn models_dir() -> PathBuf {
    runtime_root().join("models")
}

pub fn models_dir_opt() -> Option<PathBuf> {
    runtime_root_opt().map(|root| root.join("models"))
}

pub fn exports_dir() -> PathBuf {
    runtime_root().join("exports")
}

pub fn exports_dir_opt() -> Option<PathBuf> {
    runtime_root_opt().map(|root| root.join("exports"))
}

pub fn gateway_lockfile() -> PathBuf {
    runtime_root().join(APP_GATEWAY_LOCKFILE)
}

pub fn diagnostics_dir() -> PathBuf {
    runtime_root().join("diagnostics")
}

pub fn diagnostics_pending_dir() -> PathBuf {
    diagnostics_dir().join("pending")
}

pub fn diagnostics_handled_dir() -> PathBuf {
    diagnostics_dir().join("handled")
}

pub fn diagnostics_install_id_path() -> PathBuf {
    diagnostics_dir().join("install_id")
}

pub fn crash_report_exports_dir() -> PathBuf {
    exports_dir().join("crash-reports")
}

pub fn gateway_lockfile_opt() -> Option<PathBuf> {
    runtime_root_opt().map(|root| root.join(APP_GATEWAY_LOCKFILE))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_root_uses_override_when_present() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();

        std::env::set_var("CAPYINN_RUNTIME_ROOT", "/tmp/capyinn-test-suite");
        assert_eq!(
            runtime_root_opt().as_deref(),
            Some(std::path::Path::new("/tmp/capyinn-test-suite"))
        );
        std::env::remove_var("CAPYINN_RUNTIME_ROOT");
    }

    #[test]
    fn uses_capyinn_runtime_names() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();
        std::env::remove_var("CAPYINN_RUNTIME_ROOT");

        let root = runtime_root();
        assert!(root.ends_with(APP_RUNTIME_DIR));
        assert_eq!(database_path(), root.join(APP_DATABASE_FILENAME));
        assert_eq!(scans_dir(), root.join("Scans"));
        assert_eq!(models_dir(), root.join("models"));
        assert_eq!(exports_dir(), root.join("exports"));
        assert_eq!(diagnostics_dir(), root.join("diagnostics"));
        assert_eq!(
            diagnostics_pending_dir(),
            root.join("diagnostics").join("pending")
        );
        assert_eq!(
            diagnostics_handled_dir(),
            root.join("diagnostics").join("handled")
        );
        assert_eq!(
            diagnostics_install_id_path(),
            root.join("diagnostics").join("install_id")
        );
        assert_eq!(
            crash_report_exports_dir(),
            root.join("exports").join("crash-reports")
        );
        assert_eq!(gateway_lockfile(), root.join(APP_GATEWAY_LOCKFILE));
        assert_eq!(APP_NAME, "CapyInn");
        assert_eq!(APP_API_KEY_PREFIX, "capyinn_sk_");
        assert_eq!(APP_BUNDLE_IDENTIFIER, "io.capyinn.app");
    }
}
