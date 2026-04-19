use super::BackupReason;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize)]
pub struct BackupStatusPayload {
    pub job_id: String,
    pub state: &'static str,
    pub reason: &'static str,
    pub pending_jobs: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl BackupStatusPayload {
    pub(crate) fn started(job_id: usize, reason: BackupReason, pending_jobs: usize) -> Self {
        Self {
            job_id: format!("job-{job_id}"),
            state: "started",
            reason: reason.as_str(),
            pending_jobs,
            path: None,
            message: None,
        }
    }

    pub(crate) fn completed(
        job_id: usize,
        reason: BackupReason,
        pending_jobs: usize,
        path: PathBuf,
    ) -> Self {
        Self {
            job_id: format!("job-{job_id}"),
            state: "completed",
            reason: reason.as_str(),
            pending_jobs,
            path: Some(path.to_string_lossy().to_string()),
            message: None,
        }
    }

    pub(crate) fn failed(
        job_id: usize,
        reason: BackupReason,
        pending_jobs: usize,
        message: String,
    ) -> Self {
        Self {
            job_id: format!("job-{job_id}"),
            state: "failed",
            reason: reason.as_str(),
            pending_jobs,
            path: None,
            message: Some(message),
        }
    }
}

pub(crate) fn emit_backup_status(app: &AppHandle, payload: BackupStatusPayload) {
    let _ = app.emit("backup-status", payload);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn completed_payload_uses_existing_wire_contract() {
        let payload = BackupStatusPayload::completed(
            7,
            BackupReason::Manual,
            0,
            PathBuf::from("/tmp/demo.db"),
        );

        assert_eq!(payload.job_id, "job-7");
        assert_eq!(payload.state, "completed");
        assert_eq!(payload.reason, "manual");
        assert_eq!(payload.pending_jobs, 0);
        assert_eq!(payload.path.as_deref(), Some("/tmp/demo.db"));
        assert!(payload.message.is_none());
    }
}
