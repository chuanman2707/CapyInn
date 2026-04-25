use super::{storage::latest_backup_timestamp_for_reason, BackupError, BackupReason};
use chrono::{Duration as ChronoDuration, NaiveDateTime, Utc};
use std::{path::Path, time::Duration};

#[allow(dead_code)]
const SCHEDULED_BACKUP_INTERVAL: Duration = Duration::from_secs(60 * 60);

#[allow(dead_code)]
fn scheduled_backup_interval_chrono() -> ChronoDuration {
    ChronoDuration::from_std(SCHEDULED_BACKUP_INTERVAL)
        .expect("scheduled backup interval must fit in chrono duration")
}

#[allow(dead_code)]
fn scheduler_now() -> NaiveDateTime {
    Utc::now().naive_utc()
}

#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum StartupCatchUpError<ScanError, RequestError> {
    Scan(ScanError),
    Request(RequestError),
}

#[allow(dead_code)]
pub(crate) fn should_run_startup_catch_up(
    latest_scheduled: Option<NaiveDateTime>,
    now: NaiveDateTime,
) -> bool {
    match latest_scheduled {
        Some(timestamp) => now - timestamp > scheduled_backup_interval_chrono(),
        None => true,
    }
}

#[allow(dead_code)]
pub(crate) fn latest_scheduled_backup_timestamp(
    backup_dir: &Path,
) -> Result<Option<NaiveDateTime>, BackupError> {
    latest_backup_timestamp_for_reason(backup_dir, BackupReason::Scheduled)
}

#[allow(dead_code)]
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
}
