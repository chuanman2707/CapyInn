use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app_error::AppErrorKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupportErrorRecord {
    pub timestamp: String,
    pub support_id: String,
    pub command: String,
    pub code: String,
    pub kind: AppErrorKind,
    pub root_cause: String,
    pub context: Value,
}

impl SupportErrorRecord {
    pub fn new(
        command: impl Into<String>,
        code: impl Into<String>,
        root_cause: impl Into<String>,
        support_id: impl Into<String>,
        context: Value,
    ) -> Self {
        Self {
            timestamp: Utc::now().to_rfc3339(),
            support_id: support_id.into(),
            command: command.into(),
            code: code.into(),
            kind: AppErrorKind::System,
            root_cause: root_cause.into(),
            context,
        }
    }
}

fn append_mutex() -> &'static Mutex<()> {
    static APPEND_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
    APPEND_MUTEX.get_or_init(|| Mutex::new(()))
}

pub fn support_log_path(runtime_root: &Path) -> PathBuf {
    runtime_root
        .join("diagnostics")
        .join("support-errors.jsonl")
}

pub fn append_support_error_record(
    runtime_root: &Path,
    record: &SupportErrorRecord,
) -> Result<(), String> {
    let _guard = append_mutex().lock().map_err(|error| error.to_string())?;
    let path = support_log_path(runtime_root);

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let line = serde_json::to_string(record).map_err(|error| error.to_string())?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|error| error.to_string())?;

    file.write_all(line.as_bytes())
        .and_then(|_| file.write_all(b"\n"))
        .map_err(|error| error.to_string())?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::sync::{Arc, Barrier};
    use std::thread;

    fn test_runtime_root(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "capyinn-support-log-{}-{}",
            test_name,
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn support_log_path_stays_under_diagnostics() {
        let root = PathBuf::from("/tmp/capyinn-runtime-root");
        assert_eq!(
            support_log_path(&root),
            root.join("diagnostics").join("support-errors.jsonl")
        );
    }

    #[test]
    fn append_support_error_record_appends_json_lines_without_interleaving() {
        let runtime_root = test_runtime_root("append");
        let _ = fs::remove_dir_all(&runtime_root);

        let first = SupportErrorRecord::new(
            "login",
            crate::app_error::codes::SYSTEM_INTERNAL_ERROR,
            "database offline",
            "SUP-AAAA0001",
            json!({ "room_id": "R101" }),
        );
        let second = SupportErrorRecord::new(
            "check_out",
            crate::app_error::codes::SYSTEM_INTERNAL_ERROR,
            "lock failed",
            "SUP-BBBB0002",
            json!({ "booking_id": "B202" }),
        );

        let barrier = Arc::new(Barrier::new(3));
        let runtime_root_one = runtime_root.clone();
        let runtime_root_two = runtime_root.clone();
        let first_record = first.clone();
        let second_record = second.clone();

        let first_handle = {
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                append_support_error_record(&runtime_root_one, &first_record)
                    .expect("append first");
            })
        };

        let second_handle = {
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                append_support_error_record(&runtime_root_two, &second_record)
                    .expect("append second");
            })
        };

        barrier.wait();
        first_handle.join().expect("first writer thread");
        second_handle.join().expect("second writer thread");

        let contents =
            fs::read_to_string(support_log_path(&runtime_root)).expect("support log should exist");
        let lines = contents.lines().collect::<Vec<_>>();

        assert_eq!(lines.len(), 2);

        let parsed = lines
            .iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("json line"))
            .collect::<Vec<_>>();

        assert!(parsed
            .iter()
            .any(|line| line["support_id"] == "SUP-AAAA0001"));
        assert!(parsed
            .iter()
            .any(|line| line["support_id"] == "SUP-BBBB0002"));
        assert!(parsed
            .iter()
            .any(|line| line["context"]["room_id"] == "R101"));
        assert!(parsed
            .iter()
            .any(|line| line["context"]["booking_id"] == "B202"));
        assert!(parsed.iter().all(|line| line.get("room_id").is_none()));
        assert!(parsed.iter().all(|line| line.get("booking_id").is_none()));
    }

    #[test]
    fn append_support_error_record_keeps_conflicting_context_keys_nested() {
        let runtime_root = test_runtime_root("conflict");
        let _ = fs::remove_dir_all(&runtime_root);

        let record = SupportErrorRecord::new(
            "login",
            crate::app_error::codes::SYSTEM_INTERNAL_ERROR,
            "database offline",
            "SUP-CCCC0003",
            json!({
                "code": "OVERRIDE",
                "support_id": "SHADOW",
                "timestamp": "2000-01-01T00:00:00Z",
                "room_id": "R303"
            }),
        );

        append_support_error_record(&runtime_root, &record).expect("append record");

        let contents =
            fs::read_to_string(support_log_path(&runtime_root)).expect("support log should exist");
        let parsed: serde_json::Value = serde_json::from_str(contents.trim()).expect("json line");

        assert_eq!(parsed["command"], "login");
        assert_eq!(
            parsed["code"],
            crate::app_error::codes::SYSTEM_INTERNAL_ERROR
        );
        assert_eq!(parsed["support_id"], "SUP-CCCC0003");
        assert_eq!(parsed["context"]["code"], "OVERRIDE");
        assert_eq!(parsed["context"]["support_id"], "SHADOW");
        assert_eq!(parsed["context"]["timestamp"], "2000-01-01T00:00:00Z");
        assert_eq!(parsed["context"]["room_id"], "R303");
        assert_ne!(parsed["support_id"], parsed["context"]["support_id"]);
        assert_eq!(parsed.get("room_id"), None);

        let _ = fs::remove_dir_all(&runtime_root);
    }
}
