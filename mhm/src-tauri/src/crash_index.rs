use crate::diagnostics::{handled_dir_for, pending_dir_for, CrashBundle};
use chrono::{DateTime, FixedOffset};
use log::warn;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static REBUILD_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CrashIndexState {
    Pending,
    Submitted,
    Dismissed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashIndexRow {
    pub bundle_id: String,
    pub occurred_at: String,
    pub app_version: String,
    pub crash_type: String,
    pub message: String,
    pub module_hint: Option<String>,
    pub state: CrashIndexState,
}

pub fn crash_index_path(runtime_root: &Path) -> PathBuf {
    runtime_root.join("diagnostics").join("crashes.jsonl")
}

pub fn rebuild_crash_index_for(runtime_root: &Path) -> Result<PathBuf, String> {
    let _guard = REBUILD_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let path = crash_index_path(runtime_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let mut rows = Vec::new();
    rows.extend(scan_bundle_dir(
        &pending_dir_for(runtime_root),
        CrashIndexState::Pending,
    )?);
    rows.extend(scan_handled_dir(&handled_dir_for(runtime_root))?);
    rows.sort_by(compare_rows_newest_first);

    let mut payload = String::new();
    for row in rows {
        let line = serde_json::to_string(&row).map_err(|error| error.to_string())?;
        payload.push_str(&line);
        payload.push('\n');
    }

    std::fs::write(&path, payload).map_err(|error| error.to_string())?;
    Ok(path)
}

pub fn rebuild_current_runtime_root() -> Result<PathBuf, String> {
    rebuild_crash_index_for(&crate::app_identity::runtime_root())
}

fn scan_bundle_dir(dir: &Path, state: CrashIndexState) -> Result<Vec<CrashIndexRow>, String> {
    let mut rows = Vec::new();
    for path in read_json_entries(dir)? {
        match read_bundle_row(&path, state) {
            Ok(Some(row)) => rows.push(row),
            Ok(None) => {}
            Err(error) => warn!("{error}"),
        }
    }
    Ok(rows)
}

fn scan_handled_dir(dir: &Path) -> Result<Vec<CrashIndexRow>, String> {
    let mut rows = Vec::new();
    for path in read_json_entries(dir)? {
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        let state = if file_name.ends_with(".submitted.json") {
            CrashIndexState::Submitted
        } else if file_name.ends_with(".dismissed.json") {
            CrashIndexState::Dismissed
        } else {
            continue;
        };

        match read_bundle_row(&path, state) {
            Ok(Some(row)) => rows.push(row),
            Ok(None) => {}
            Err(error) => warn!("{error}"),
        }
    }
    Ok(rows)
}

fn read_json_entries(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(format!(
                "failed to read crash index directory {}: {}",
                dir.display(),
                error
            ));
        }
    };

    collect_json_entries(entries, dir)
}

fn collect_json_entries<I>(entries: I, dir: &Path) -> Result<Vec<PathBuf>, String>
where
    I: IntoIterator<Item = Result<std::fs::DirEntry, std::io::Error>>,
{
    let mut paths = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|error| {
            format!(
                "failed to iterate crash index directory {}: {}",
                dir.display(),
                error
            )
        })?;

        let path = entry.path();
        if path.extension() == Some(std::ffi::OsStr::new("json")) {
            paths.push(path);
        }
    }

    Ok(paths)
}

fn read_bundle_row(path: &Path, state: CrashIndexState) -> Result<Option<CrashIndexRow>, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return Err(format!(
                "skipping unreadable crash bundle {}: {}",
                path.display(),
                error
            ));
        }
    };

    let bundle = match serde_json::from_slice::<CrashBundle>(&bytes) {
        Ok(bundle) => bundle,
        Err(error) => {
            return Err(format!(
                "skipping corrupt crash bundle {}: {}",
                path.display(),
                error
            ));
        }
    };

    if parse_occurred_at(&bundle.occurred_at).is_none() {
        warn!(
            "skipping crash bundle {} with malformed occurred_at {}",
            path.display(),
            bundle.occurred_at
        );
        return Ok(None);
    }

    Ok(Some(bundle_to_row(bundle, state)))
}

fn parse_occurred_at(value: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(value).ok()
}

