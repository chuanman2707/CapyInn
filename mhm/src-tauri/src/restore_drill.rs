use chrono::{NaiveDateTime, Utc};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    Sqlite,
};
use std::{
    collections::HashSet,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const BACKUP_PREFIX: &str = "capyinn_backup_";
const BACKUP_SUFFIX: &str = ".db";
const REPORT_PREFIX: &str = "restore-drill";
const REPORT_SUFFIX: &str = ".md";
const REQUIRED_TABLES: [&str; 6] = [
    "settings",
    "rooms",
    "guests",
    "bookings",
    "audit_logs",
    "schema_version",
];
const BACKUP_REASONS: [&str; 7] = [
    "settings",
    "checkout",
    "group_checkout",
    "night_audit",
    "app_exit",
    "manual",
    "scheduled",
];

static TEMP_WORKSPACE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Default)]
pub struct RestoreDrillOptions {
    pub runtime_root: Option<PathBuf>,
    pub backup_path: Option<PathBuf>,
    pub now: Option<NaiveDateTime>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RestoreDrillStatus {
    Pass,
    Fail,
}

impl RestoreDrillStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
        }
    }
}

#[derive(Debug)]
pub struct RestoreDrillRun {
    pub status: RestoreDrillStatus,
    pub report_path: Option<PathBuf>,
    pub backup_path: Option<PathBuf>,
    pub copied_path: Option<PathBuf>,
    pub checks: Vec<RestoreDrillCheck>,
    pub message: String,
}

impl RestoreDrillRun {
    pub fn exit_code(&self) -> i32 {
        match self.status {
            RestoreDrillStatus::Pass => 0,
            RestoreDrillStatus::Fail => 1,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RestoreDrillCheck {
    name: String,
    status: CheckStatus,
    detail: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CheckStatus {
    Pass,
    Fail,
    Warn,
}

impl CheckStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Warn => "WARN",
        }
    }
}

#[derive(Clone, Debug)]
struct ManagedBackup {
    path: PathBuf,
    timestamp: NaiveDateTime,
    collision_index: u64,
    file_name: String,
}

#[derive(Debug)]
struct TempWorkspace {
    path: PathBuf,
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

pub async fn run_restore_drill(options: RestoreDrillOptions) -> RestoreDrillRun {
    let now = options.now.unwrap_or_else(drill_timestamp_now);
    let Some(runtime_root) = options
        .runtime_root
        .clone()
        .or_else(crate::app_identity::runtime_root_opt)
    else {
        return RestoreDrillRun {
            status: RestoreDrillStatus::Fail,
            report_path: None,
            backup_path: options.backup_path,
            copied_path: None,
            checks: vec![fail_check(
                "Runtime root",
                "cannot find CapyInn runtime root",
            )],
            message: "cannot find CapyInn runtime root".to_string(),
        };
    };

    let mut checks = Vec::new();
    let selected = match select_backup(&runtime_root, options.backup_path.clone()) {
        Ok(selected) => selected,
        Err(error) => {
            checks.push(fail_check("Backup selection", error.clone()));
            return finish_run(FinishRunInput {
                runtime_root,
                now,
                backup_path: options.backup_path,
                copied_path: None,
                checks,
                failure: Some(error),
            });
        }
    };

    if is_live_database_path(&selected.path, &runtime_root) {
        let failure = "refusing to validate the live capyinn.db database".to_string();
        checks.push(fail_check("Safety check", failure.clone()));
        return finish_run(FinishRunInput {
            runtime_root,
            now,
            backup_path: Some(selected.path),
            copied_path: None,
            checks,
            failure: Some(failure),
        });
    }

    checks.push(pass_check(
        "Backup selection",
        format!("selected {}", selected.path.display()),
    ));

    let (copied_path, failure) = validate_selected_backup(&selected.path, &mut checks, now).await;

    finish_run(FinishRunInput {
        runtime_root,
        now,
        backup_path: Some(selected.path),
        copied_path,
        checks,
        failure,
    })
}

struct FinishRunInput {
    runtime_root: PathBuf,
    now: NaiveDateTime,
    backup_path: Option<PathBuf>,
    copied_path: Option<PathBuf>,
    checks: Vec<RestoreDrillCheck>,
    failure: Option<String>,
}

fn finish_run(input: FinishRunInput) -> RestoreDrillRun {
    let status = if input.failure.is_none()
        && input
            .checks
            .iter()
            .all(|check| check.status != CheckStatus::Fail)
    {
        RestoreDrillStatus::Pass
    } else {
        RestoreDrillStatus::Fail
    };
    let message = input.failure.unwrap_or_else(|| match status {
        RestoreDrillStatus::Pass => {
            "Backup restore drill passed. The checked backup can be opened and basic data structures are readable."
                .to_string()
        }
        RestoreDrillStatus::Fail => "Backup restore drill failed.".to_string(),
    });

    let report = render_report(
        status,
        input.now,
        &input.runtime_root,
        input.backup_path.as_deref(),
        input.copied_path.as_deref(),
        &input.checks,
        &message,
    );

    match write_report(&input.runtime_root, input.now, &report) {
        Ok(report_path) => RestoreDrillRun {
            status,
            report_path: Some(report_path),
            backup_path: input.backup_path,
            copied_path: input.copied_path,
            checks: input.checks,
            message,
        },
        Err(error) => RestoreDrillRun {
            status: RestoreDrillStatus::Fail,
            report_path: None,
            backup_path: input.backup_path,
            copied_path: input.copied_path,
            checks: input.checks,
            message: format!("failed to write restore drill report: {error}; result was {message}"),
        },
    }
}

fn select_backup(
    runtime_root: &Path,
    explicit_backup_path: Option<PathBuf>,
) -> Result<ManagedBackup, String> {
    match explicit_backup_path {
        Some(path) => {
            let file_name = path
                .file_name()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_else(|| "selected backup".to_string());
            Ok(ManagedBackup {
                path,
                timestamp: drill_timestamp_now(),
                collision_index: 0,
                file_name,
            })
        }
        None => select_newest_managed_backup(runtime_root),
    }
}

fn select_newest_managed_backup(runtime_root: &Path) -> Result<ManagedBackup, String> {
    let backup_dir = runtime_root.join("backups");
    let entries =
        fs::read_dir(&backup_dir).map_err(|error| backup_dir_read_error(&backup_dir, error))?;

    let mut newest: Option<ManagedBackup> = None;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("cannot read backup directory entry: {error}"))?;
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let Some(metadata) = parse_managed_backup_file_name(&file_name) else {
            continue;
        };

