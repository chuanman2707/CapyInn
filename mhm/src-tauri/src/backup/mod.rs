mod coordinator;
mod events;
mod runner;
mod storage;
#[cfg(test)]
mod test_support;
mod types;

pub use coordinator::{drain_and_backup_on_exit, request_backup, BackupCoordinator};
pub(crate) use events::emit_backup_status;
pub use events::BackupStatusPayload;
pub use runner::run_backup_once;
#[allow(unused_imports)]
pub use storage::{build_backup_filename, is_managed_backup_file, prune_old_backups};
#[allow(unused_imports)]
pub use types::BackupRequestErrorKind;
#[allow(unused_imports)]
pub use types::{
    log_backup_request_error, BackupError, BackupOutcome, BackupPruneOutcome, BackupReason,
    BackupRequestError,
};

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

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
}
