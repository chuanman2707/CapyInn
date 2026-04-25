use super::types::{BackupError, BackupPruneOutcome, BackupReason};
use chrono::{Duration, NaiveDateTime};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
struct BackupMetadata {
    reason: BackupReason,
    timestamp: NaiveDateTime,
    collision_index: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BackupRetentionGroup {
    Manual,
    Automatic,
}

impl BackupRetentionGroup {
    fn for_reason(reason: BackupReason) -> Self {
        match reason {
            BackupReason::Manual => Self::Manual,
            BackupReason::Settings
            | BackupReason::Checkout
            | BackupReason::GroupCheckout
            | BackupReason::NightAudit
            | BackupReason::AppExit => Self::Automatic,
        }
    }

    fn retention_window(self) -> Duration {
        match self {
            Self::Manual => Duration::days(30),
            Self::Automatic => Duration::days(7),
        }
    }
}

impl BackupMetadata {
    fn retention_group(&self) -> BackupRetentionGroup {
        BackupRetentionGroup::for_reason(self.reason)
    }

    fn is_expired_at(&self, now: NaiveDateTime) -> bool {
        now - self.timestamp > self.retention_group().retention_window()
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

pub(crate) struct BackupReservation {
    pub(crate) final_path: PathBuf,
    pub(crate) temp_path: PathBuf,
    lock_path: PathBuf,
    lock_file: Option<fs::File>,
}

impl BackupReservation {
    pub(crate) fn acquire(
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

pub fn prune_old_backups(backup_dir: &Path, now: NaiveDateTime) -> BackupPruneOutcome {
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

    let newest_manual = backups
        .iter()
        .find(|(metadata, _, _)| metadata.retention_group() == BackupRetentionGroup::Manual)
        .map(|(_, _, path)| path.clone());
    let newest_automatic = backups
        .iter()
        .find(|(metadata, _, _)| metadata.retention_group() == BackupRetentionGroup::Automatic)
        .map(|(_, _, path)| path.clone());

    for (metadata, _, path) in backups {
        let is_safety_floor = match metadata.retention_group() {
            BackupRetentionGroup::Manual => newest_manual.as_ref() == Some(&path),
            BackupRetentionGroup::Automatic => newest_automatic.as_ref() == Some(&path),
        };

        if !metadata.is_expired_at(now) || is_safety_floor {
            outcome.kept_files.push(path);
            continue;
        }

        if let Err(error) = fs::remove_file(&path) {
            outcome.error = Some(error.into());
            break;
        }
        outcome.removed_files.push(path);
    }

    outcome
}

pub(crate) fn sqlite_string_literal(path: &Path) -> String {
    let escaped = path.to_string_lossy().replace('\'', "''");
    format!("'{}'", escaped)
}

fn reservation_lock_path(final_path: &Path) -> PathBuf {
    final_path.with_extension("db.lock")
}

#[cfg(unix)]
pub(crate) fn sync_directory(path: &Path) -> io::Result<()> {
    fs::OpenOptions::new().read(true).open(path)?.sync_all()
}

#[cfg(not(unix))]
pub(crate) fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
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
    let reason = parse_backup_reason(reason)?;
    let timestamp =
        NaiveDateTime::parse_from_str(&format!("{}_{}", date, time), "%Y%m%d_%H%M%S").ok()?;

    Some(BackupMetadata {
        reason,
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, NaiveDate, NaiveDateTime};
    use std::{
        env, fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn backup_file_name(path: &Path) -> String {
        path.file_name().unwrap().to_string_lossy().into_owned()
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

    fn write_backup_at(
        backup_dir: &Path,
        reason: BackupReason,
        timestamp: NaiveDateTime,
    ) -> PathBuf {
        let path = backup_dir.join(build_backup_filename(reason, timestamp));
        fs::write(&path, b"db").unwrap();
        path
    }

    fn write_named_backup(backup_dir: &Path, name: &str) -> PathBuf {
        let path = backup_dir.join(name);
        fs::write(&path, b"db").unwrap();
        path
    }

    fn sorted_backup_file_names(paths: &[PathBuf]) -> Vec<String> {
        let mut names = paths
            .iter()
            .map(|path| backup_file_name(path))
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[test]
    fn matches_only_managed_backup_files_including_collision_suffixes() {
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

    #[test]
    fn prune_old_backups_applies_manual_thirty_day_and_auto_seven_day_windows() {
        let temp = make_temp_dir("backup-retention-window");
        let backup_dir = temp.join("backups");
        fs::create_dir_all(&backup_dir).unwrap();

        let now = fixed_time(2026, 4, 25, 8, 0, 0);
        let manual_at_boundary =
            write_backup_at(&backup_dir, BackupReason::Manual, now - Duration::days(30));
        let manual_expired =
            write_backup_at(&backup_dir, BackupReason::Manual, now - Duration::days(31));
        let auto_at_boundary =
            write_backup_at(&backup_dir, BackupReason::Checkout, now - Duration::days(7));
        let auto_expired =
            write_backup_at(&backup_dir, BackupReason::Settings, now - Duration::days(8));
        fs::write(backup_dir.join("notes.db"), b"keep").unwrap();

        let outcome = prune_old_backups(&backup_dir, now);

        assert_eq!(
            sorted_backup_file_names(&outcome.kept_files),
            sorted_backup_file_names(&[manual_at_boundary, auto_at_boundary])
        );
        assert_eq!(
            sorted_backup_file_names(&outcome.removed_files),
            sorted_backup_file_names(&[manual_expired, auto_expired])
        );
        assert!(outcome.error.is_none());
        assert!(backup_dir.join("notes.db").exists());
    }

    #[test]
    fn prune_old_backups_keeps_newest_expired_backup_per_retention_group() {
        let temp = make_temp_dir("backup-retention-floor");
        let backup_dir = temp.join("backups");
        fs::create_dir_all(&backup_dir).unwrap();

        let now = fixed_time(2026, 4, 25, 8, 0, 0);
        let newest_manual =
            write_backup_at(&backup_dir, BackupReason::Manual, now - Duration::days(45));
        let older_manual =
            write_backup_at(&backup_dir, BackupReason::Manual, now - Duration::days(60));
        let newest_auto = write_backup_at(
            &backup_dir,
            BackupReason::NightAudit,
            now - Duration::days(10),
        );
        let older_auto = write_backup_at(
            &backup_dir,
            BackupReason::Checkout,
            now - Duration::days(12),
        );

        let outcome = prune_old_backups(&backup_dir, now);

        assert_eq!(
            sorted_backup_file_names(&outcome.kept_files),
            sorted_backup_file_names(&[newest_manual, newest_auto])
        );
        assert_eq!(
            sorted_backup_file_names(&outcome.removed_files),
            sorted_backup_file_names(&[older_manual, older_auto])
        );
        assert!(outcome.error.is_none());
    }

    #[test]
    fn prune_old_backups_uses_collision_suffix_when_selecting_safety_floor() {
        let temp = make_temp_dir("backup-retention-collision");
        let backup_dir = temp.join("backups");
        fs::create_dir_all(&backup_dir).unwrap();

        let now = fixed_time(2026, 4, 25, 8, 0, 0);
        let base = write_named_backup(&backup_dir, "capyinn_backup_checkout_20260401_080000.db");
        let collision =
            write_named_backup(&backup_dir, "capyinn_backup_checkout_20260401_080000-1.db");

        let outcome = prune_old_backups(&backup_dir, now);

        assert_eq!(
            sorted_backup_file_names(&outcome.kept_files),
            sorted_backup_file_names(&[collision])
        );
        assert_eq!(
            sorted_backup_file_names(&outcome.removed_files),
            sorted_backup_file_names(&[base])
        );
        assert!(outcome.error.is_none());
    }
}
