//! Issue #88: targeted optimistic version conflict handling.
//!
//! Services should increment `optimistic_version` in the same guarded UPDATE
//! that changes a row, and call `ensure_updated` with the number of rows
//! affected. A zero-row result means the row changed while the operator was
//! editing it; return `CONFLICT_OPTIMISTIC_VERSION` instead of overwriting.

#![allow(dead_code)]

pub const CONFLICT_OPTIMISTIC_VERSION: &str = "CONFLICT_OPTIMISTIC_VERSION";
pub const OPTIMISTIC_VERSION_COLUMN: &str = "optimistic_version";
pub const VERSION_BUMP_SQL: &str = "optimistic_version = optimistic_version + 1";
pub const VERSION_PREDICATE_SQL: &str = "AND optimistic_version = ?";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptimisticVersionConflict {
    pub aggregate: String,
    pub expected_version: i64,
}

impl OptimisticVersionConflict {
    pub fn stale_write(aggregate: impl Into<String>, expected_version: i64) -> Self {
        Self {
            aggregate: aggregate.into(),
            expected_version,
        }
    }

    pub fn code(&self) -> &'static str {
        CONFLICT_OPTIMISTIC_VERSION
    }

    pub fn message(&self) -> String {
        format!(
            "This {aggregate} was changed by another action. Reload the latest version and try again.",
            aggregate = self.aggregate
        )
    }

    pub fn command_error(&self) -> serde_json::Value {
        serde_json::json!({
            "code": self.code(),
            "kind": "user",
            "message": self.message(),
            "support_id": null,
            "context": {
                "aggregate": self.aggregate,
                "expected_version": self.expected_version,
            }
        })
    }
}

pub fn ensure_updated(
    affected_rows: u64,
    aggregate: &str,
    expected_version: i64,
) -> Result<(), OptimisticVersionConflict> {
    if affected_rows == 0 {
        Err(OptimisticVersionConflict::stale_write(aggregate, expected_version))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn stale_guarded_update_reports_optimistic_version_conflict() {
        let conflict = ensure_updated(0, "booking", 4).unwrap_err();
        assert_eq!(conflict.code(), CONFLICT_OPTIMISTIC_VERSION);
        assert_eq!(conflict.aggregate, "booking");
        assert_eq!(conflict.expected_version, 4);
        let payload = conflict.command_error();
        assert_eq!(payload["code"].as_str(), Some(CONFLICT_OPTIMISTIC_VERSION));
        assert_eq!(payload["kind"].as_str(), Some("user"));
        assert!(payload["support_id"].is_null());
    }

    #[test]
    fn successful_guarded_update_is_not_a_conflict() {
        assert!(ensure_updated(1, "room", 7).is_ok());
    }

    #[tokio::test]
    async fn guarded_update_returns_zero_rows_for_stale_version() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE bookings (id TEXT PRIMARY KEY, optimistic_version INTEGER NOT NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO bookings (id, optimistic_version) VALUES ('b1', 1)")
            .execute(&pool)
            .await
            .unwrap();

        let update_sql =
            "UPDATE bookings SET optimistic_version = optimistic_version + 1 WHERE id = ? AND optimistic_version = ?";
        let updated = sqlx::query(update_sql).bind("b1").bind(1).execute(&pool).await.unwrap();
        ensure_updated(updated.rows_affected(), "booking", 1).unwrap();

        let stale = sqlx::query(update_sql).bind("b1").bind(1).execute(&pool).await.unwrap();
        let conflict = ensure_updated(stale.rows_affected(), "booking", 1).unwrap_err();
        assert_eq!(conflict.code(), CONFLICT_OPTIMISTIC_VERSION);
    }

    #[test]
    fn migration_is_present_and_adds_targeted_version_columns() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("migrations")
            .join("24_add_targeted_optimistic_version_columns.sql");
        let sql = std::fs::read_to_string(path).expect("migration file exists");
        assert!(sql.contains(
            "ALTER TABLE bookings ADD COLUMN optimistic_version INTEGER NOT NULL DEFAULT 1;"
        ));
        assert!(sql.contains(
            "ALTER TABLE rooms ADD COLUMN optimistic_version INTEGER NOT NULL DEFAULT 1;"
        ));
    }
}
