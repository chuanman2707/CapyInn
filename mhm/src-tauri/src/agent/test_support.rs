use serde_json::{Map, Number, Value};
use sqlx::{sqlite::SqliteRow, Pool, Row, Sqlite, TypeInfo as _, ValueRef as _};

const ALLOWED_PHASE_ONE_AGENT_MUTATION_TABLES: &[&str] =
    &["agent_sessions", "agent_audit_events", "agent_digest_runs"];

pub(crate) fn phase_one_table_is_snapshotted(table: &str) -> bool {
    if table == "schema_version" || table.starts_with("sqlite_") {
        return false;
    }

    !ALLOWED_PHASE_ONE_AGENT_MUTATION_TABLES.contains(&table)
}

pub(crate) async fn phase_one_pms_table_snapshots(
    pool: &Pool<Sqlite>,
) -> Vec<(String, Vec<Value>)> {
    let rows = sqlx::query(
        "SELECT name FROM sqlite_master
         WHERE type = 'table'
           AND name NOT LIKE 'sqlite_%'
         ORDER BY name ASC",
    )
    .fetch_all(pool)
    .await
    .expect("list tables");

    let mut snapshots = Vec::new();
    for row in rows {
        let table: String = row.get("name");
        if !phase_one_table_is_snapshotted(&table) {
            continue;
        }

        let columns = table_column_names(pool, &table).await;
        let select_sql = format!("SELECT * FROM {}", quote_identifier(&table));
        let row_values = sqlx::query(&select_sql)
            .fetch_all(pool)
            .await
            .expect("snapshot table rows")
            .into_iter()
            .map(|row| sqlite_row_json(&row, &columns))
            .collect::<Vec<_>>();
        let mut serialized_rows = row_values
            .into_iter()
            .map(|value| serde_json::to_string(&value).expect("serialize snapshot row"))
            .collect::<Vec<_>>();
        serialized_rows.sort();
        let sorted_rows = serialized_rows
            .into_iter()
            .map(|value| serde_json::from_str(&value).expect("deserialize snapshot row"))
            .collect::<Vec<_>>();
        snapshots.push((table, sorted_rows));
    }
    snapshots
}

async fn table_column_names(pool: &Pool<Sqlite>, table: &str) -> Vec<String> {
    let pragma_sql = format!("PRAGMA table_info({})", quote_identifier(table));
    let mut columns = sqlx::query(&pragma_sql)
        .fetch_all(pool)
        .await
        .expect("list table columns")
        .into_iter()
        .map(|row| (row.get::<i64, _>("cid"), row.get::<String, _>("name")))
        .collect::<Vec<_>>();
    columns.sort_by_key(|(cid, _)| *cid);
    columns.into_iter().map(|(_, name)| name).collect()
}

fn sqlite_row_json(row: &SqliteRow, columns: &[String]) -> Value {
    let mut object = Map::new();
    for column in columns {
        object.insert(column.clone(), sqlite_cell_json(row, column));
    }
    Value::Object(object)
}

fn sqlite_cell_json(row: &SqliteRow, column: &str) -> Value {
    let raw = row.try_get_raw(column).expect("read raw sqlite value");
    if raw.is_null() {
        return Value::Null;
    }

    match raw.type_info().name() {
        "INTEGER" | "BOOLEAN" => Value::from(row.get::<i64, _>(column)),
        "REAL" => Number::from_f64(row.get::<f64, _>(column))
            .map(Value::Number)
            .unwrap_or(Value::Null),
        "TEXT" | "DATE" | "TIME" | "DATETIME" => Value::from(row.get::<String, _>(column)),
        "BLOB" => Value::Array(
            row.get::<Vec<u8>, _>(column)
                .into_iter()
                .map(Value::from)
                .collect(),
        ),
        other => Value::from(format!("[unsupported sqlite type: {other}]")),
    }
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};

    async fn test_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect sqlite");
        crate::db::run_migrations(&pool).await.expect("migrations");
        pool
    }

    #[test]
    fn phase_one_classifier_defaults_unknown_non_agent_tables_to_snapshotted() {
        assert!(phase_one_table_is_snapshotted("rooms"));
        assert!(phase_one_table_is_snapshotted("outbox_events"));
        assert!(phase_one_table_is_snapshotted("settings"));
        assert!(phase_one_table_is_snapshotted("command_idempotency"));
        assert!(phase_one_table_is_snapshotted("agent_memory_items"));
        assert!(phase_one_table_is_snapshotted(
            "future_business_truth_table"
        ));
        assert!(!phase_one_table_is_snapshotted("schema_version"));
        assert!(!phase_one_table_is_snapshotted("sqlite_sequence"));
        assert!(!phase_one_table_is_snapshotted("agent_sessions"));
        assert!(!phase_one_table_is_snapshotted("agent_audit_events"));
        assert!(!phase_one_table_is_snapshotted("agent_digest_runs"));
    }

    #[tokio::test]
    async fn phase_one_snapshots_include_sensitive_metadata_and_outbox_tables() {
        let pool = test_pool().await;
        let snapshots = phase_one_pms_table_snapshots(&pool).await;
        let names = snapshots
            .iter()
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();

        assert!(names.contains(&"settings".to_string()));
        assert!(names.contains(&"outbox_events".to_string()));
        assert!(names.contains(&"agent_memory_items".to_string()));
        assert!(!names.contains(&"agent_sessions".to_string()));
        assert!(!names.contains(&"agent_audit_events".to_string()));
        assert!(!names.contains(&"agent_digest_runs".to_string()));
        assert!(!names.iter().any(|name| name.starts_with("sqlite_")));
    }
}