        let candidate = ManagedBackup {
            path: entry.path(),
            timestamp: metadata.timestamp,
            collision_index: metadata.collision_index,
            file_name,
        };

        if newest
            .as_ref()
            .map(|current| compare_backup(candidate_key(&candidate), candidate_key(current)))
            .unwrap_or(true)
        {
            newest = Some(candidate);
        }
    }

    newest.ok_or_else(|| "no managed CapyInn backup found".to_string())
}

fn compare_backup(left: (NaiveDateTime, u64, &str), right: (NaiveDateTime, u64, &str)) -> bool {
    left.0
        .cmp(&right.0)
        .then_with(|| left.1.cmp(&right.1))
        .then_with(|| left.2.cmp(right.2))
        .is_gt()
}

fn candidate_key(candidate: &ManagedBackup) -> (NaiveDateTime, u64, &str) {
    (
        candidate.timestamp,
        candidate.collision_index,
        &candidate.file_name,
    )
}

fn backup_dir_read_error(backup_dir: &Path, error: io::Error) -> String {
    if error.kind() == io::ErrorKind::NotFound {
        format!("no backup directory found: {}", backup_dir.display())
    } else {
        format!(
            "cannot read backup directory {}: {error}",
            backup_dir.display()
        )
    }
}

struct ParsedBackupName {
    timestamp: NaiveDateTime,
    collision_index: u64,
}

fn parse_managed_backup_file_name(name: &str) -> Option<ParsedBackupName> {
    let stem = name.strip_suffix(BACKUP_SUFFIX)?;
    let rest = stem.strip_prefix(BACKUP_PREFIX)?;
    let mut parts = rest.rsplitn(3, '_');
    let time_or_suffix = parts.next()?;
    let date = parts.next()?;
    let reason = parts.next()?;

    if !BACKUP_REASONS.contains(&reason) {
        return None;
    }

    let (time, collision_index) = match time_or_suffix.split_once('-') {
        Some((time, suffix)) => (time, suffix.parse().ok()?),
        None => (time_or_suffix, 0),
    };
    let timestamp =
        NaiveDateTime::parse_from_str(&format!("{date}_{time}"), "%Y%m%d_%H%M%S").ok()?;

    Some(ParsedBackupName {
        timestamp,
        collision_index,
    })
}

