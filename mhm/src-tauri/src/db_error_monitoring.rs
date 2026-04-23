use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

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
        classify_db_failure, inject_db_error_group, DbErrorGroup, MonitoredDbFailure,
    };
    use serde_json::json;

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
            classify_db_failure(MonitoredDbFailure::DatabaseRead("row missing from projection")),
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
