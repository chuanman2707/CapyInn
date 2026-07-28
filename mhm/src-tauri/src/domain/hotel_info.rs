//! The hotel's own name, address and phone, as printed on invoices.
//!
//! Stored as one JSON blob under the `hotel_info` setting. Two callers decode
//! it — the per-booking invoice and the group invoice — and each used to carry
//! its own copy of the fallback rules. Same rules, written twice, on the
//! letterhead of a document the guest keeps.

use crate::app_identity::APP_NAME;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotelInfo {
    pub name: String,
    pub address: String,
    pub phone: String,
}

impl Default for HotelInfo {
    /// An unconfigured hotel still prints a header: the app's own name, and
    /// nothing where the address and phone would be.
    fn default() -> Self {
        Self {
            name: APP_NAME.to_string(),
            address: String::new(),
            phone: String::new(),
        }
    }
}

impl HotelInfo {
    /// Decodes the stored blob. Missing, malformed, or partially filled all
    /// fall back field by field rather than failing — an invoice must print.
    pub fn from_settings_json(stored: Option<&str>) -> Self {
        let Some(parsed) =
            stored.and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok())
        else {
            return Self::default();
        };

        let field = |key: &str| {
            parsed
                .get(key)
                .and_then(|value| value.as_str())
                .map(str::to_string)
        };

        Self {
            name: field("name").unwrap_or_else(|| APP_NAME.to_string()),
            address: field("address").unwrap_or_default(),
            phone: field("phone").unwrap_or_default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HotelInfo;
    use crate::app_identity::APP_NAME;

    #[test]
    fn a_complete_blob_is_used_as_written() {
        let info = HotelInfo::from_settings_json(Some(
            r#"{"name":"Nhà nghỉ Bình Minh","address":"12 Trần Phú","phone":"0901234567"}"#,
        ));

        assert_eq!(info.name, "Nhà nghỉ Bình Minh");
        assert_eq!(info.address, "12 Trần Phú");
        assert_eq!(info.phone, "0901234567");
    }

    #[test]
    fn no_setting_at_all_falls_back_to_the_app_name() {
        assert_eq!(HotelInfo::from_settings_json(None), HotelInfo::default());
        assert_eq!(HotelInfo::from_settings_json(None).name, APP_NAME);
    }

    #[test]
    fn a_malformed_blob_falls_back_rather_than_failing() {
        // An invoice must print even if the setting was corrupted.
        assert_eq!(
            HotelInfo::from_settings_json(Some("{not-json")),
            HotelInfo::default()
        );
        assert_eq!(
            HotelInfo::from_settings_json(Some("")),
            HotelInfo::default()
        );
        assert_eq!(
            HotelInfo::from_settings_json(Some("[1,2,3]")),
            HotelInfo::default(),
            "valid JSON of the wrong shape is still a fallback"
        );
    }

    #[test]
    fn each_field_falls_back_on_its_own() {
        let info = HotelInfo::from_settings_json(Some(r#"{"address":"12 Trần Phú"}"#));

        assert_eq!(info.name, APP_NAME, "a missing name is the app name");
        assert_eq!(info.address, "12 Trần Phú", "a present field survives");
        assert_eq!(info.phone, "", "a missing phone is blank, not the app name");
    }

    #[test]
    fn a_non_string_field_is_treated_as_missing() {
        let info = HotelInfo::from_settings_json(Some(r#"{"name":42,"phone":null}"#));

        assert_eq!(info.name, APP_NAME);
        assert_eq!(info.phone, "");
    }
}
