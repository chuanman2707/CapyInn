use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::ffi::OsStr;
use std::fs::DirEntry;
use std::path::{Path, PathBuf};
use std::sync::Once;
use std::time::SystemTime;

use chrono::Utc;
use regex::Regex;

use crate::app_identity;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CrashBundle {
    pub bundle_id: String,
    pub crash_type: String,
    pub occurred_at: String,
    pub app_version: String,
    pub environment: String,
    pub platform: String,
    pub arch: String,
    pub installation_id: String,
    pub message: String,
    pub stacktrace: Vec<String>,
    pub module_hint: Option<String>,
    pub attempt_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JsCrashReportInput {
    pub crash_type: String,
    pub message: String,
    pub stacktrace: Vec<String>,
    pub module_hint: Option<String>,
}

static PANIC_HOOK_INIT: Once = Once::new();

fn ensure_dir(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|error| error.to_string())
}

fn validate_bundle_id(bundle_id: &str) -> Result<&str, String> {
    if !bundle_id.is_empty()
        && bundle_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Ok(bundle_id);
    }

    Err("invalid crash bundle id".to_string())
}

fn bundle_path(root: &Path, bundle_id: &str) -> Result<PathBuf, String> {
    let bundle_id = validate_bundle_id(bundle_id)?;
    Ok(pending_dir_for(root).join(format!("{bundle_id}.json")))
}

fn handled_bundle_path(root: &Path, bundle_id: &str, disposition: &str) -> Result<PathBuf, String> {
    let bundle_id = validate_bundle_id(bundle_id)?;
    Ok(handled_dir_for(root).join(format!("{bundle_id}.{disposition}.json")))
}

fn export_path_for(root: &Path, bundle_id: &str) -> Result<PathBuf, String> {
    let bundle_id = validate_bundle_id(bundle_id)?;
    Ok(root
        .join("exports")
        .join("crash-reports")
        .join(format!("{bundle_id}.json")))
}

fn json_file_entries(dir: &Path) -> Result<Vec<DirEntry>, String> {
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = std::fs::read_dir(dir)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension() == Some(OsStr::new("json")))
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| {
        entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH)
    });
    Ok(entries)
}

fn read_bundle_from_path(path: &Path) -> Result<CrashBundle, String> {
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    serde_json::from_slice::<CrashBundle>(&bytes).map_err(|error| error.to_string())
}

fn write_bundle_to_path(path: &Path, bundle: &CrashBundle) -> Result<(), String> {
    let json = serde_json::to_vec_pretty(bundle).map_err(|error| error.to_string())?;
    std::fs::write(path, json).map_err(|error| error.to_string())
}

fn quarantine_corrupt_bundle(root: &Path, entry: &DirEntry) -> Result<(), String> {
    ensure_dir(&handled_dir_for(root))?;
    let entry_path = entry.path();
    let file_stem = entry_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("corrupt-bundle");
    let target_path = handled_dir_for(root).join(format!("{file_stem}.corrupt.json"));
    std::fs::rename(entry_path, target_path).map_err(|error| error.to_string())
}

fn scrub_runtime_paths(text: &str) -> String {
    let mut scrubbed = text.to_string();

    if let Some(runtime_root) = app_identity::runtime_root_opt() {
        let runtime_root = runtime_root.to_string_lossy();
        scrubbed = scrubbed.replace(runtime_root.as_ref(), "<runtime>");
    }

    let capyinn_root_pattern =
        Regex::new(r"([A-Za-z]:)?(?:[/\\][^/\s\\]+)*[/\\]CapyInn").expect("valid runtime regex");
    capyinn_root_pattern
        .replace_all(&scrubbed, "<runtime>")
        .into_owned()
}

fn os_name() -> &'static str {
    std::env::consts::OS
}

