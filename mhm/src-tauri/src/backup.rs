use chrono::NaiveDateTime;
use chrono::Utc;
use serde::Serialize;
use std::{
    fmt, fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackupReason {
    Settings,
    Checkout,
    GroupCheckout,
    NightAudit,
    AppExit,
    Manual,
}

impl BackupReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Settings => "settings",
            Self::Checkout => "checkout",
            Self::GroupCheckout => "group_checkout",
            Self::NightAudit => "night_audit",
            Self::AppExit => "app_exit",
            Self::Manual => "manual",
        }
    }
}

pub fn build_backup_filename(reason: BackupReason, timestamp: NaiveDateTime) -> String {
    format!(
        "capyinn_backup_{}_{}.db",
        reason.as_str(),
        timestamp.format("%Y%m%d_%H%M%S")
    )
}

pub fn is_managed_backup_file(name: &str) -> bool {
    parse_backup_filename(name).is_some()
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct BackupOutcome {
    pub path: PathBuf,
    pub prune: BackupPruneOutcome,
}

#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct BackupPruneOutcome {
    pub kept_files: Vec<PathBuf>,
    pub removed_files: Vec<PathBuf>,
    pub error: Option<BackupError>,
}

#[allow(dead_code)]
#[derive(Debug)]
pub enum BackupError {
    Io(io::Error),
    Sqlx(sqlx::Error),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackupRequestErrorKind {
    ShutdownSkip,
    Failure,
}

#[derive(Debug)]
pub enum BackupRequestError {
    ShutdownInProgress,
    MissingHomeDirectory,
    BackupFailed(BackupError),
    ShutdownTimedOut,
}

impl fmt::Display for BackupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Sqlx(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for BackupError {}

impl From<io::Error> for BackupError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<sqlx::Error> for BackupError {
    fn from(error: sqlx::Error) -> Self {
        Self::Sqlx(error)
    }
}

impl BackupRequestError {
    pub fn kind(&self) -> BackupRequestErrorKind {
        match self {
            Self::ShutdownInProgress => BackupRequestErrorKind::ShutdownSkip,
            Self::MissingHomeDirectory
            | Self::BackupFailed(_)
            | Self::ShutdownTimedOut => BackupRequestErrorKind::Failure,
        }
    }
}

impl fmt::Display for BackupRequestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ShutdownInProgress => {
                write!(formatter, "backup skipped because shutdown is in progress")
            }
            Self::MissingHomeDirectory => write!(formatter, "Cannot find home directory"),
            Self::BackupFailed(error) => write!(formatter, "{error}"),
            Self::ShutdownTimedOut => write!(formatter, "backup timed out during app shutdown"),
        }
    }
}

impl std::error::Error for BackupRequestError {}

