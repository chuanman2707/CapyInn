use chrono::NaiveDateTime;

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
    let Some(stem) = name.strip_suffix(".db") else {
        return false;
    };

    let Some(rest) = stem.strip_prefix("capyinn_backup_") else {
        return false;
    };

    let parts = rest.split('_').collect::<Vec<_>>();
    if parts.len() < 3 {
        return false;
    }

    let reason = parts[..parts.len() - 2].join("_");
    let date = parts[parts.len() - 2];
    let time = parts[parts.len() - 1];

    let valid_reason = matches!(
        reason.as_str(),
        "settings" | "checkout" | "group_checkout" | "night_audit" | "app_exit" | "manual"
    );

    let valid_timestamp = date.len() == 8
        && date.chars().all(|ch| ch.is_ascii_digit())
        && time.len() == 6
        && time.chars().all(|ch| ch.is_ascii_digit());

    valid_reason && valid_timestamp
}

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

    #[test]
    fn matches_only_managed_backup_files() {
        assert!(is_managed_backup_file("capyinn_backup_settings_20260418_231500.db"));
        assert!(is_managed_backup_file("capyinn_backup_app_exit_20260419_000102.db"));
        assert!(!is_managed_backup_file("capyinn_backup_unknown_20260418_231500.db"));
        assert!(!is_managed_backup_file("capyinn_backup_checkout_20260418_231500.db.tmp"));
        assert!(!is_managed_backup_file("notes.db"));
    }
}