pub fn build_rust_panic_bundle(app_version: &str, environment: &str, message: &str) -> CrashBundle {
    let installation_id = app_identity::runtime_root_opt()
        .as_deref()
        .map(load_or_create_install_id)
        .transpose()
        .ok()
        .flatten()
        .unwrap_or_else(|| "unknown-installation".to_string());

    CrashBundle {
        bundle_id: uuid::Uuid::new_v4().to_string(),
        crash_type: "rust_panic".to_string(),
        occurred_at: Utc::now().to_rfc3339(),
        app_version: app_version.to_string(),
        environment: environment.to_string(),
        platform: os_name().to_string(),
        arch: std::env::consts::ARCH.to_string(),
        installation_id,
        message: scrub_runtime_paths(message),
        stacktrace: Vec::new(),
        module_hint: None,
        attempt_count: 0,
    }
}

fn build_js_crash_bundle(root: &Path, report: JsCrashReportInput) -> Result<CrashBundle, String> {
    Ok(CrashBundle {
        bundle_id: uuid::Uuid::new_v4().to_string(),
        crash_type: report.crash_type,
        occurred_at: Utc::now().to_rfc3339(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        environment: if cfg!(debug_assertions) {
            "development".to_string()
        } else {
            "production".to_string()
        },
        platform: os_name().to_string(),
        arch: std::env::consts::ARCH.to_string(),
        installation_id: load_or_create_install_id(root)?,
        message: scrub_runtime_paths(&report.message),
        stacktrace: report
            .stacktrace
            .into_iter()
            .map(|frame| scrub_runtime_paths(&frame))
            .collect(),
        module_hint: report.module_hint,
        attempt_count: 0,
    })
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> Cow<'_, str> {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        Cow::Borrowed(message)
    } else if let Some(message) = payload.downcast_ref::<String>() {
        Cow::Borrowed(message.as_str())
    } else {
        Cow::Borrowed("panic occurred")
    }
}

pub fn load_or_create_install_id(root: &Path) -> Result<String, String> {
    let diagnostics_root = root.join("diagnostics");
    ensure_dir(&diagnostics_root)?;
    let install_id_path = diagnostics_root.join("install_id");

    if install_id_path.exists() {
        let existing =
            std::fs::read_to_string(&install_id_path).map_err(|error| error.to_string())?;
        let trimmed = existing.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let install_id = uuid::Uuid::new_v4().to_string();
    std::fs::write(&install_id_path, &install_id).map_err(|error| error.to_string())?;
    Ok(install_id)
}

pub fn pending_dir_for(root: &Path) -> PathBuf {
    root.join("diagnostics").join("pending")
}

pub fn handled_dir_for(root: &Path) -> PathBuf {
    root.join("diagnostics").join("handled")
}

pub fn write_pending_bundle_for(root: &Path, bundle: &CrashBundle) -> Result<PathBuf, String> {
    ensure_dir(&pending_dir_for(root))?;
    ensure_dir(&handled_dir_for(root))?;
    let path = bundle_path(root, &bundle.bundle_id)?;
    write_bundle_to_path(&path, bundle)?;
    Ok(path)
}

pub fn read_oldest_pending_bundle_for(root: &Path) -> Result<Option<CrashBundle>, String> {
    for entry in json_file_entries(&pending_dir_for(root))? {
        match read_bundle_from_path(&entry.path()) {
            Ok(bundle) => return Ok(Some(bundle)),
            Err(_) => quarantine_corrupt_bundle(root, &entry)?,
        }
    }

    Ok(None)
}

pub fn mark_bundle_handled_for(
    root: &Path,
    bundle_id: &str,
    disposition: &str,
) -> Result<(), String> {
    ensure_dir(&handled_dir_for(root))?;
    let current_path = bundle_path(root, bundle_id)?;
    let target_path = handled_bundle_path(root, bundle_id, disposition)?;
    std::fs::rename(&current_path, &target_path).map_err(|error| error.to_string())
}

pub fn mark_bundle_send_failed_for(root: &Path, bundle_id: &str) -> Result<(), String> {
    let path = bundle_path(root, bundle_id)?;
    let mut bundle = read_bundle_from_path(&path)?;
    bundle.attempt_count += 1;
    write_bundle_to_path(&path, &bundle)
}

pub fn export_bundle_for(root: &Path, bundle_id: &str) -> Result<PathBuf, String> {
    let export_path = export_path_for(root, bundle_id)?;
    let exports_dir = export_path
        .parent()
        .ok_or_else(|| "missing export parent directory".to_string())?;
    ensure_dir(exports_dir)?;

    let pending_path = bundle_path(root, bundle_id)?;
    let source_path = if pending_path.exists() {
        pending_path
    } else {
        json_file_entries(&handled_dir_for(root))?
            .into_iter()
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(&format!("{bundle_id}.")))
            })
            .ok_or_else(|| "crash bundle not found".to_string())?
    };

    std::fs::copy(&source_path, &export_path).map_err(|error| error.to_string())?;
    Ok(export_path)
}