async fn validate_selected_backup(
    backup_path: &Path,
    checks: &mut Vec<RestoreDrillCheck>,
    now: NaiveDateTime,
) -> (Option<PathBuf>, Option<String>) {
    match fs::metadata(backup_path) {
        Ok(metadata) if metadata.is_file() => {
            checks.push(pass_check(
                "Backup file exists",
                backup_path.display().to_string(),
            ));
            if metadata.len() > 0 {
                checks.push(pass_check(
                    "Backup file is not empty",
                    format!("{} bytes", metadata.len()),
                ));
            } else {
                let failure = "backup file is empty".to_string();
                checks.push(fail_check("Backup file is not empty", failure.clone()));
                return (None, Some(failure));
            }
        }
        Ok(_) => {
            let failure = format!(
                "backup path is not a regular file: {}",
                backup_path.display()
            );
            checks.push(fail_check("Backup file exists", failure.clone()));
            return (None, Some(failure));
        }
        Err(error) => {
            let failure = format!("cannot read backup file {}: {error}", backup_path.display());
            checks.push(fail_check("Backup file exists", failure.clone()));
            return (None, Some(failure));
        }
    }

    let workspace = match create_temp_workspace(now) {
        Ok(workspace) => workspace,
        Err(error) => {
            let failure = format!("cannot create temporary drill workspace: {error}");
            checks.push(fail_check("Temporary workspace", failure.clone()));
            return (None, Some(failure));
        }
    };

    let copy_name = backup_path
        .file_name()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("backup.db"));
    let copied_path = workspace.path.join(copy_name);

    match fs::copy(backup_path, &copied_path) {
        Ok(_) => checks.push(pass_check(
            "Backup copied to temp workspace",
            copied_path.display().to_string(),
        )),
        Err(error) => {
            let failure = format!(
                "cannot copy backup from {} to {}: {error}",
                backup_path.display(),
                copied_path.display()
            );
            checks.push(fail_check(
                "Backup copied to temp workspace",
                failure.clone(),
            ));
            return (Some(copied_path), Some(failure));
        }
    }

    let validation_failure = validate_copied_database(&copied_path, checks).await;
    (Some(copied_path), validation_failure)
}

async fn validate_copied_database(
    copied_path: &Path,
    checks: &mut Vec<RestoreDrillCheck>,
) -> Option<String> {
    let options = SqliteConnectOptions::new()
        .filename(copied_path)
        .read_only(true)
        .create_if_missing(false);
    let pool = match SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
    {
        Ok(pool) => {
            checks.push(pass_check(
                "SQLite opens read-only",
                copied_path.display().to_string(),
            ));
            pool
        }
        Err(error) => {
            let failure = format!("SQLite read-only open failed: {error}");
            checks.push(fail_check("SQLite opens read-only", failure.clone()));
            return Some(failure);
        }
    };

    let integrity_rows = match sqlx::query_scalar::<Sqlite, String>("PRAGMA integrity_check;")
        .fetch_all(&pool)
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            let failure = format!("SQLite integrity check failed: {error}");
            checks.push(fail_check("Integrity check", failure.clone()));
            return Some(failure);
        }
    };

    if integrity_rows.len() == 1 && integrity_rows[0].eq_ignore_ascii_case("ok") {
        checks.push(pass_check("Integrity check", "ok"));
    } else {
        let failure = "SQLite integrity check failed".to_string();
        checks.push(fail_check(
            "Integrity check",
            format!("{failure}: {} result rows", integrity_rows.len()),
        ));
        return Some(failure);
    }

    let foreign_key_rows = match sqlx::query("PRAGMA foreign_key_check;")
        .fetch_all(&pool)
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            let failure = format!("foreign key check failed: {error}");
            checks.push(fail_check("Foreign key check", failure.clone()));
            return Some(failure);
        }
    };

    if foreign_key_rows.is_empty() {
        checks.push(pass_check("Foreign key check", "0 violations"));
    } else {
        let failure = format!(
            "foreign key check failed: {} violations",
            foreign_key_rows.len()
        );
        checks.push(fail_check("Foreign key check", failure.clone()));
        return Some(failure);
    }

    let table_names = match sqlx::query_scalar::<Sqlite, String>(
        "SELECT name FROM sqlite_master WHERE type = 'table';",
    )
    .fetch_all(&pool)
    .await
    {
        Ok(names) => names.into_iter().collect::<HashSet<_>>(),
        Err(error) => {
            let failure = format!("required table lookup failed: {error}");
            checks.push(fail_check("Required tables", failure.clone()));
            return Some(failure);
        }
    };

    let missing_tables = REQUIRED_TABLES
        .iter()
        .copied()
        .filter(|table| !table_names.contains(*table))
        .collect::<Vec<_>>();
    if !missing_tables.is_empty() {
        let failure = format!("missing tables: {}", missing_tables.join(", "));
        checks.push(fail_check("Required tables", failure.clone()));
        return Some(failure);
    }
    checks.push(pass_check("Required tables", REQUIRED_TABLES.join(", ")));

    let mut counts = Vec::new();
    for table in REQUIRED_TABLES {
        let sql = format!("SELECT COUNT(*) FROM \"{table}\";");
        match sqlx::query_scalar::<Sqlite, i64>(&sql)
            .fetch_one(&pool)
            .await
        {
            Ok(count) => counts.push(format!("{table}={count}")),
            Err(error) => {
                let failure = format!("core table {table} cannot be read: {error}");
                checks.push(fail_check("Core tables readable", failure.clone()));
                return Some(failure);
            }
        }
    }
    checks.push(pass_check("Core tables readable", counts.join(", ")));

    match sqlx::query_scalar::<Sqlite, i64>("SELECT COUNT(*) FROM schema_version;")
        .fetch_one(&pool)
        .await
    {
        Ok(count) => {
            checks.push(pass_check(
                "Schema version readable",
                format!("{count} rows"),
            ));
            None
        }
        Err(error) => {
            let failure = format!("schema_version cannot be read: {error}");
            checks.push(fail_check("Schema version readable", failure.clone()));
            Some(failure)
        }
    }
}

