use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::app_error::AppErrorKind;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandFailureRecord {
    pub schema_version: u32,
    pub timestamp: String,
    pub command: String,
    pub code: String,
    pub kind: AppErrorKind,
    pub correlation_id: Option<String>,
    pub support_id: Option<String>,
    pub context: Value,
}

impl CommandFailureRecord {
    pub fn new(
        command: impl Into<String>,
        code: impl Into<String>,
        kind: AppErrorKind,
        correlation_id: Option<impl Into<String>>,
        support_id: Option<impl Into<String>>,
        context: Value,
    ) -> Self {
        Self {
            schema_version: 1,
            timestamp: Utc::now().to_rfc3339(),
            command: command.into(),
            code: code.into(),
            kind,
            correlation_id: correlation_id.map(Into::into),
            support_id: support_id.map(Into::into),
            context,
        }
    }
}

pub fn command_failure_log_path(runtime_root: &Path) -> PathBuf {
    runtime_root
        .join("diagnostics")
        .join("command-failures.jsonl")
}

fn append_mutex() -> &'static Mutex<()> {
    static APPEND_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
    APPEND_MUTEX.get_or_init(|| Mutex::new(()))
}

pub fn append_command_failure_record(
    runtime_root: &Path,
    record: &CommandFailureRecord,
) -> Result<(), String> {
    let _guard = append_mutex().lock().map_err(|error| error.to_string())?;
    let path = command_failure_log_path(runtime_root);

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
    use crate::app_error::{codes, correlation_context};
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Barrier};
    use std::thread;

    fn test_runtime_root(test_name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "capyinn-command-failure-log-{}-{}",
            test_name,
            uuid::Uuid::new_v4()
        ))
    }

    #[test]
    fn command_failure_log_path_stays_under_diagnostics() {
        let root = PathBuf::from("/tmp/capyinn-runtime-root");
        assert_eq!(
            command_failure_log_path(&root),
            root.join("diagnostics").join("command-failures.jsonl")
        );
    }

    #[test]
    fn append_command_failure_record_writes_json_lines_without_interleaving() {
        let runtime_root = test_runtime_root("append");
        let _ = fs::remove_dir_all(&runtime_root);

        let first = CommandFailureRecord::new(
            "login",
            codes::AUTH_INVALID_PIN,
            AppErrorKind::User,
            Some("COR-AAAA0001"),
            None::<String>,
            correlation_context("COR-AAAA0001", json!({ "room_id": "R101" })),
        );
        let second = CommandFailureRecord::new(
            "check_out",
            codes::SYSTEM_INTERNAL_ERROR,
            AppErrorKind::System,
            Some("COR-BBBB0002"),
            Some("SUP-BBBB0002"),
            correlation_context("COR-BBBB0002", json!({ "booking_id": "B202" })),
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
                append_command_failure_record(&runtime_root_one, &first_record)
                    .expect("append first");
            })
        };

        let second_handle = {
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                append_command_failure_record(&runtime_root_two, &second_record)
                    .expect("append second");
            })
        };

        barrier.wait();
        first_handle.join().expect("first writer thread");
        second_handle.join().expect("second writer thread");

        let contents = fs::read_to_string(command_failure_log_path(&runtime_root))
            .expect("command failure log should exist");
        let lines = contents.lines().collect::<Vec<_>>();

        assert_eq!(lines.len(), 2);

        let parsed = lines
            .iter()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("json line"))
            .collect::<Vec<_>>();

        assert!(parsed.iter().all(|line| line["schema_version"] == 1));
        assert!(parsed
            .iter()
            .any(|line| line["command"] == "login" && line["support_id"].is_null()));
        assert!(parsed
            .iter()
            .any(|line| line["command"] == "check_out" && line["support_id"] == "SUP-BBBB0002"));
        assert!(parsed
            .iter()
            .any(|line| line["context"]["room_id"] == "R101"));
        assert!(parsed
            .iter()
            .any(|line| line["context"]["booking_id"] == "B202"));
        assert!(parsed.iter().all(|line| line.get("room_id").is_none()));
        assert!(parsed.iter().all(|line| line.get("booking_id").is_none()));

        let _ = fs::remove_dir_all(&runtime_root);
    }

    #[test]
    fn append_command_failure_record_keeps_context_nested_and_schema_version_is_one() {
        let runtime_root = test_runtime_root("nested");
        let _ = fs::remove_dir_all(&runtime_root);

        let record = CommandFailureRecord::new(
            "create_room",
            codes::ROOM_ALREADY_EXISTS,
            AppErrorKind::User,
            Some("COR-CCCC0003"),
            None::<String>,
            json!({
                "correlation_id": "COR-CCCC0003",
                "room_id": "R303",
                "timestamp": "2000-01-01T00:00:00Z",
            }),
        );

        append_command_failure_record(&runtime_root, &record).expect("append record");

        let contents = fs::read_to_string(command_failure_log_path(&runtime_root))
            .expect("command failure log should exist");
        let parsed: serde_json::Value = serde_json::from_str(contents.trim()).expect("json line");

        assert_eq!(parsed["schema_version"], 1);
        assert_eq!(parsed["command"], "create_room");
        assert_eq!(parsed["code"], codes::ROOM_ALREADY_EXISTS);
        assert_eq!(parsed["kind"], "user");
        assert_eq!(parsed["correlation_id"], "COR-CCCC0003");
        assert!(parsed["support_id"].is_null());
        assert!(
            parsed["timestamp"]
                .as_str()
                .expect("top-level timestamp")
                .len()
                > 0
        );
        assert_eq!(parsed["context"]["correlation_id"], "COR-CCCC0003");
        assert_eq!(parsed["context"]["room_id"], "R303");
        assert_eq!(parsed["context"]["timestamp"], "2000-01-01T00:00:00Z");
        assert_eq!(parsed.get("room_id"), None);

        let _ = fs::remove_dir_all(&runtime_root);
    }
}