pub fn prune_handled_bundles_for(root: &Path, max_entries: usize) -> Result<(), String> {
    ensure_dir(&handled_dir_for(root))?;
    let entries = json_file_entries(&handled_dir_for(root))?;
    let to_remove = entries.len().saturating_sub(max_entries);

    for entry in entries.into_iter().take(to_remove) {
        std::fs::remove_file(entry.path()).map_err(|error| error.to_string())?;
    }

    Ok(())
}

pub fn install_panic_hook(app_version: &'static str, environment: &'static str) {
    PANIC_HOOK_INIT.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            let message = panic_message(panic_info.payload());
            let location = panic_info.location().map(|location| {
                format!(
                    "{}:{}:{}",
                    location.file(),
                    location.line(),
                    location.column()
                )
            });
            let full_message = location
                .map(|location| format!("{message} ({location})"))
                .unwrap_or_else(|| message.into_owned());
            let bundle = build_rust_panic_bundle(app_version, environment, &full_message);

            if let Some(root) = app_identity::runtime_root_opt() {
                let _ = write_pending_bundle_for(&root, &bundle);
            }

            previous(panic_info);
        }));
    });
}

pub fn record_js_crash(report: JsCrashReportInput) -> Result<(), String> {
    let root = app_identity::runtime_root();
    let bundle = build_js_crash_bundle(&root, report)?;
    write_pending_bundle_for(&root, &bundle)?;
    Ok(())
}

pub fn get_pending_crash_report() -> Result<Option<CrashBundle>, String> {
    read_oldest_pending_bundle_for(&app_identity::runtime_root())
}

pub fn mark_crash_report_submitted(bundle_id: &str) -> Result<(), String> {
    let root = app_identity::runtime_root();
    mark_bundle_handled_for(&root, bundle_id, "submitted")?;
    prune_handled_bundles_for(&root, 20)
}

pub fn mark_crash_report_dismissed(bundle_id: &str) -> Result<(), String> {
    let root = app_identity::runtime_root();
    mark_bundle_handled_for(&root, bundle_id, "dismissed")?;
    prune_handled_bundles_for(&root, 20)
}

pub fn mark_crash_report_send_failed(bundle_id: &str) -> Result<(), String> {
    mark_bundle_send_failed_for(&app_identity::runtime_root(), bundle_id)
}

