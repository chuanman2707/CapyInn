use chrono::NaiveDateTime;
use chrono::Utc;
use std::{
    fs,
    io,
    path::{Path, PathBuf},
};

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
    pub pruned_count: usize,
}

#[allow(dead_code)]
#[derive(Debug)]
pub enum BackupError {
    Io(io::Error),
    Sqlx(sqlx::Error),
}

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

pub async fn run_backup_once(
    db_path: &Path,
    runtime_root: &Path,
    reason: BackupReason,
) -> Result<BackupOutcome, BackupError> {
    fs::create_dir_all(runtime_root.join("backups"))?;
    let backup_dir = runtime_root.join("backups");
    let timestamp = Utc::now().naive_utc();
    let final_name = build_backup_filename(reason, timestamp);
    let final_path = backup_dir.join(&final_name);
    let temp_path = backup_dir.join(format!("{final_name}.tmp"));

    if temp_path.exists() {
        fs::remove_file(&temp_path)?;
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

    let pruned_count = prune_old_backups(&backup_dir, 30)?;

    Ok(BackupOutcome {
        path: final_path,
        pruned_count,
    })
}

pub fn prune_old_backups(backup_dir: &Path, keep: usize) -> Result<usize, BackupError> {
    if !backup_dir.exists() {
        return Ok(0);
    }

    let mut backups = Vec::new();
    for entry in fs::read_dir(backup_dir)? {
        let entry = entry?;
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy().into_owned();
        if !is_managed_backup_file(&file_name) {
            continue;
        }

        let Some((timestamp, _reason)) = parse_backup_filename(&file_name) else {
            continue;
        };

        backups.push((timestamp, file_name, entry.path()));
    }

    backups.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| right.1.cmp(&left.1)));

    let mut removed = 0;
    for (_, _, path) in backups.into_iter().skip(keep) {
        fs::remove_file(path)?;
        removed += 1;
    }

    Ok(removed)
}

fn parse_backup_filename(name: &str) -> Option<(NaiveDateTime, BackupReason)> {
    let stem = name.strip_suffix(".db")?;
    let rest = stem.strip_prefix("capyinn_backup_")?;
    let parts = rest.split('_').collect::<Vec<_>>();
    if parts.len() < 3 {
        return None;
    }

    let reason = parse_backup_reason(&parts[..parts.len() - 2].join("_"))?;
    let date = parts[parts.len() - 2];
    let time = parts[parts.len() - 1];
    let timestamp = NaiveDateTime::parse_from_str(&format!("{}_{}", date, time), "%Y%m%d_%H%M%S")
        .ok()?;

    Some((timestamp, reason))
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
        time::{SystemTime, UNIX_EPOCH},
    };

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

        assert!(is_managed_backup_file("capyinn_backup_settings_20260418_231500.db"));
        assert!(is_managed_backup_file("capyinn_backup_app_exit_20260419_000102.db"));
        assert!(!is_managed_backup_file("capyinn_backup_unknown_20260418_231500.db"));
        assert!(!is_managed_backup_file("capyinn_backup_manual_20261340_999999.db"));
        assert!(!is_managed_backup_file("capyinn_backup_checkout_20260418_231500.db.tmp"));
        assert!(!is_managed_backup_file("notes.db"));
    }

    #[tokio::test]
    async fn run_backup_once_creates_standalone_snapshot_db() {
        let fixture = BackupFixture::new().await;
        fixture.insert_demo_row("guest-001").await;

        let outcome =
            run_backup_once(&fixture.db_path, &fixture.runtime_root, BackupReason::Manual)
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
    fn prune_old_backups_keeps_newest_thirty_files() {
        let temp = make_temp_dir("backup-prune");
        let backup_dir = temp.join("backups");
        fs::create_dir_all(&backup_dir).unwrap();

        for index in 0..32 {
            let filename = format!("capyinn_backup_manual_20260418_{index:06}.db");
            fs::write(backup_dir.join(filename), b"db").unwrap();
        }
        fs::write(backup_dir.join("notes.db"), b"keep").unwrap();

        let removed = prune_old_backups(&backup_dir, 30).unwrap();

        assert_eq!(removed, 2);
        assert!(backup_dir.join("notes.db").exists());
    }
}
