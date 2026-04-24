use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::app_error::codes::{
    AUDIT_DATE_ALREADY_RUN, CONFLICT_ROOM_UNAVAILABLE, DB_LOCKED_RETRYABLE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DbErrorGroup {
    Constraint,
    Locked,
    NotFound,
    WriteFailed,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitoredDbFailure<'a> {
    MissingRecord,
    DatabaseRead(&'a str),
    DatabaseWrite(&'a str),
}

pub fn classify_db_failure(failure: MonitoredDbFailure<'_>) -> DbErrorGroup {
    match failure {
        MonitoredDbFailure::MissingRecord => DbErrorGroup::NotFound,
        MonitoredDbFailure::DatabaseRead(message) => {
            classify_message(message).unwrap_or(DbErrorGroup::Unknown)
        }
        MonitoredDbFailure::DatabaseWrite(message) => {
            classify_message(message).unwrap_or(DbErrorGroup::WriteFailed)
        }
    }
}

pub fn classify_db_error_code(message: &str) -> Option<&'static str> {
    let normalized = message.to_ascii_lowercase();

    if message.contains(DB_LOCKED_RETRYABLE) {
        return Some(DB_LOCKED_RETRYABLE);
    }

    if message.contains(CONFLICT_ROOM_UNAVAILABLE) {
        return Some(CONFLICT_ROOM_UNAVAILABLE);
    }

    if message.contains(AUDIT_DATE_ALREADY_RUN) {
        return Some(AUDIT_DATE_ALREADY_RUN);
    }

    if normalized.contains("locked") || normalized.contains("busy") {
        return Some(DB_LOCKED_RETRYABLE);
    }

    if message.contains("UNIQUE constraint failed: room_calendar") {
        return Some(CONFLICT_ROOM_UNAVAILABLE);
    }

    if message.contains("UNIQUE constraint failed: night_audit_logs.audit_date") {
        return Some(AUDIT_DATE_ALREADY_RUN);
    }

    None
}

pub fn is_room_unavailable_conflict_message(message: &str) -> bool {
    if classify_db_error_code(message) == Some(CONFLICT_ROOM_UNAVAILABLE) {
        return true;
    }

    let lower = message.to_ascii_lowercase();
    if lower.starts_with("room ")
        && (lower.contains(" is booked on ") || lower.contains(" has a reservation starting "))
    {
        return true;
    }

    message.starts_with("Phòng ")
        && (message.contains(" có lịch trùng ") || message.contains(" không trống "))
}

fn classify_message(message: &str) -> Option<DbErrorGroup> {
    let normalized = message.to_ascii_lowercase();

    if normalized.contains("constraint")
        || normalized.contains("foreign key")
        || normalized.contains("not null")
        || normalized.contains("unique")
        || normalized.contains("duplicate")
    {
        return Some(DbErrorGroup::Constraint);
    }

    if normalized.contains("locked") || normalized.contains("busy") {
        return Some(DbErrorGroup::Locked);
    }

    None
}

pub fn inject_db_error_group(context: Value, group: DbErrorGroup) -> Value {
    match context {
        Value::Object(mut object) => {
            object.insert(
                "db_error_group".to_string(),
                serde_json::to_value(group).expect("db error group serializes"),
            );
            Value::Object(object)
        }
        other => {
            let mut object = Map::new();
            object.insert(
                "db_error_group".to_string(),
                serde_json::to_value(group).expect("db error group serializes"),
            );
            object.insert("context".to_string(), other);
            Value::Object(object)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_db_error_code, classify_db_failure, inject_db_error_group,
        is_room_unavailable_conflict_message, DbErrorGroup, MonitoredDbFailure,
    };
    use crate::app_error::codes::{
        AUDIT_DATE_ALREADY_RUN, CONFLICT_ROOM_UNAVAILABLE, DB_LOCKED_RETRYABLE,
    };
    use serde_json::json;

    #[test]
    fn classify_db_error_code_maps_retryable_lock_and_busy_messages() {
        assert_eq!(
            classify_db_error_code("database is locked"),
            Some(DB_LOCKED_RETRYABLE)
        );
        assert_eq!(
            classify_db_error_code("SQLITE_BUSY: database is busy"),
            Some(DB_LOCKED_RETRYABLE)
        );
        assert_eq!(
            classify_db_error_code("DB_LOCKED_RETRYABLE: retry later"),
            Some(DB_LOCKED_RETRYABLE)
        );
    }

    #[test]
    fn classify_db_error_code_maps_known_unique_constraints() {
        assert_eq!(
            classify_db_error_code(
                "UNIQUE constraint failed: room_calendar.room_id, room_calendar.date"
            ),
            Some(CONFLICT_ROOM_UNAVAILABLE)
        );
        assert_eq!(
            classify_db_error_code(
                "CONFLICT_ROOM_UNAVAILABLE: room_calendar conflict on 2026-04-20"
            ),
            Some(CONFLICT_ROOM_UNAVAILABLE)
        );
        assert_eq!(
            classify_db_error_code("UNIQUE constraint failed: night_audit_logs.audit_date"),
            Some(AUDIT_DATE_ALREADY_RUN)
        );
        assert_eq!(
            classify_db_error_code("AUDIT_DATE_ALREADY_RUN: 2026-04-20"),
            Some(AUDIT_DATE_ALREADY_RUN)
        );
    }

    #[test]
    fn classify_db_error_code_returns_none_for_unmapped_messages() {
        assert_eq!(classify_db_error_code("disk I/O error"), None);
    }

    #[test]
    fn room_unavailable_conflict_matches_stable_and_legacy_messages() {
        assert!(is_room_unavailable_conflict_message(
            "CONFLICT_ROOM_UNAVAILABLE: room_calendar conflict on 2026-04-20"
        ));
        assert!(is_room_unavailable_conflict_message(
            "Room R101 is booked on 2026-04-20. Cannot create reservation."
        ));
        assert!(is_room_unavailable_conflict_message(
            "Room R101 has a reservation starting 2026-04-20 (Guest). Max 2 nights."
        ));
        assert!(is_room_unavailable_conflict_message(
            "Phòng R101 có lịch trùng trong khoảng ngày đã chọn"
        ));
        assert!(is_room_unavailable_conflict_message(
            "Phòng R101 không trống (status: occupied)"
        ));
        assert!(!is_room_unavailable_conflict_message(
            "Can only cancel reservations in 'booked' status"
        ));
    }

    #[test]
    fn classify_db_failure_maps_constraint_locked_unknown_missing_and_write_failed() {
        assert_eq!(
            classify_db_failure(MonitoredDbFailure::DatabaseWrite(
                "UNIQUE constraint failed: rooms.number",
            )),
            DbErrorGroup::Constraint
        );
        assert_eq!(
            classify_db_failure(MonitoredDbFailure::DatabaseWrite("database is locked")),
            DbErrorGroup::Locked
        );
        assert_eq!(
            classify_db_failure(MonitoredDbFailure::DatabaseRead(
                "row missing from projection"
            )),
            DbErrorGroup::Unknown
        );
        assert_eq!(
            classify_db_failure(MonitoredDbFailure::MissingRecord),
            DbErrorGroup::NotFound
        );
        assert_eq!(
            classify_db_failure(MonitoredDbFailure::DatabaseWrite("disk I/O error")),
            DbErrorGroup::WriteFailed
        );
    }

    #[test]
    fn inject_db_error_group_keeps_existing_context_fields_and_adds_snake_case_group() {
        let merged = inject_db_error_group(
            json!({
                "command": "check_in",
                "room_id": "R101",
            }),
            DbErrorGroup::WriteFailed,
        );

        assert_eq!(
            merged,
            json!({
                "command": "check_in",
                "room_id": "R101",
                "db_error_group": "write_failed",
            })
        );
    }

    #[test]
    fn inject_db_error_group_wraps_non_object_context() {
        let merged = inject_db_error_group(json!("raw context"), DbErrorGroup::Locked);

        assert_eq!(
            merged,
            json!({
                "db_error_group": "locked",
                "context": "raw context",
            })
        );
    }
}
