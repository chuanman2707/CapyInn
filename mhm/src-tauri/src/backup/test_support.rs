#![cfg(test)]

use crate::backup::{BackupOutcome, BackupPruneOutcome};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static TEMP_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) struct BackupFixture {
    pub(crate) db_path: PathBuf,
    pub(crate) runtime_root: PathBuf,
    _guard: TempDirGuard,
}

pub(crate) struct TempDirGuard {
    path: PathBuf,
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

impl BackupFixture {
    pub(crate) async fn new() -> Self {
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

    pub(crate) async fn insert_demo_row(&self, code: &str) {
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

pub(crate) fn make_temp_dir(prefix: &str) -> PathBuf {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let unique = TEMP_DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let path = env::temp_dir().join(format!(
        "{}_{}_{}_{}",
        prefix,
        std::process::id(),
        now,
        unique
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

pub(crate) fn backup_file_name(path: &Path) -> String {
    path.file_name().unwrap().to_string_lossy().into_owned()
}

pub(crate) fn fake_backup_outcome(file_name: &str) -> BackupOutcome {
    BackupOutcome {
        path: PathBuf::from(format!("/tmp/{file_name}")),
        prune: BackupPruneOutcome::default(),
    }
}