fn create_temp_workspace(now: NaiveDateTime) -> io::Result<TempWorkspace> {
    let timestamp = now.format("%Y%m%d_%H%M%S");
    for _ in 0..1000 {
        let unique = TEMP_WORKSPACE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "capyinn-restore-drill-{timestamp}-{}-{unique}",
            std::process::id()
        ));
        match fs::create_dir(&path) {
            Ok(()) => return Ok(TempWorkspace { path }),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "cannot allocate unique temporary drill workspace",
    ))
}

fn write_report(runtime_root: &Path, now: NaiveDateTime, report: &str) -> io::Result<PathBuf> {
    let report_dir = runtime_root.join("restore-drills");
    fs::create_dir_all(&report_dir)?;
    let timestamp = now.format("%Y%m%d_%H%M%S");

    for collision_index in 0..1000 {
        let file_name = if collision_index == 0 {
            format!("{REPORT_PREFIX}-{timestamp}{REPORT_SUFFIX}")
        } else {
            format!("{REPORT_PREFIX}-{timestamp}-{collision_index}{REPORT_SUFFIX}")
        };
        let report_path = report_dir.join(file_name);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&report_path)
        {
            Ok(mut file) => {
                file.write_all(report.as_bytes())?;
                file.sync_all()?;
                return Ok(report_path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "cannot allocate unique restore drill report path",
    ))
}

fn render_report(
    status: RestoreDrillStatus,
    now: NaiveDateTime,
    runtime_root: &Path,
    backup_path: Option<&Path>,
    copied_path: Option<&Path>,
    checks: &[RestoreDrillCheck],
    message: &str,
) -> String {
    let mut report = String::new();
    report.push_str("# Restore Drill Report\n\n");
    report.push_str(&format!("Status: {}\n", status.as_str()));
    report.push_str(&format!(
        "Checked at: {}\n",
        now.format("%Y-%m-%d %H:%M:%S")
    ));
    report.push_str(&format!("Runtime root: {}\n", runtime_root.display()));
    report.push_str(&format!(
        "Backup checked: {}\n",
        backup_path
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "none".to_string())
    ));
    report.push_str(&format!(
        "Backup copied to: {}\n\n",
        copied_path
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "none".to_string())
    ));
    report.push_str("## Checks\n\n");
    report.push_str("| Check | Status | Detail |\n");
    report.push_str("| --- | --- | --- |\n");
    for check in checks {
        report.push_str(&format!(
            "| {} | {} | {} |\n",
            escape_markdown_table(&check.name),
            check.status.as_str(),
            escape_markdown_table(&check.detail)
        ));
    }
    report.push_str("\n## Result\n\n");
    report.push_str(message);
    report.push('\n');
    if status == RestoreDrillStatus::Fail {
        report.push_str("\n## Failure\n\n");
        report.push_str(message);
        report.push('\n');
    }
    report
}

fn escape_markdown_table(value: &str) -> String {
    value
        .replace(['\n', '\r'], " ")
        .replace('|', "\\|")
}