pub fn export_crash_report(bundle_id: &str) -> Result<PathBuf, String> {
    export_bundle_for(&app_identity::runtime_root(), bundle_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "capyinn-diagnostics-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("create temp root");
        root
    }

    fn sample_bundle() -> CrashBundle {
        CrashBundle {
            bundle_id: "bundle-1".to_string(),
            crash_type: "js_unhandled_error".to_string(),
            occurred_at: "2026-04-20T10:00:00+07:00".to_string(),
            app_version: "0.1.1".to_string(),
            environment: "development".to_string(),
            platform: "macos".to_string(),
            arch: "aarch64".to_string(),
            installation_id: "install-1".to_string(),
            message: "boom".to_string(),
            stacktrace: vec!["frame-a".to_string(), "frame-b".to_string()],
            module_hint: Some("Dashboard".to_string()),
            attempt_count: 0,
        }
    }

    #[test]
    fn install_id_persists_for_same_runtime_root() {
        let root = temp_root("install-id");

        let first = load_or_create_install_id(&root).expect("first install id");
        let second = load_or_create_install_id(&root).expect("second install id");

        assert_eq!(first, second);
        assert!(root.join("diagnostics").join("install_id").exists());
    }

    #[test]
    fn diagnostics_dirs_are_nested_under_the_runtime_root() {
        let root = temp_root("paths");

        assert_eq!(
            pending_dir_for(&root),
            root.join("diagnostics").join("pending")
        );
        assert_eq!(
            handled_dir_for(&root),
            root.join("diagnostics").join("handled")
        );
    }

    #[test]
    fn pending_bundle_round_trips_and_exports() {
        let root = temp_root("roundtrip");
        let bundle = sample_bundle();

        write_pending_bundle_for(&root, &bundle).expect("write pending bundle");
        let loaded = read_oldest_pending_bundle_for(&root)
            .expect("read pending bundle")
            .expect("bundle should exist");

        assert_eq!(loaded.bundle_id, bundle.bundle_id);

        let export_path =
            export_bundle_for(&root, &bundle.bundle_id).expect("export pending bundle");
        assert!(export_path.ends_with("exports/crash-reports/bundle-1.json"));
    }

    #[test]
    fn mark_send_failed_increments_attempt_count() {
        let root = temp_root("attempts");
        let bundle = sample_bundle();

        write_pending_bundle_for(&root, &bundle).expect("write pending bundle");
        mark_bundle_send_failed_for(&root, &bundle.bundle_id).expect("mark send failed");

        let loaded = read_oldest_pending_bundle_for(&root)
            .expect("read pending bundle")
            .expect("bundle should exist");

        assert_eq!(loaded.attempt_count, 1);
    }

    #[test]
    fn handled_bundle_prune_keeps_only_recent_files() {
        let root = temp_root("prune");
        let bundle = sample_bundle();

        write_pending_bundle_for(&root, &bundle).expect("write pending bundle");
        mark_bundle_handled_for(&root, &bundle.bundle_id, "submitted").expect("mark handled");
        prune_handled_bundles_for(&root, 0).expect("prune handled bundles");

        assert!(handled_dir_for(&root).exists());
    }

    #[test]
    fn build_rust_panic_bundle_scrubs_runtime_paths() {
        let bundle = build_rust_panic_bundle(
            "0.1.1",
            "production",
            "/Users/test/CapyInn/capyinn.db not readable",
        );

        assert!(bundle.message.contains("<runtime>"));
        assert!(!bundle.message.contains("/Users/test/CapyInn"));
    }

    #[test]
    fn rejects_invalid_bundle_ids_for_filesystem_operations() {
        let root = temp_root("invalid-bundle-id");

        let error = export_bundle_for(&root, "../escape").expect_err("invalid id must be rejected");
        assert_eq!(error, "invalid crash bundle id");
    }

    #[test]
    fn read_oldest_pending_bundle_skips_corrupt_json_files() {
        let root = temp_root("corrupt-pending");
        let pending_dir = pending_dir_for(&root);
        std::fs::create_dir_all(&pending_dir).expect("create pending dir");
        std::fs::write(pending_dir.join("bad.json"), b"{not-json").expect("write corrupt bundle");

        let bundle = sample_bundle();
        write_pending_bundle_for(&root, &bundle).expect("write valid pending bundle");

        let loaded = read_oldest_pending_bundle_for(&root)
            .expect("read pending bundle")
            .expect("valid bundle should be returned");

        assert_eq!(loaded.bundle_id, bundle.bundle_id);
        assert!(handled_dir_for(&root).join("bad.corrupt.json").exists());
    }
}
