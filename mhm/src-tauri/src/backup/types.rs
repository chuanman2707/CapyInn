use std::{fmt, io, path::PathBuf};

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

impl fmt::Display for BackupError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "{error}"),
            Self::Sqlx(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for BackupError {}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_reason_wire_values_stay_stable() {
        assert_eq!(BackupReason::Settings.as_str(), "settings");
        assert_eq!(BackupReason::Checkout.as_str(), "checkout");
        assert_eq!(BackupReason::GroupCheckout.as_str(), "group_checkout");
        assert_eq!(BackupReason::NightAudit.as_str(), "night_audit");
        assert_eq!(BackupReason::AppExit.as_str(), "app_exit");
        assert_eq!(BackupReason::Manual.as_str(), "manual");
    }

    #[test]
    fn request_error_kind_preserves_skip_vs_failure_behavior() {
        assert_eq!(
            BackupRequestError::ShutdownInProgress.kind(),
            BackupRequestErrorKind::ShutdownSkip
        );
        assert_eq!(
            BackupRequestError::MissingHomeDirectory.kind(),
            BackupRequestErrorKind::Failure
        );
    }
}
