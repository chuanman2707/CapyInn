use super::{
    log_backup_request_error, request_backup, storage::latest_backup_timestamp_for_reason,
    BackupError, BackupReason, BackupRequestError,
};
use chrono::{Duration as ChronoDuration, NaiveDateTime, Utc};
use log::{error, info, warn};
use std::{future::Future, path::Path, sync::Mutex, time::Duration};
use tauri::AppHandle;
use tokio::sync::oneshot;

const SCHEDULED_BACKUP_INTERVAL: Duration = Duration::from_secs(60 * 60);

fn scheduled_backup_interval_chrono() -> ChronoDuration {
    ChronoDuration::from_std(SCHEDULED_BACKUP_INTERVAL)
        .expect("scheduled backup interval must fit in chrono duration")
}

fn scheduler_now() -> NaiveDateTime {
    crate::runtime_config::test_now()
        .map(|value| value.naive_local())
        .unwrap_or_else(|| Utc::now().naive_utc())
}

pub struct BackupSchedulerHandle {
    task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    shutdown_tx: Mutex<Option<oneshot::Sender<()>>>,
}

impl BackupSchedulerHandle {
    pub(crate) fn new(
        task: tauri::async_runtime::JoinHandle<()>,
        shutdown_tx: oneshot::Sender<()>,
    ) -> Self {
        Self {
            task: Mutex::new(Some(task)),
            shutdown_tx: Mutex::new(Some(shutdown_tx)),
        }
    }

    pub fn shutdown(&self) {
        let shutdown_tx = self
            .shutdown_tx
            .lock()
            .ok()
            .and_then(|mut guard| guard.take());

        if let Some(shutdown_tx) = shutdown_tx {
            let _ = shutdown_tx.send(());
        }

        // Do not abort: the task may be inside BackupCoordinator. Dropping the
        // join handle detaches it so any active backup can finish and drain.
        let _task = self.task.lock().ok().and_then(|mut guard| guard.take());
    }
}

pub fn start_backup_scheduler(app: AppHandle) -> BackupSchedulerHandle {
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let task = tauri::async_runtime::spawn(async move {
        run_backup_scheduler(app, shutdown_rx).await;
    });

    BackupSchedulerHandle::new(task, shutdown_tx)
}

#[derive(Debug)]
pub(crate) enum StartupCatchUpError<ScanError, RequestError> {
    Scan(ScanError),
    Request(RequestError),
}

pub(crate) fn should_run_startup_catch_up(
    latest_scheduled: Option<NaiveDateTime>,
    now: NaiveDateTime,
) -> bool {
    match latest_scheduled {
        Some(timestamp) => now - timestamp > scheduled_backup_interval_chrono(),
        None => true,
    }
}

pub(crate) fn latest_scheduled_backup_timestamp(
    backup_dir: &Path,
) -> Result<Option<NaiveDateTime>, BackupError> {
    latest_backup_timestamp_for_reason(backup_dir, BackupReason::Scheduled)
}

pub(crate) async fn run_startup_catch_up_with<
    Scan,
    Request,
    RequestFuture,
    ScanError,
    RequestError,
>(
    scan_latest: Scan,
    request_backup: Request,
    now: NaiveDateTime,
) -> Result<(), StartupCatchUpError<ScanError, RequestError>>
where
    Scan: FnOnce() -> Result<Option<NaiveDateTime>, ScanError>,
    Request: FnOnce() -> RequestFuture,
    RequestFuture: std::future::Future<Output = Result<(), RequestError>>,
{
    let latest_scheduled = scan_latest().map_err(StartupCatchUpError::Scan)?;
    if should_run_startup_catch_up(latest_scheduled, now) {
        request_backup()
            .await
            .map_err(StartupCatchUpError::Request)?;
    }
    Ok(())
}

async fn run_backup_scheduler(app: AppHandle, shutdown_rx: oneshot::Receiver<()>) {
    run_startup_catch_up(&app).await;
    run_interval_loop_with(
        || request_scheduled_backup(&app, "scheduled backup interval"),
        shutdown_rx,
        SCHEDULED_BACKUP_INTERVAL,
    )
    .await;
}