pub fn log_backup_request_error(context: &str, error: &BackupRequestError) {
    match error.kind() {
        BackupRequestErrorKind::ShutdownSkip => {
            log::warn!("autobackup skipped after {context}: {error}");
        }
        BackupRequestErrorKind::Failure => {
            log::error!("autobackup failed after {context}: {error}");
        }
    }
}

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
        self.request_backup_with(reason, |payload| emit_backup_status(app, payload), || async {
            let db_path =
                crate::app_identity::database_path_opt().ok_or(BackupRequestError::MissingHomeDirectory)?;
            let runtime_root =
                crate::app_identity::runtime_root_opt().ok_or(BackupRequestError::MissingHomeDirectory)?;

            run_backup_once(&db_path, &runtime_root, reason)
                .await
                .map_err(BackupRequestError::BackupFailed)
        })
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

    async fn request_backup_with_before_enqueue<Emit, BeforeEnqueue, BeforeEnqueueFuture, Run, RunFuture>(
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
        self.drain_and_backup_with(|payload| emit_backup_status(app, payload), || async {
            let db_path =
                crate::app_identity::database_path_opt().ok_or(BackupRequestError::MissingHomeDirectory)?;
            let runtime_root =
                crate::app_identity::runtime_root_opt().ok_or(BackupRequestError::MissingHomeDirectory)?;

            run_backup_once(&db_path, &runtime_root, BackupReason::AppExit)
                .await
                .map_err(BackupRequestError::BackupFailed)
        })
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

pub async fn run_backup_once(
    db_path: &Path,
    runtime_root: &Path,
    reason: BackupReason,
) -> Result<BackupOutcome, BackupError> {
    run_backup_once_at(db_path, runtime_root, reason, Utc::now().naive_utc(), None).await
}

async fn run_backup_once_at(
    db_path: &Path,
    runtime_root: &Path,
    reason: BackupReason,
    timestamp: NaiveDateTime,
    hold_for: Option<std::time::Duration>,
) -> Result<BackupOutcome, BackupError> {
    fs::create_dir_all(runtime_root.join("backups"))?;
    let backup_dir = runtime_root.join("backups");

    let reservation = BackupReservation::acquire(&backup_dir, reason, timestamp)?;
    let final_path = reservation.final_path.clone();
    let temp_path = reservation.temp_path.clone();

    if let Some(duration) = hold_for {
        tokio::time::sleep(duration).await;
    }

    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(false);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;

    let vacuum_target = sqlite_string_literal(&temp_path);
    sqlx::query(&format!("VACUUM INTO {}", vacuum_target))
        .execute(&pool)
        .await?;

    fs::File::open(&temp_path)?.sync_all()?;
    fs::rename(&temp_path, &final_path)?;
    sync_directory(&backup_dir)?;
    drop(reservation);

    let prune = prune_old_backups(&backup_dir, 30);

    Ok(BackupOutcome {
        path: final_path,
        prune,
    })
}

pub fn prune_old_backups(backup_dir: &Path, keep: usize) -> BackupPruneOutcome {
    let mut outcome = BackupPruneOutcome::default();
    if !backup_dir.exists() {
        return outcome;
    }

    let mut backups = Vec::new();
    let entries = match fs::read_dir(backup_dir) {
        Ok(entries) => entries,
        Err(error) => {
            outcome.error = Some(error.into());
            return outcome;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                outcome.error = Some(error.into());
                return outcome;
            }
        };
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy().into_owned();
        if !is_managed_backup_file(&file_name) {
            continue;
        }

        let Some(metadata) = parse_backup_filename(&file_name) else {
            continue;
        };

        backups.push((metadata, file_name, entry.path()));
    }

    backups.sort_by(|left, right| {
        right
            .0
            .timestamp
            .cmp(&left.0.timestamp)
            .then_with(|| right.0.collision_index.cmp(&left.0.collision_index))
            .then_with(|| right.1.cmp(&left.1))
    });

    outcome.kept_files = backups
        .iter()
        .take(keep)
        .map(|(_, _, path)| path.clone())
        .collect();

    for (_, _, path) in backups.into_iter().skip(keep) {
        if let Err(error) = fs::remove_file(&path) {
            outcome.error = Some(error.into());
            break;
        }
        outcome.removed_files.push(path);
    }

    outcome
}

#[derive(Clone, Debug)]
struct BackupMetadata {
    timestamp: NaiveDateTime,
    collision_index: u64,
}

struct BackupReservation {
    final_path: PathBuf,
    temp_path: PathBuf,
    lock_path: PathBuf,
    lock_file: Option<fs::File>,
}

