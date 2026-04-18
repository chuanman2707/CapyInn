use serde::Serialize;
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager};

mod runner;
mod storage;
#[cfg(test)]
mod test_support;
mod types;

#[allow(unused_imports)]
pub use storage::{build_backup_filename, is_managed_backup_file, prune_old_backups};
pub use runner::run_backup_once;
#[allow(unused_imports)]
pub use types::BackupRequestErrorKind;
#[allow(unused_imports)]
pub use types::{
    log_backup_request_error, BackupError, BackupOutcome, BackupPruneOutcome, BackupReason,
    BackupRequestError,
};

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
    fn started(job_id: usize, reason: BackupReason, pending_jobs: usize) -> Self {
        Self {
            job_id: format!("job-{job_id}"),
            state: "started",
            reason: reason.as_str(),
            pending_jobs,
            path: None,
            message: None,
        }
    }

    fn completed(job_id: usize, reason: BackupReason, pending_jobs: usize, path: PathBuf) -> Self {
        Self {
            job_id: format!("job-{job_id}"),
            state: "completed",
            reason: reason.as_str(),
            pending_jobs,
            path: Some(path.to_string_lossy().to_string()),
            message: None,
        }
    }

    fn failed(job_id: usize, reason: BackupReason, pending_jobs: usize, message: String) -> Self {
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

#[derive(Default)]
pub struct BackupCoordinator {
    gate: tokio::sync::Mutex<()>,
    pending_jobs: AtomicUsize,
    shutdown_started: AtomicBool,
    exit_drain_started: AtomicBool,
    next_job_id: AtomicUsize,
}

impl BackupCoordinator {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mark_shutdown_started(&self) {
        self.shutdown_started.store(true, Ordering::SeqCst);
    }

    pub fn begin_exit_drain(&self) -> bool {
        self.mark_shutdown_started();
        self.exit_drain_started
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    pub fn ensure_request_allowed(&self, reason: BackupReason) -> Result<(), BackupRequestError> {
        if self.shutdown_started.load(Ordering::SeqCst) && reason != BackupReason::AppExit {
            return Err(BackupRequestError::ShutdownInProgress);
        }
        Ok(())
    }

    pub async fn request_backup(
        &self,
        app: &AppHandle,
        reason: BackupReason,
    ) -> Result<BackupOutcome, BackupRequestError> {
        self.request_backup_with(
            reason,
            |payload| emit_backup_status(app, payload),
            || async {
                let db_path = crate::app_identity::database_path_opt()
                    .ok_or(BackupRequestError::MissingHomeDirectory)?;
                let runtime_root = crate::app_identity::runtime_root_opt()
                    .ok_or(BackupRequestError::MissingHomeDirectory)?;

                run_backup_once(&db_path, &runtime_root, reason)
                    .await
                    .map_err(BackupRequestError::BackupFailed)
            },
        )
        .await
    }

    async fn request_backup_with<Emit, Run, RunFuture>(
        &self,
        reason: BackupReason,
        emit: Emit,
        run_backup: Run,
    ) -> Result<BackupOutcome, BackupRequestError>
    where
        Emit: Fn(BackupStatusPayload),
        Run: FnOnce() -> RunFuture,
        RunFuture: std::future::Future<Output = Result<BackupOutcome, BackupRequestError>>,
    {
        self.request_backup_with_before_enqueue(reason, emit, || async {}, run_backup)
            .await
    }

    async fn request_backup_with_before_enqueue<
        Emit,
        BeforeEnqueue,
        BeforeEnqueueFuture,
        Run,
        RunFuture,
    >(
        &self,
        reason: BackupReason,
        emit: Emit,
        before_enqueue: BeforeEnqueue,
        run_backup: Run,
    ) -> Result<BackupOutcome, BackupRequestError>
    where
        Emit: Fn(BackupStatusPayload),
        BeforeEnqueue: FnOnce() -> BeforeEnqueueFuture,
        BeforeEnqueueFuture: std::future::Future<Output = ()>,
        Run: FnOnce() -> RunFuture,
        RunFuture: std::future::Future<Output = Result<BackupOutcome, BackupRequestError>>,
    {
        self.ensure_request_allowed(reason)?;
        before_enqueue().await;
        self.pending_jobs.fetch_add(1, Ordering::SeqCst);

        if let Err(error) = self.ensure_request_allowed(reason) {
            self.pending_jobs.fetch_sub(1, Ordering::SeqCst);
            return Err(error);
        }

        let job_id = self.next_job_id.fetch_add(1, Ordering::SeqCst) + 1;
        let _guard = self.gate.lock().await;

        let started_pending_jobs = self.pending_jobs.load(Ordering::SeqCst);
        emit(BackupStatusPayload::started(
            job_id,
            reason,
            started_pending_jobs,
        ));

        let result = run_backup().await;

        let remaining = self.pending_jobs.fetch_sub(1, Ordering::SeqCst) - 1;
        match &result {
            Ok(outcome) => emit(BackupStatusPayload::completed(
                job_id,
                reason,
                remaining,
                outcome.path.clone(),
            )),
            Err(error) => emit(BackupStatusPayload::failed(
                job_id,
                reason,
                remaining,
                error.to_string(),
            )),
        }

        result
    }

    pub async fn drain_and_backup_on_exit(
        &self,
        app: &AppHandle,
    ) -> Result<(), BackupRequestError> {
        self.drain_and_backup_with(
            |payload| emit_backup_status(app, payload),
            || async {
                let db_path = crate::app_identity::database_path_opt()
                    .ok_or(BackupRequestError::MissingHomeDirectory)?;
                let runtime_root = crate::app_identity::runtime_root_opt()
                    .ok_or(BackupRequestError::MissingHomeDirectory)?;

                run_backup_once(&db_path, &runtime_root, BackupReason::AppExit)
                    .await
                    .map_err(BackupRequestError::BackupFailed)
            },
        )
        .await
    }

    async fn drain_and_backup_with<Emit, Run, RunFuture>(
        &self,
        emit: Emit,
        run_backup: Run,
    ) -> Result<(), BackupRequestError>
    where
        Emit: Fn(BackupStatusPayload),
        Run: FnOnce() -> RunFuture,
        RunFuture: std::future::Future<Output = Result<BackupOutcome, BackupRequestError>>,
    {
        self.mark_shutdown_started();

        tokio::time::timeout(Duration::from_secs(10), async {
            let _guard = self.gate.lock().await;
            drop(_guard);
            self.request_backup_with(BackupReason::AppExit, emit, run_backup)
                .await
                .map(|_| ())
        })
        .await
        .map_err(|_| BackupRequestError::ShutdownTimedOut)?
    }
}

pub async fn request_backup(
    app: &AppHandle,
    reason: BackupReason,
) -> Result<BackupOutcome, BackupRequestError> {
    let coordinator = app.state::<BackupCoordinator>();
    coordinator.request_backup(app, reason).await
}

pub async fn drain_and_backup_on_exit(app: &AppHandle) -> Result<(), BackupRequestError> {
    let coordinator = app.state::<BackupCoordinator>();
    coordinator.drain_and_backup_on_exit(app).await
}

fn emit_backup_status(app: &AppHandle, payload: BackupStatusPayload) {
    let _ = app.emit("backup-status", payload);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::test_support::fake_backup_outcome;
    use chrono::NaiveDate;
    use std::sync::{Arc, Mutex};
    use tokio::sync::Notify;

    #[test]
    fn builds_reason_tagged_backup_filename() {
        let timestamp = NaiveDate::from_ymd_opt(2026, 4, 18)
            .unwrap()
            .and_hms_opt(23, 15, 0)
            .unwrap();

        assert_eq!(
            build_backup_filename(BackupReason::Checkout, timestamp),
            "capyinn_backup_checkout_20260418_231500.db"
        );
    }

    #[tokio::test]
    async fn rejects_non_exit_requests_after_shutdown_starts() {
        let coordinator = BackupCoordinator::new();
        coordinator.mark_shutdown_started();

        let result = coordinator.ensure_request_allowed(BackupReason::Settings);

        assert!(matches!(
            result,
            Err(BackupRequestError::ShutdownInProgress)
        ));
    }

    #[test]
    fn typed_request_errors_drive_skip_vs_failure_logging() {
        assert_eq!(
            BackupRequestError::ShutdownInProgress.kind(),
            BackupRequestErrorKind::ShutdownSkip
        );
        assert_eq!(
            BackupRequestError::MissingHomeDirectory.kind(),
            BackupRequestErrorKind::Failure
        );
    }

    #[tokio::test]
    async fn request_backup_serializes_jobs_and_emits_live_pending_counts() {
        let coordinator = Arc::new(BackupCoordinator::new());
        let events = Arc::new(Mutex::new(Vec::<BackupStatusPayload>::new()));
        let first_entered = Arc::new(Notify::new());
        let release_first = Arc::new(Notify::new());

        let first = {
            let coordinator = coordinator.clone();
            let events = events.clone();
            let first_entered = first_entered.clone();
            let release_first = release_first.clone();
            tokio::spawn(async move {
                coordinator
                    .request_backup_with(
                        BackupReason::Manual,
                        move |payload| events.lock().unwrap().push(payload),
                        || async move {
                            first_entered.notify_one();
                            release_first.notified().await;
                            Ok(fake_backup_outcome("first.db"))
                        },
                    )
                    .await
            })
        };

        first_entered.notified().await;

        let second = {
            let coordinator = coordinator.clone();
            let events = events.clone();
            tokio::spawn(async move {
                coordinator
                    .request_backup_with(
                        BackupReason::Manual,
                        move |payload| events.lock().unwrap().push(payload),
                        || async { Ok(fake_backup_outcome("second.db")) },
                    )
                    .await
            })
        };

        tokio::time::timeout(Duration::from_secs(1), async {
            while coordinator.pending_jobs.load(Ordering::SeqCst) != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("second job should queue");

        release_first.notify_one();

        let first = first.await.unwrap().unwrap();
        let second = second.await.unwrap().unwrap();
        assert_eq!(first.path, PathBuf::from("/tmp/first.db"));
        assert_eq!(second.path, PathBuf::from("/tmp/second.db"));

        let events = events.lock().unwrap().clone();
        let actual = events
            .iter()
            .map(|payload| {
                (
                    payload.state,
                    payload.reason,
                    payload.pending_jobs,
                    payload.path.clone(),
                )
            })
            .collect::<Vec<_>>();

        let expected = vec![
            ("started", "manual", 1, None),
            ("completed", "manual", 1, Some("/tmp/first.db".to_string())),
            ("started", "manual", 1, None),
            ("completed", "manual", 0, Some("/tmp/second.db".to_string())),
        ];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn begin_exit_drain_blocks_new_jobs_and_drains_with_app_exit_backup() {
        let coordinator = Arc::new(BackupCoordinator::new());
        let events = Arc::new(Mutex::new(Vec::<BackupStatusPayload>::new()));
        let first_entered = Arc::new(Notify::new());
        let release_first = Arc::new(Notify::new());
        let exit_started = Arc::new(Notify::new());

        let first = {
            let coordinator = coordinator.clone();
            let events = events.clone();
            let first_entered = first_entered.clone();
            let release_first = release_first.clone();
            tokio::spawn(async move {
                coordinator
                    .request_backup_with(
                        BackupReason::Manual,
                        move |payload| events.lock().unwrap().push(payload),
                        || async move {
                            first_entered.notify_one();
                            release_first.notified().await;
                            Ok(fake_backup_outcome("manual.db"))
                        },
                    )
                    .await
            })
        };

        first_entered.notified().await;

        assert!(coordinator.begin_exit_drain());
        assert!(!coordinator.begin_exit_drain());

        let skipped = coordinator
            .request_backup_with(
                BackupReason::Settings,
                |_| panic!("shutdown-skipped requests must not emit backup status"),
                || async { Ok(fake_backup_outcome("skipped.db")) },
            )
            .await;
        assert!(matches!(
            skipped,
            Err(BackupRequestError::ShutdownInProgress)
        ));

        let drain = {
            let coordinator = coordinator.clone();
            let events = events.clone();
            let exit_started = exit_started.clone();
            tokio::spawn(async move {
                coordinator
                    .drain_and_backup_with(
                        move |payload| events.lock().unwrap().push(payload),
                        || async move {
                            exit_started.notify_one();
                            Ok(fake_backup_outcome("app-exit.db"))
                        },
                    )
                    .await
            })
        };

        assert!(
            tokio::time::timeout(Duration::from_millis(50), exit_started.notified())
                .await
                .is_err(),
            "exit backup should wait for the in-flight job to finish"
        );

        release_first.notify_one();

        first.await.unwrap().unwrap();
        drain.await.unwrap().unwrap();

        let events = events.lock().unwrap().clone();
        let actual = events
            .iter()
            .map(|payload| {
                (
                    payload.state,
                    payload.reason,
                    payload.pending_jobs,
                    payload.path.clone(),
                )
            })
            .collect::<Vec<_>>();

        let expected = vec![
            ("started", "manual", 1, None),
            ("completed", "manual", 0, Some("/tmp/manual.db".to_string())),
            ("started", "app_exit", 1, None),
            (
                "completed",
                "app_exit",
                0,
                Some("/tmp/app-exit.db".to_string()),
            ),
        ];

        assert_eq!(actual, expected);
    }

    #[tokio::test]
    async fn request_backup_rejects_job_that_loses_admission_race_to_shutdown() {
        let coordinator = Arc::new(BackupCoordinator::new());
        let before_enqueue_reached = Arc::new(Notify::new());
        let release_before_enqueue = Arc::new(Notify::new());
        let ran_backup = Arc::new(AtomicBool::new(false));

        let request = {
            let coordinator = coordinator.clone();
            let before_enqueue_reached = before_enqueue_reached.clone();
            let release_before_enqueue = release_before_enqueue.clone();
            let ran_backup = ran_backup.clone();
            tokio::spawn(async move {
                coordinator
                    .request_backup_with_before_enqueue(
                        BackupReason::Settings,
                        |_| panic!("rejected requests must not emit backup status"),
                        move || async move {
                            before_enqueue_reached.notify_one();
                            release_before_enqueue.notified().await;
                        },
                        move || async move {
                            ran_backup.store(true, Ordering::SeqCst);
                            Ok(fake_backup_outcome("should-not-run.db"))
                        },
                    )
                    .await
            })
        };

        before_enqueue_reached.notified().await;
        coordinator.mark_shutdown_started();
        release_before_enqueue.notify_one();

        let result = request.await.unwrap();

        assert!(matches!(
            result,
            Err(BackupRequestError::ShutdownInProgress)
        ));
        assert_eq!(coordinator.pending_jobs.load(Ordering::SeqCst), 0);
        assert!(!ran_backup.load(Ordering::SeqCst));
    }
}