async fn run_interval_loop_with<Request, RequestFuture>(
    mut request_backup: Request,
    mut shutdown_rx: oneshot::Receiver<()>,
    interval: Duration,
) where
    Request: FnMut() -> RequestFuture,
    RequestFuture: Future<Output = Result<(), BackupRequestError>>,
{
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown_rx => {
                break;
            }
            _ = tokio::time::sleep(interval) => {
                let _ = request_backup().await;
            }
        }
    }
}

async fn run_startup_catch_up(app: &AppHandle) {
    let Some(runtime_root) = crate::app_identity::runtime_root_opt() else {
        warn!("scheduled backup startup catch-up skipped: cannot find home directory");
        return;
    };

    let backup_dir = runtime_root.join("backups");
    match run_startup_catch_up_with(
        || latest_scheduled_backup_timestamp(&backup_dir),
        || request_scheduled_backup(app, "scheduled backup startup catch-up"),
        scheduler_now(),
    )
    .await
    {
        Ok(()) => {
            info!("scheduled backup startup catch-up completed");
        }
        Err(StartupCatchUpError::Scan(error)) => {
            error!("scheduled backup startup catch-up scan failed: {error}");
        }
        Err(StartupCatchUpError::Request(_)) => {}
    }
}

async fn request_scheduled_backup(
    app: &AppHandle,
    context: &str,
) -> Result<(), BackupRequestError> {
    request_backup(app, BackupReason::Scheduled)
        .await
        .inspect_err(|error| {
            log_backup_request_error(context, error);
        })
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::{
        build_backup_filename,
        test_support::{backup_file_name, make_temp_dir},
        BackupReason,
    };
    use chrono::{NaiveDate, NaiveDateTime};
    use std::fs;

    fn fixed_time(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(hour, minute, second)
            .unwrap()
    }

    #[test]
    fn startup_catch_up_runs_when_no_scheduled_backup_exists() {
        assert!(should_run_startup_catch_up(
            None,
            fixed_time(2026, 4, 25, 10, 0, 0),
        ));
    }

    #[test]
    fn startup_catch_up_runs_when_latest_scheduled_backup_is_older_than_one_hour() {
        assert!(should_run_startup_catch_up(
            Some(fixed_time(2026, 4, 25, 8, 59, 59)),
            fixed_time(2026, 4, 25, 10, 0, 0),
        ));
    }

    #[test]
    fn startup_catch_up_skips_when_latest_scheduled_backup_is_within_one_hour() {
        assert!(!should_run_startup_catch_up(
            Some(fixed_time(2026, 4, 25, 9, 0, 0)),
            fixed_time(2026, 4, 25, 10, 0, 0),
        ));
        assert!(!should_run_startup_catch_up(
            Some(fixed_time(2026, 4, 25, 9, 30, 0)),
            fixed_time(2026, 4, 25, 10, 0, 0),
        ));
    }

    #[test]
    fn latest_scheduled_backup_timestamp_uses_storage_parser() {
        let temp = make_temp_dir("scheduler-latest");
        let backup_dir = temp.join("backups");
        fs::create_dir_all(&backup_dir).unwrap();

        let scheduled = fixed_time(2026, 4, 25, 9, 0, 0);
        let manual = fixed_time(2026, 4, 25, 10, 0, 0);
        let scheduled_path =
            backup_dir.join(build_backup_filename(BackupReason::Scheduled, scheduled));
        let manual_path = backup_dir.join(build_backup_filename(BackupReason::Manual, manual));

        fs::write(&scheduled_path, b"db").unwrap();
        fs::write(&manual_path, b"db").unwrap();

        let latest = latest_scheduled_backup_timestamp(&backup_dir).unwrap();

        assert_eq!(
            backup_file_name(&scheduled_path),
            "capyinn_backup_scheduled_20260425_090000.db"
        );
        assert_eq!(latest, Some(scheduled));
    }

    #[test]
    fn scheduler_now_uses_frozen_backup_clock_when_present() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();

        std::env::set_var("CAPYINN_TEST_NOW", "2026-04-21T09:15:00+07:00");
        let timestamp = scheduler_now();
        std::env::remove_var("CAPYINN_TEST_NOW");

        assert_eq!(timestamp, fixed_time(2026, 4, 21, 9, 15, 0));
    }

    #[tokio::test]
    async fn startup_catch_up_requests_backup_when_due() {
        let requested = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let requested_for_call = requested.clone();

        run_startup_catch_up_with(
            || Ok::<Option<NaiveDateTime>, BackupError>(None),
            move || async move {
                requested_for_call.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok::<(), BackupError>(())
            },
            fixed_time(2026, 4, 25, 10, 0, 0),
        )
        .await
        .unwrap();

        assert!(requested.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn startup_catch_up_skips_backup_when_recent() {
        let requested = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let requested_for_call = requested.clone();

        run_startup_catch_up_with(
            || Ok::<Option<NaiveDateTime>, BackupError>(Some(fixed_time(2026, 4, 25, 9, 30, 0))),
            move || async move {
                requested_for_call.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok::<(), BackupError>(())
            },
            fixed_time(2026, 4, 25, 10, 0, 0),
        )
        .await
        .unwrap();

        assert!(!requested.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn startup_catch_up_returns_scan_errors_without_requesting_backup() {
        let requested = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let requested_for_call = requested.clone();
        let temp = make_temp_dir("scheduler-scan-error");
        let backup_dir = temp.join("backups");
        fs::write(&backup_dir, b"not a directory").unwrap();

        let result = run_startup_catch_up_with(
            || latest_scheduled_backup_timestamp(&backup_dir),
            move || async move {
                requested_for_call.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok::<(), BackupError>(())
            },
            fixed_time(2026, 4, 25, 10, 0, 0),
        )
        .await;

        assert!(matches!(result, Err(StartupCatchUpError::Scan(_))));
        assert!(!requested.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn startup_catch_up_returns_request_errors() {
        let requested = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let requested_for_call = requested.clone();

        let result = run_startup_catch_up_with(
            || Ok::<Option<NaiveDateTime>, BackupError>(None),
            move || async move {
                requested_for_call.store(true, std::sync::atomic::Ordering::SeqCst);
                Err(BackupError::Io(std::io::Error::other("request failed")))
            },
            fixed_time(2026, 4, 25, 10, 0, 0),
        )
        .await;

        assert!(matches!(result, Err(StartupCatchUpError::Request(_))));
        assert!(requested.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn scheduler_shutdown_is_idempotent() {
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        let handle = BackupSchedulerHandle::new(
            tauri::async_runtime::spawn(async {
                std::future::pending::<()>().await;
            }),
            shutdown_tx,
        );

        handle.shutdown();
        handle.shutdown();
    }

    #[tokio::test]
    async fn scheduler_shutdown_does_not_abort_in_flight_task() {
        let (shutdown_tx, _shutdown_rx) = oneshot::channel();
        let completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let completed_for_task = completed.clone();
        let handle = BackupSchedulerHandle::new(
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                completed_for_task.store(true, std::sync::atomic::Ordering::SeqCst);
            }),
            shutdown_tx,
        );

        handle.shutdown();
        tokio::time::sleep(Duration::from_millis(30)).await;

        assert!(completed.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn scheduler_interval_shutdown_waits_for_in_flight_request() {
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let request_started = std::sync::Arc::new(tokio::sync::Notify::new());
        let release_request = std::sync::Arc::new(tokio::sync::Notify::new());
        let request_completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));

        let request_started_for_loop = request_started.clone();
        let release_request_for_loop = release_request.clone();
        let request_completed_for_loop = request_completed.clone();
        let scheduler_task = tokio::spawn(async move {
            run_interval_loop_with(
                move || {
                    let request_started = request_started_for_loop.clone();
                    let release_request = release_request_for_loop.clone();
                    let request_completed = request_completed_for_loop.clone();
                    async move {
                        request_started.notify_one();
                        release_request.notified().await;
                        request_completed.store(true, std::sync::atomic::Ordering::SeqCst);
                        Ok::<(), BackupRequestError>(())
                    }
                },
                shutdown_rx,
                Duration::from_millis(1),
            )
            .await;
        });

        request_started.notified().await;
        let _ = shutdown_tx.send(());
        tokio::time::sleep(Duration::from_millis(10)).await;

        assert!(!scheduler_task.is_finished());
        assert!(!request_completed.load(std::sync::atomic::Ordering::SeqCst));

        release_request.notify_one();
        scheduler_task.await.unwrap();

        assert!(request_completed.load(std::sync::atomic::Ordering::SeqCst));
    }
}
