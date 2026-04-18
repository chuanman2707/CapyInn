use crate::backup::{
    storage::{prune_old_backups, sqlite_string_literal, sync_directory, BackupReservation},
    BackupError, BackupOutcome, BackupReason,
};
use chrono::{NaiveDateTime, Utc};
use std::{fs, path::Path, time::Duration};

pub async fn run_backup_once(
    db_path: &Path,
    runtime_root: &Path,
    reason: BackupReason,
) -> Result<BackupOutcome, BackupError> {
    run_backup_once_at(db_path, runtime_root, reason, Utc::now().naive_utc(), None).await
}

pub(crate) async fn run_backup_once_at(
    db_path: &Path,
    runtime_root: &Path,
    reason: BackupReason,
    timestamp: NaiveDateTime,
    hold_for: Option<Duration>,
) -> Result<BackupOutcome, BackupError> {
    let backup_dir = runtime_root.join("backups");
    fs::create_dir_all(&backup_dir)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::{
        is_managed_backup_file,
        test_support::{backup_file_name, BackupFixture},
    };
    use chrono::NaiveDate;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    use std::fs;

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
            Some(Duration::from_millis(50)),
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
        assert!(backup_file_name(&second.path)
            .starts_with("capyinn_backup_manual_20260418_231500"));
    }
}