impl BackupReservation {
    fn acquire(
        backup_dir: &Path,
        reason: BackupReason,
        timestamp: NaiveDateTime,
    ) -> Result<Self, BackupError> {
        let base_name = build_backup_filename(reason, timestamp);
        let base_stem = base_name.strip_suffix(".db").unwrap();
        let mut collision_index = 0u64;

        loop {
            let candidate_name = if collision_index == 0 {
                base_name.clone()
            } else {
                format!("{base_stem}-{collision_index}.db")
            };
            let final_path = backup_dir.join(candidate_name);
            let lock_path = reservation_lock_path(&final_path);

            if final_path.exists() {
                collision_index += 1;
                continue;
            }

            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
            {
                Ok(lock_file) => {
                    return Ok(Self {
                        temp_path: final_path.with_extension("db.tmp"),
                        final_path,
                        lock_path,
                        lock_file: Some(lock_file),
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    collision_index += 1;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }
}

impl Drop for BackupReservation {
    fn drop(&mut self) {
        if let Some(lock_file) = self.lock_file.take() {
            drop(lock_file);
        }
        let _ = fs::remove_file(&self.temp_path);
        let _ = fs::remove_file(&self.lock_path);
    }
}

fn parse_backup_filename(name: &str) -> Option<BackupMetadata> {
    let stem = name.strip_suffix(".db")?;
    let rest = stem.strip_prefix("capyinn_backup_")?;
    let mut parts = rest.rsplitn(3, '_');
    let time_or_suffix = parts.next()?;
    let date = parts.next()?;
    let reason = parts.next()?;
    let (time, collision_index) = match time_or_suffix.split_once('-') {
        Some((time, suffix)) => (time, suffix.parse().ok()?),
        None => (time_or_suffix, 0),
    };
    parse_backup_reason(reason)?;
    let timestamp =
        NaiveDateTime::parse_from_str(&format!("{}_{}", date, time), "%Y%m%d_%H%M%S").ok()?;

    Some(BackupMetadata {
        timestamp,
        collision_index,
    })
}

fn parse_backup_reason(reason: &str) -> Option<BackupReason> {
    match reason {
        "settings" => Some(BackupReason::Settings),
        "checkout" => Some(BackupReason::Checkout),
        "group_checkout" => Some(BackupReason::GroupCheckout),
        "night_audit" => Some(BackupReason::NightAudit),
        "app_exit" => Some(BackupReason::AppExit),
        "manual" => Some(BackupReason::Manual),
        _ => None,
    }
}

fn sqlite_string_literal(path: &Path) -> String {
    let escaped = path.to_string_lossy().replace('\'', "''");
    format!("'{}'", escaped)
}

fn reservation_lock_path(final_path: &Path) -> PathBuf {
    final_path.with_extension("db.lock")
}

fn emit_backup_status(app: &AppHandle, payload: BackupStatusPayload) {
    let _ = app.emit("backup-status", payload);
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    fs::OpenOptions::new().read(true).open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::{
        env,
        sync::{Arc, Mutex},
        time::{SystemTime, UNIX_EPOCH},
    };
    use tokio::sync::Notify;

    struct BackupFixture {
        db_path: PathBuf,
        runtime_root: PathBuf,
        _guard: TempDirGuard,
    }

    struct TempDirGuard {
        path: PathBuf,
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    impl BackupFixture {
        async fn new() -> Self {
            let root = make_temp_dir("backup-fixture");
            let db_path = root.join("capyinn.db");

            let options = SqliteConnectOptions::new()
                .filename(&db_path)
                .create_if_missing(true);
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(options)
                .await
                .unwrap();

            sqlx::query(
                "CREATE TABLE IF NOT EXISTS demo (id INTEGER PRIMARY KEY AUTOINCREMENT, code TEXT NOT NULL)",
            )
            .execute(&pool)
            .await
            .unwrap();

            Self {
                db_path,
                runtime_root: root.clone(),
                _guard: TempDirGuard { path: root },
            }
        }

        async fn insert_demo_row(&self, code: &str) {
            let options = SqliteConnectOptions::new()
                .filename(&self.db_path)
                .create_if_missing(false);
            let pool = SqlitePoolOptions::new()
                .max_connections(1)
                .connect_with(options)
                .await
                .unwrap();

            sqlx::query("INSERT INTO demo (code) VALUES (?)")
                .bind(code)
                .execute(&pool)
                .await
                .unwrap();
        }
    }

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!("{}_{}_{}", prefix, std::process::id(), now));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn backup_file_name(path: &Path) -> String {
        path.file_name().unwrap().to_string_lossy().into_owned()
    }

    fn fake_backup_outcome(file_name: &str) -> BackupOutcome {
        BackupOutcome {
            path: PathBuf::from(format!("/tmp/{file_name}")),
            prune: BackupPruneOutcome::default(),
        }
    }

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

    #[test]
    fn matches_only_managed_backup_files() {
        assert_eq!(BackupReason::Settings.as_str(), "settings");
        assert_eq!(BackupReason::Checkout.as_str(), "checkout");
        assert_eq!(BackupReason::GroupCheckout.as_str(), "group_checkout");
        assert_eq!(BackupReason::NightAudit.as_str(), "night_audit");
        assert_eq!(BackupReason::AppExit.as_str(), "app_exit");
        assert_eq!(BackupReason::Manual.as_str(), "manual");

        assert!(is_managed_backup_file(
            "capyinn_backup_settings_20260418_231500.db"
        ));
        assert!(is_managed_backup_file(
            "capyinn_backup_app_exit_20260419_000102.db"
        ));
        assert!(is_managed_backup_file(
            "capyinn_backup_checkout_20260418_231500-1.db"
        ));
        assert!(!is_managed_backup_file(
            "capyinn_backup_unknown_20260418_231500.db"
        ));
        assert!(!is_managed_backup_file(
            "capyinn_backup_manual_20261340_999999.db"
        ));
        assert!(!is_managed_backup_file(
            "capyinn_backup_checkout_20260418_231500.db.tmp"
        ));
        assert!(!is_managed_backup_file(
            "capyinn_backup_checkout_20260418_231500-abc.db"
        ));
        assert!(!is_managed_backup_file("notes.db"));
    }

    #[tokio::test]
    async fn run_backup_once_creates_standalone_snapshot_db() {
        let fixture = BackupFixture::new().await;
        fixture.insert_demo_row("guest-001").await;

        let outcome = run_backup_once(
            &fixture.db_path,
            &fixture.runtime_root,
            BackupReason::Manual,
        )
        .await
        .expect("backup should succeed");

        assert!(outcome.path.exists());

        let options = SqliteConnectOptions::new()
            .filename(&outcome.path)
            .create_if_missing(false);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();

        let copied: (String,) = sqlx::query_as("SELECT code FROM demo LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();

        assert_eq!(copied.0, "guest-001");
    }

    #[test]
    fn reserve_backup_path_uses_atomic_lock_files_for_collisions() {
        let temp = make_temp_dir("backup-reserve");
        let backup_dir = temp.join("backups");
        fs::create_dir_all(&backup_dir).unwrap();

        let collision_timestamp = NaiveDate::from_ymd_opt(2026, 4, 18)
            .unwrap()
            .and_hms_opt(23, 15, 0)
            .unwrap();
        let first_reservation =
            BackupReservation::acquire(&backup_dir, BackupReason::Manual, collision_timestamp)
                .unwrap();
        assert_eq!(
            backup_file_name(&first_reservation.final_path),
            "capyinn_backup_manual_20260418_231500.db"
        );
        assert!(backup_dir
            .join("capyinn_backup_manual_20260418_231500.db.lock")
            .exists());

        fs::write(&first_reservation.final_path, b"completed").unwrap();
        drop(first_reservation);

        let second_reservation =
            BackupReservation::acquire(&backup_dir, BackupReason::Manual, collision_timestamp)
                .unwrap();
        assert_eq!(
            backup_file_name(&second_reservation.final_path),
            "capyinn_backup_manual_20260418_231500-1.db"
        );

        let later_timestamp = NaiveDate::from_ymd_opt(2026, 4, 18)
            .unwrap()
            .and_hms_opt(23, 16, 0)
            .unwrap();
        let later_reserved =
            BackupReservation::acquire(&backup_dir, BackupReason::Manual, later_timestamp).unwrap();
        assert_eq!(
            backup_file_name(&later_reserved.final_path),
            "capyinn_backup_manual_20260418_231600.db"
        );

        assert!(is_managed_backup_file(&backup_file_name(
            &second_reservation.final_path
        )));
        assert!(is_managed_backup_file(&backup_file_name(
            &later_reserved.final_path
        )));
    }

    #[tokio::test]
    async fn run_backup_once_serializes_concurrent_backups() {
        let fixture = BackupFixture::new().await;
        fixture.insert_demo_row("guest-001").await;

        let timestamp = NaiveDate::from_ymd_opt(2026, 4, 18)
            .unwrap()
            .and_hms_opt(23, 15, 0)
            .unwrap();

        let first = run_backup_once_at(
            &fixture.db_path,
            &fixture.runtime_root,
            BackupReason::Manual,
            timestamp,
            Some(std::time::Duration::from_millis(50)),
        );
        let second = run_backup_once_at(
            &fixture.db_path,
            &fixture.runtime_root,
            BackupReason::Manual,
            timestamp,
            None,
        );

        let (first, second) = tokio::join!(first, second);
        let first = first.expect("first backup should succeed");
        let second = second.expect("second backup should succeed");

        let mut managed_files = fs::read_dir(fixture.runtime_root.join("backups"))
            .unwrap()
            .map(|entry| backup_file_name(&entry.unwrap().path()))
            .filter(|name| is_managed_backup_file(name))
            .collect::<Vec<_>>();
        managed_files.sort();

        let mut expected = vec![
            backup_file_name(&first.path),
            backup_file_name(&second.path),
        ];
        expected.sort();

        assert_eq!(managed_files, expected);
        assert_ne!(
            backup_file_name(&first.path),
            backup_file_name(&second.path)
        );
        assert!(backup_file_name(&first.path).starts_with("capyinn_backup_manual_20260418_231500"));
        assert!(backup_file_name(&second.path).starts_with("capyinn_backup_manual_20260418_231500"));
    }

    #[test]
    fn prune_old_backups_keeps_newest_thirty_files() {
        let temp = make_temp_dir("backup-prune");
        let backup_dir = temp.join("backups");
        fs::create_dir_all(&backup_dir).unwrap();

        for index in 0..32 {
            let filename = format!("capyinn_backup_manual_20260418_{index:06}.db");
            fs::write(backup_dir.join(filename), b"db").unwrap();
        }
        fs::write(backup_dir.join("notes.db"), b"keep").unwrap();

        let outcome = prune_old_backups(&backup_dir, 30);

        let expected_kept = (2..32)
            .rev()
            .map(|index| format!("capyinn_backup_manual_20260418_{index:06}.db"))
            .collect::<Vec<_>>();
        let expected_removed = vec![
            "capyinn_backup_manual_20260418_000001.db".to_string(),
            "capyinn_backup_manual_20260418_000000.db".to_string(),
        ];

        let kept = outcome
            .kept_files
            .iter()
            .map(|path| backup_file_name(path))
            .collect::<Vec<_>>();
        let removed = outcome
            .removed_files
            .iter()
            .map(|path| backup_file_name(path))
            .collect::<Vec<_>>();

        assert_eq!(kept, expected_kept);
        assert_eq!(removed, expected_removed);
        assert!(outcome.error.is_none());
        assert!(backup_dir.join("notes.db").exists());
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
            (
                "completed",
                "manual",
                1,
                Some("/tmp/first.db".to_string()),
            ),
            ("started", "manual", 1, None),
            (
                "completed",
                "manual",
                0,
                Some("/tmp/second.db".to_string()),
            ),
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
            (
                "completed",
                "manual",
                0,
                Some("/tmp/manual.db".to_string()),
            ),
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