fn pass_check(name: impl Into<String>, detail: impl Into<String>) -> RestoreDrillCheck {
    RestoreDrillCheck {
        name: name.into(),
        status: CheckStatus::Pass,
        detail: detail.into(),
    }
}

fn fail_check(name: impl Into<String>, detail: impl Into<String>) -> RestoreDrillCheck {
    RestoreDrillCheck {
        name: name.into(),
        status: CheckStatus::Fail,
        detail: detail.into(),
    }
}

#[allow(dead_code)]
fn warn_check(name: impl Into<String>, detail: impl Into<String>) -> RestoreDrillCheck {
    RestoreDrillCheck {
        name: name.into(),
        status: CheckStatus::Warn,
        detail: detail.into(),
    }
}

fn drill_timestamp_now() -> NaiveDateTime {
    crate::runtime_config::test_now()
        .map(|value| value.naive_local())
        .unwrap_or_else(|| Utc::now().naive_utc())
}

fn is_live_database_path(path: &Path, runtime_root: &Path) -> bool {
    let live_database = runtime_root.join(crate::app_identity::APP_DATABASE_FILENAME);
    comparable_path(path) == comparable_path(&live_database)
}

fn comparable_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn fixed_time(
        year: i32,
        month: u32,
        day: u32,
        hour: u32,
        minute: u32,
        second: u32,
    ) -> chrono::NaiveDateTime {
        NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .and_hms_opt(hour, minute, second)
            .unwrap()
    }

    fn make_temp_dir(prefix: &str) -> TempDir {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let unique = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "{}_{}_{}_{}",
            prefix,
            std::process::id(),
            now,
            unique
        ));
        fs::create_dir_all(&path).unwrap();
        TempDir { path }
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    async fn create_valid_backup(path: &Path) {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();

        for statement in [
            "CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            "CREATE TABLE rooms (id TEXT PRIMARY KEY, name TEXT NOT NULL)",
            "CREATE TABLE guests (id TEXT PRIMARY KEY, full_name TEXT NOT NULL)",
            "CREATE TABLE bookings (id TEXT PRIMARY KEY, room_id TEXT, guest_id TEXT)",
            "CREATE TABLE audit_logs (id TEXT PRIMARY KEY, action TEXT NOT NULL)",
            "CREATE TABLE schema_version (version INTEGER NOT NULL DEFAULT 0)",
            "INSERT INTO guests (id, full_name) VALUES ('guest-1', 'Alice Example')",
            "INSERT INTO schema_version (version) VALUES (10)",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
    }

    async fn create_incomplete_backup(path: &Path) {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();

        sqlx::query("CREATE TABLE settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
    }

    #[test]
    fn select_newest_managed_backup_ignores_unmanaged_files() {
        let temp = make_temp_dir("restore-drill-select");
        let backup_dir = temp.path().join("backups");
        fs::create_dir_all(&backup_dir).unwrap();
        fs::write(
            backup_dir.join("capyinn_backup_manual_20260426_090000.db"),
            b"old",
        )
        .unwrap();
        fs::write(
            backup_dir.join("capyinn_backup_scheduled_20260426_110000.db"),
            b"new",
        )
        .unwrap();
        fs::write(backup_dir.join("notes.db"), b"ignore").unwrap();

        let selected = select_newest_managed_backup(temp.path()).unwrap();

        assert_eq!(
            selected.path.file_name().unwrap().to_string_lossy(),
            "capyinn_backup_scheduled_20260426_110000.db"
        );
    }

    #[test]
    fn select_newest_managed_backup_prefers_highest_collision_suffix() {
        let temp = make_temp_dir("restore-drill-collision");
        let backup_dir = temp.path().join("backups");
        fs::create_dir_all(&backup_dir).unwrap();
        fs::write(
            backup_dir.join("capyinn_backup_scheduled_20260426_110000.db"),
            b"base",
        )
        .unwrap();
        fs::write(
            backup_dir.join("capyinn_backup_scheduled_20260426_110000-2.db"),
            b"latest",
        )
        .unwrap();
        fs::write(
            backup_dir.join("capyinn_backup_scheduled_20260426_110000-1.db"),
            b"middle",
        )
        .unwrap();

        let selected = select_newest_managed_backup(temp.path()).unwrap();

        assert_eq!(
            selected.path.file_name().unwrap().to_string_lossy(),
            "capyinn_backup_scheduled_20260426_110000-2.db"
        );
    }

    #[tokio::test]
    async fn drill_writes_pass_report_for_valid_backup_copy() {
        let temp = make_temp_dir("restore-drill-pass");
        let backup_dir = temp.path().join("backups");
        fs::create_dir_all(&backup_dir).unwrap();
        create_valid_backup(&backup_dir.join("capyinn_backup_scheduled_20260426_110000.db")).await;

        let result = run_restore_drill(RestoreDrillOptions {
            runtime_root: Some(temp.path().to_path_buf()),
            backup_path: None,
            now: Some(fixed_time(2026, 4, 26, 11, 30, 0)),
        })
        .await;

        assert_eq!(result.status, RestoreDrillStatus::Pass);
        let report_path = result.report_path.expect("pass run should write report");
        let report = fs::read_to_string(report_path).unwrap();
        assert!(report.contains("Status: PASS"));
        assert!(report.contains("capyinn_backup_scheduled_20260426_110000.db"));
        assert!(!report.contains("Alice Example"));
    }

    #[tokio::test]
    async fn drill_writes_fail_report_when_no_backup_exists() {
        let temp = make_temp_dir("restore-drill-fail");

        let result = run_restore_drill(RestoreDrillOptions {
            runtime_root: Some(temp.path().to_path_buf()),
            backup_path: None,
            now: Some(fixed_time(2026, 4, 26, 11, 30, 0)),
        })
        .await;

        assert_eq!(result.status, RestoreDrillStatus::Fail);
        let report_path = result.report_path.expect("fail run should write report");
        let report = fs::read_to_string(report_path).unwrap();
        assert!(report.contains("Status: FAIL"));
        assert!(report.contains("no backup directory found"));
    }

    #[tokio::test]
    async fn drill_writes_fail_report_when_required_tables_are_missing() {
        let temp = make_temp_dir("restore-drill-missing-tables");
        let backup_dir = temp.path().join("backups");
        fs::create_dir_all(&backup_dir).unwrap();
        create_incomplete_backup(&backup_dir.join("capyinn_backup_manual_20260426_110000.db"))
            .await;

        let result = run_restore_drill(RestoreDrillOptions {
            runtime_root: Some(temp.path().to_path_buf()),
            backup_path: None,
            now: Some(fixed_time(2026, 4, 26, 11, 30, 0)),
        })
        .await;

        assert_eq!(result.status, RestoreDrillStatus::Fail);
        let report_path = result.report_path.expect("fail run should write report");
        let report = fs::read_to_string(report_path).unwrap();
        assert!(report.contains("Status: FAIL"));
        assert!(report.contains("missing tables"));
        assert!(!report.contains("Alice Example"));
    }

    #[tokio::test]
    async fn drill_refuses_to_validate_live_database_path() {
        let temp = make_temp_dir("restore-drill-live-db");
        let live_db = temp.path().join(crate::app_identity::APP_DATABASE_FILENAME);
        fs::write(&live_db, b"not used").unwrap();

        let result = run_restore_drill(RestoreDrillOptions {
            runtime_root: Some(temp.path().to_path_buf()),
            backup_path: Some(live_db),
            now: Some(fixed_time(2026, 4, 26, 11, 30, 0)),
        })
        .await;

        assert_eq!(result.status, RestoreDrillStatus::Fail);
        let report_path = result.report_path.expect("fail run should write report");
        let report = fs::read_to_string(report_path).unwrap();
        assert!(report.contains("Status: FAIL"));
        assert!(report.contains("refusing to validate the live capyinn.db database"));
    }

    #[tokio::test]
    async fn drill_does_not_overwrite_existing_report_for_same_second() {
        let temp = make_temp_dir("restore-drill-report-collision");
        let report_dir = temp.path().join("restore-drills");
        fs::create_dir_all(&report_dir).unwrap();
        let existing_report = report_dir.join("restore-drill-20260426_113000.md");
        fs::write(&existing_report, "existing report").unwrap();

        let result = run_restore_drill(RestoreDrillOptions {
            runtime_root: Some(temp.path().to_path_buf()),
            backup_path: None,
            now: Some(fixed_time(2026, 4, 26, 11, 30, 0)),
        })
        .await;

        assert_eq!(result.status, RestoreDrillStatus::Fail);
        let report_path = result.report_path.expect("fail run should write report");
        assert_eq!(
            report_path.file_name().unwrap().to_string_lossy(),
            "restore-drill-20260426_113000-1.md"
        );
        assert_eq!(
            fs::read_to_string(existing_report).unwrap(),
            "existing report"
        );
    }
}