fn bundle_to_row(bundle: CrashBundle, state: CrashIndexState) -> CrashIndexRow {
    CrashIndexRow {
        bundle_id: bundle.bundle_id,
        occurred_at: bundle.occurred_at,
        app_version: bundle.app_version,
        crash_type: bundle.crash_type,
        message: bundle.message,
        module_hint: bundle.module_hint,
        state,
    }
}

fn compare_rows_newest_first(left: &CrashIndexRow, right: &CrashIndexRow) -> std::cmp::Ordering {
    match (
        parse_occurred_at(&left.occurred_at),
        parse_occurred_at(&right.occurred_at),
    ) {
        (Some(left_time), Some(right_time)) => right_time
            .cmp(&left_time)
            .then_with(|| right.bundle_id.cmp(&left.bundle_id)),
        _ => right
            .occurred_at
            .cmp(&left.occurred_at)
            .then_with(|| right.bundle_id.cmp(&left.bundle_id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::{
        handled_dir_for, mark_bundle_handled_for, pending_dir_for, write_pending_bundle_for,
        CrashBundle,
    };
    use std::ffi::OsString;

    struct RuntimeRootOverrideGuard {
        previous_runtime_root: Option<OsString>,
    }

    impl RuntimeRootOverrideGuard {
        fn set(root: &Path) -> Self {
            let previous_runtime_root = std::env::var_os("CAPYINN_RUNTIME_ROOT");
            std::env::set_var("CAPYINN_RUNTIME_ROOT", root);
            Self {
                previous_runtime_root,
            }
        }
    }

    impl Drop for RuntimeRootOverrideGuard {
        fn drop(&mut self) {
            match self.previous_runtime_root.take() {
                Some(value) => std::env::set_var("CAPYINN_RUNTIME_ROOT", value),
                None => std::env::remove_var("CAPYINN_RUNTIME_ROOT"),
            }
        }
    }

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "capyinn-crash-index-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).expect("create temp root");
        root
    }

    fn bundle(bundle_id: &str, occurred_at: &str) -> CrashBundle {
        CrashBundle {
            bundle_id: bundle_id.to_string(),
            crash_type: "js_unhandled_error".to_string(),
            occurred_at: occurred_at.to_string(),
            app_version: "0.1.1".to_string(),
            environment: "development".to_string(),
            platform: "macos".to_string(),
            arch: "aarch64".to_string(),
            installation_id: "install-1".to_string(),
            message: format!("message-{bundle_id}"),
            stacktrace: vec!["frame-a".to_string()],
            module_hint: Some("Dashboard".to_string()),
            attempt_count: 0,
        }
    }

    #[test]
    fn crash_index_path_stays_under_diagnostics() {
        let root = temp_root("path");

        assert_eq!(
            crash_index_path(&root),
            root.join("diagnostics").join("crashes.jsonl")
        );
    }

    #[test]
    fn rebuild_crash_index_includes_pending_and_handled_bundles() {
        let root = temp_root("rebuild");

        let pending = bundle("pending-1", "2026-04-22T10:00:00+07:00");
        let submitted = bundle("submitted-1", "2026-04-23T10:00:00+07:00");
        let dismissed = bundle("dismissed-1", "2026-04-21T10:00:00+07:00");

        write_pending_bundle_for(&root, &pending).expect("write pending bundle");
        write_pending_bundle_for(&root, &submitted).expect("write submitted bundle");
        mark_bundle_handled_for(&root, &submitted.bundle_id, "submitted").expect("mark submitted");
        write_pending_bundle_for(&root, &dismissed).expect("write dismissed bundle");
        mark_bundle_handled_for(&root, &dismissed.bundle_id, "dismissed").expect("mark dismissed");

        let index_path = rebuild_crash_index_for(&root).expect("rebuild crash index");
        let index = std::fs::read_to_string(index_path).expect("read index");

        let rows = index.lines().collect::<Vec<_>>();
        assert_eq!(rows.len(), 3);

        let parsed = rows
            .into_iter()
            .map(|line| serde_json::from_str::<CrashIndexRow>(line).expect("parse row"))
            .collect::<Vec<_>>();

        assert_eq!(
            parsed
                .iter()
                .map(|row| row.bundle_id.as_str())
                .collect::<Vec<_>>(),
            vec!["submitted-1", "pending-1", "dismissed-1"]
        );
        assert_eq!(
            parsed.iter().map(|row| row.state).collect::<Vec<_>>(),
            vec![
                CrashIndexState::Submitted,
                CrashIndexState::Pending,
                CrashIndexState::Dismissed,
            ]
        );
    }

    #[test]
    fn rebuild_crash_index_skips_corrupt_handled_artifacts() {
        let root = temp_root("corrupt");
        let handled_dir = handled_dir_for(&root);
        std::fs::create_dir_all(&handled_dir).expect("create handled dir");
        std::fs::write(handled_dir.join("bad.corrupt.json"), b"{not-json").expect("write corrupt");

        let bundle = bundle("pending-2", "2026-04-20T10:00:00+07:00");
        write_pending_bundle_for(&root, &bundle).expect("write pending bundle");

        let index_path = rebuild_crash_index_for(&root).expect("rebuild crash index");
        let index = std::fs::read_to_string(index_path).expect("read index");

        assert!(!index.contains("bad.corrupt.json"));
        assert!(index.contains("pending-2"));
        assert!(pending_dir_for(&root).exists());
    }

    #[test]
    fn rebuild_current_runtime_root_uses_runtime_root_override() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();
        let root = temp_root("runtime-root-override");
        let _runtime_root = RuntimeRootOverrideGuard::set(&root);

        let pending = bundle("pending-override", "2026-04-23T11:00:00+07:00");
        write_pending_bundle_for(&root, &pending).expect("write pending bundle");

        let index_path =
            rebuild_current_runtime_root().expect("rebuild crash index for runtime root");
        let index = std::fs::read_to_string(&index_path).expect("read index");

        assert_eq!(index_path, crash_index_path(&root));
        assert!(index.contains("pending-override"));
    }

    #[test]
    fn runtime_root_override_guard_restores_after_panic() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();
        let previous_runtime_root = std::env::var_os("CAPYINN_RUNTIME_ROOT");
        let root = temp_root("runtime-root-guard-panic");

        let result = std::panic::catch_unwind(|| {
            let _runtime_root = RuntimeRootOverrideGuard::set(&root);
            panic!("forced panic");
        });

        assert!(result.is_err());
        assert_eq!(
            std::env::var_os("CAPYINN_RUNTIME_ROOT"),
            previous_runtime_root
        );
    }

    #[test]
    fn rebuild_crash_index_skips_bundles_with_malformed_timestamps() {
        let root = temp_root("malformed-timestamp");

        let malformed = bundle("bad-time", "not-rfc3339");
        let valid = bundle("good-time", "2026-04-23T10:00:00+07:00");

        write_pending_bundle_for(&root, &malformed).expect("write malformed pending bundle");
        write_pending_bundle_for(&root, &valid).expect("write valid pending bundle");

        let index_path = rebuild_crash_index_for(&root).expect("rebuild crash index");
        let index = std::fs::read_to_string(index_path).expect("read index");

        assert!(!index.contains("bad-time"));
        assert!(index.contains("good-time"));
        assert_eq!(index.lines().count(), 1);
    }

    #[test]
    fn rebuild_crash_index_returns_error_for_real_directory_read_failure() {
        let root = temp_root("read-dir-failure");
        let diagnostics_dir = root.join("diagnostics");
        std::fs::create_dir_all(&diagnostics_dir).expect("create diagnostics dir");
        std::fs::write(crash_index_path(&root), "{\"stale\":true}\n").expect("seed existing index");
        std::fs::write(diagnostics_dir.join("pending"), b"blocking-file")
            .expect("block pending dir");

        let error = rebuild_crash_index_for(&root).expect_err("read_dir failure must surface");
        assert!(!error.is_empty());

        let index = std::fs::read_to_string(crash_index_path(&root)).expect("read existing index");
        assert_eq!(index, "{\"stale\":true}\n");
    }

    #[test]
    fn read_json_entries_surfaces_iterator_errors() {
        let root = temp_root("iterator-error");
        let dir = root.join("diagnostics").join("pending");
        std::fs::create_dir_all(&dir).expect("create pending dir");

        let entries = vec![Err(std::io::Error::other("boom"))];
        let error = collect_json_entries(entries, &dir).expect_err("iterator error must surface");

        assert!(error.contains("failed to iterate crash index directory"));
        assert!(error.contains("boom"));
    }
}
