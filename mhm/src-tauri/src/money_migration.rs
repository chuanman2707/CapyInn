use crate::money::{MoneyVnd, MAX_TRANSPORT_SAFE_MONEY_VND, MIN_TRANSPORT_SAFE_MONEY_VND};
use serde_json::{Number, Value};
use sqlx::{Row, Sqlite, Transaction};
use std::collections::HashSet;

#[derive(Debug, Clone)]
pub(crate) struct MoneyMigrationIssue {
    pub location: String,
    pub value: String,
    pub reason: String,
}

impl MoneyMigrationIssue {
    fn message(&self) -> String {
        format!("{} = {} ({})", self.location, self.value, self.reason)
    }
}

#[derive(Debug, Clone, Copy)]
struct MoneyColumn {
    table: &'static str,
    id_column: &'static str,
    column: &'static str,
}

#[derive(Debug, Clone, Copy)]
struct MoneyTable {
    table: &'static str,
    columns: &'static [&'static str],
}

#[derive(Debug, Clone, Copy)]
struct JsonColumn {
    table: &'static str,
    id_column: &'static str,
    column: &'static str,
}

const MONEY_COLUMNS: &[MoneyColumn] = &[
    MoneyColumn {
        table: "rooms",
        id_column: "id",
        column: "base_price",
    },
    MoneyColumn {
        table: "rooms",
        id_column: "id",
        column: "extra_person_fee",
    },
    MoneyColumn {
        table: "pricing_rules",
        id_column: "id",
        column: "hourly_rate",
    },
    MoneyColumn {
        table: "pricing_rules",
        id_column: "id",
        column: "overnight_rate",
    },
    MoneyColumn {
        table: "pricing_rules",
        id_column: "id",
        column: "daily_rate",
    },
    MoneyColumn {
        table: "bookings",
        id_column: "id",
        column: "total_price",
    },
    MoneyColumn {
        table: "bookings",
        id_column: "id",
        column: "paid_amount",
    },
    MoneyColumn {
        table: "bookings",
        id_column: "id",
        column: "deposit_amount",
    },
    MoneyColumn {
        table: "transactions",
        id_column: "id",
        column: "amount",
    },
    MoneyColumn {
        table: "expenses",
        id_column: "id",
        column: "amount",
    },
    MoneyColumn {
        table: "folio_lines",
        id_column: "id",
        column: "amount",
    },
    MoneyColumn {
        table: "night_audit_logs",
        id_column: "id",
        column: "total_revenue",
    },
    MoneyColumn {
        table: "night_audit_logs",
        id_column: "id",
        column: "room_revenue",
    },
    MoneyColumn {
        table: "night_audit_logs",
        id_column: "id",
        column: "folio_revenue",
    },
    MoneyColumn {
        table: "night_audit_logs",
        id_column: "id",
        column: "total_expenses",
    },
    MoneyColumn {
        table: "invoices",
        id_column: "id",
        column: "subtotal",
    },
    MoneyColumn {
        table: "invoices",
        id_column: "id",
        column: "deposit_amount",
    },
    MoneyColumn {
        table: "invoices",
        id_column: "id",
        column: "total",
    },
    MoneyColumn {
        table: "invoices",
        id_column: "id",
        column: "balance_due",
    },
    MoneyColumn {
        table: "group_services",
        id_column: "id",
        column: "unit_price",
    },
    MoneyColumn {
        table: "group_services",
        id_column: "id",
        column: "total_price",
    },
];

const MONEY_TABLES: &[MoneyTable] = &[
    MoneyTable {
        table: "rooms",
        columns: &["base_price", "extra_person_fee"],
    },
    MoneyTable {
        table: "pricing_rules",
        columns: &["hourly_rate", "overnight_rate", "daily_rate"],
    },
    MoneyTable {
        table: "bookings",
        columns: &["total_price", "paid_amount", "deposit_amount"],
    },
    MoneyTable {
        table: "transactions",
        columns: &["amount"],
    },
    MoneyTable {
        table: "expenses",
        columns: &["amount"],
    },
    MoneyTable {
        table: "folio_lines",
        columns: &["amount"],
    },
    MoneyTable {
        table: "night_audit_logs",
        columns: &[
            "total_revenue",
            "room_revenue",
            "folio_revenue",
            "total_expenses",
        ],
    },
    MoneyTable {
        table: "invoices",
        columns: &["subtotal", "deposit_amount", "total", "balance_due"],
    },
    MoneyTable {
        table: "group_services",
        columns: &["unit_price", "total_price"],
    },
];

const BUSINESS_JSON_COLUMNS: &[JsonColumn] = &[
    JsonColumn {
        table: "bookings",
        id_column: "id",
        column: "pricing_snapshot",
    },
    JsonColumn {
        table: "invoices",
        id_column: "id",
        column: "pricing_breakdown",
    },
];

const COMMAND_JSON_COLUMNS: &[JsonColumn] = &[
    JsonColumn {
        table: "command_idempotency",
        id_column: "id",
        column: "intent_json",
    },
    JsonColumn {
        table: "command_idempotency",
        id_column: "id",
        column: "response_json",
    },
    JsonColumn {
        table: "command_idempotency",
        id_column: "id",
        column: "error_json",
    },
    JsonColumn {
        table: "command_idempotency",
        id_column: "id",
        column: "summary_json",
    },
    JsonColumn {
        table: "command_idempotency",
        id_column: "id",
        column: "result_summary_json",
    },
    JsonColumn {
        table: "command_idempotency",
        id_column: "id",
        column: "error_summary_json",
    },
];

const JSON_MONEY_KEYS: &[&str] = &[
    "amount",
    "base_amount",
    "surcharge_amount",
    "weekend_amount",
    "special_amount",
    "total",
    "total_price",
    "paid_amount",
    "deposit_amount",
    "balance_due",
    "subtotal",
    "unit_price",
    "base_price",
    "extra_person_fee",
    "hourly_rate",
    "overnight_rate",
    "daily_rate",
    "final_total",
    "recommended_total",
    "original_total",
    "settled_total",
    "charge_total",
    "cancellation_fee_total",
    "folio_total",
    "total_revenue",
    "room_revenue",
    "folio_revenue",
    "total_expenses",
    "subtotal_rooms",
    "subtotal_services",
    "total_room_cost",
    "total_service_cost",
    "grand_total",
    "price_per_night",
    "deposit_vnd_units",
];

pub(crate) async fn migrate_integer_vnd_money(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<(), sqlx::Error> {
    let issues = scan_money_issues(tx).await?;
    if !issues.is_empty() {
        let preview = issues
            .iter()
            .take(20)
            .map(MoneyMigrationIssue::message)
            .collect::<Vec<_>>()
            .join("; ");
        return Err(sqlx::Error::Protocol(format!(
            "Integer VND migration blocked by {} invalid money value(s): {}",
            issues.len(),
            preview
        )));
    }

    rebuild_money_tables(tx).await?;
    convert_money_json(tx, BUSINESS_JSON_COLUMNS).await?;
    migrate_command_replay_json(tx).await?;
    assert_foreign_keys_valid(tx).await?;
    Ok(())
}

async fn scan_money_issues(
    tx: &mut Transaction<'_, Sqlite>,
) -> Result<Vec<MoneyMigrationIssue>, sqlx::Error> {
    let mut issues = Vec::new();

    for column in MONEY_COLUMNS {
        if !column_exists(tx, column.table, column.column).await? {
            continue;
        }

        let sql = format!(
            "SELECT {id} AS row_id, {col} AS value, CAST({col} AS TEXT) AS value_text
             FROM {table}
             WHERE {col} IS NOT NULL",
            id = quote_identifier(column.id_column),
            col = quote_identifier(column.column),
            table = quote_identifier(column.table)
        );
        for row in sqlx::query(&sql).fetch_all(&mut **tx).await? {
            let row_id = read_row_id(&row);
            let value = read_sqlite_money_value(&row, "value");
            if let Err(reason) = validate_legacy_money_value(value) {
                let value_text = row
                    .try_get::<Option<String>, _>("value_text")
                    .ok()
                    .flatten()
                    .or_else(|| value.map(|value| value.to_string()))
                    .unwrap_or_else(|| "NULL".to_string());
                issues.push(MoneyMigrationIssue {
                    location: format!("{}.{} row {}", column.table, column.column, row_id),
                    value: value_text,
                    reason,
                });
            }
        }
    }

    scan_json_issues(tx, BUSINESS_JSON_COLUMNS, &mut issues).await?;
    scan_json_issues(tx, COMMAND_JSON_COLUMNS, &mut issues).await?;

    Ok(issues)
}

fn read_sqlite_money_value(row: &sqlx::sqlite::SqliteRow, column: &str) -> Option<f64> {
    row.try_get::<f64, _>(column)
        .ok()
        .or_else(|| row.try_get::<i64, _>(column).ok().map(|value| value as f64))
}

fn validate_legacy_money_value(value: Option<f64>) -> Result<MoneyVnd, String> {
    let value = value.ok_or_else(|| "missing money value".to_string())?;
    if !value.is_finite() {
        return Err("money value is not finite".to_string());
    }
    if value.fract() != 0.0 {
        return Err("money value contains fractional VND".to_string());
    }
    if value < MIN_TRANSPORT_SAFE_MONEY_VND as f64 || value > MAX_TRANSPORT_SAFE_MONEY_VND as f64 {
        return Err("money value is outside safe integer range".to_string());
    }
    Ok(value as MoneyVnd)
}

async fn rebuild_money_tables(tx: &mut Transaction<'_, Sqlite>) -> Result<(), sqlx::Error> {
    for table in MONEY_TABLES {
        if !table_exists(tx, table.table).await? {
            continue;
        }

        let existing_columns = table_columns(tx, table.table).await?;
        let money_columns = table
            .columns
            .iter()
            .copied()
            .filter(|column| existing_columns.iter().any(|existing| existing == column))
            .collect::<HashSet<_>>();
        if money_columns.is_empty() {
            continue;
        }

        rebuild_table_with_integer_money(tx, table.table, &existing_columns, &money_columns)
            .await?;
    }

    Ok(())
}

async fn rebuild_table_with_integer_money(
    tx: &mut Transaction<'_, Sqlite>,
    table: &str,
    columns: &[String],
    money_columns: &HashSet<&str>,
) -> Result<(), sqlx::Error> {
    let new_table = format!("__money_migration_{table}");
    let create_sql: String = sqlx::query_scalar(
        "SELECT sql
         FROM sqlite_master
         WHERE type = 'table' AND name = ?",
    )
    .bind(table)
    .fetch_one(&mut **tx)
    .await?;
    let index_sql = table_index_sql(tx, table).await?;
    let new_create_sql = rewrite_create_table_sql(&create_sql, &new_table, money_columns)?;

    sqlx::query(&format!(
        "DROP TABLE IF EXISTS {}",
        quote_identifier(&new_table)
    ))
    .execute(&mut **tx)
    .await?;
    sqlx::query(&new_create_sql).execute(&mut **tx).await?;

    let column_list = columns
        .iter()
        .map(|column| quote_identifier(column))
        .collect::<Vec<_>>()
        .join(", ");
    let select_list = columns
        .iter()
        .map(|column| {
            let quoted = quote_identifier(column);
            if money_columns.contains(column.as_str()) {
                format!("CAST({quoted} AS INTEGER) AS {quoted}")
            } else {
                quoted
            }
        })
        .collect::<Vec<_>>()
        .join(", ");

    sqlx::query(&format!(
        "INSERT INTO {new_table} ({column_list})
         SELECT {select_list} FROM {table}",
        new_table = quote_identifier(&new_table),
        table = quote_identifier(table)
    ))
    .execute(&mut **tx)
    .await?;

    sqlx::query(&format!("DROP TABLE {}", quote_identifier(table)))
        .execute(&mut **tx)
        .await?;
    sqlx::query(&format!(
        "ALTER TABLE {} RENAME TO {}",
        quote_identifier(&new_table),
        quote_identifier(table)
    ))
    .execute(&mut **tx)
    .await?;

    for sql in index_sql {
        sqlx::query(&sql).execute(&mut **tx).await?;
    }

    Ok(())
}

async fn convert_money_json(
    tx: &mut Transaction<'_, Sqlite>,
    columns: &[JsonColumn],
) -> Result<(), sqlx::Error> {
    for column in columns {
        convert_json_column(tx, column).await?;
    }
    Ok(())
}

async fn migrate_command_replay_json(tx: &mut Transaction<'_, Sqlite>) -> Result<(), sqlx::Error> {
    if table_exists(tx, "command_idempotency").await?
        && column_exists(tx, "command_idempotency", "legacy_request_hash").await?
    {
        sqlx::query(
            "UPDATE command_idempotency
             SET legacy_request_hash = request_hash
             WHERE legacy_request_hash IS NULL",
        )
        .execute(&mut **tx)
        .await?;
    }

    convert_money_json(tx, COMMAND_JSON_COLUMNS).await
}

async fn convert_json_column(
    tx: &mut Transaction<'_, Sqlite>,
    column: &JsonColumn,
) -> Result<(), sqlx::Error> {
    if !column_exists(tx, column.table, column.column).await? {
        return Ok(());
    }

    let sql = format!(
        "SELECT {id} AS row_id, {col} AS raw_json
         FROM {table}
         WHERE {col} IS NOT NULL AND trim({col}) != ''",
        id = quote_identifier(column.id_column),
        col = quote_identifier(column.column),
        table = quote_identifier(column.table)
    );
    let rows = sqlx::query(&sql).fetch_all(&mut **tx).await?;

    for row in rows {
        let row_id = read_row_id(&row);
        let raw_json: String = row.get("raw_json");
        let mut value = serde_json::from_str::<Value>(&raw_json).map_err(|error| {
            sqlx::Error::Protocol(format!(
                "Integer VND migration failed to parse {}.{} row {}: {}",
                column.table, column.column, row_id, error
            ))
        })?;
        let mut issues = Vec::new();
        let changed = convert_json_money_value(
            &mut value,
            &format!("{}.{} row {}", column.table, column.column, row_id),
            "$",
            &mut issues,
        );
        if !issues.is_empty() {
            let preview = issues
                .iter()
                .take(20)
                .map(MoneyMigrationIssue::message)
                .collect::<Vec<_>>()
                .join("; ");
            return Err(sqlx::Error::Protocol(format!(
                "Integer VND migration blocked by {} invalid money value(s): {}",
                issues.len(),
                preview
            )));
        }
        if changed {
            let converted = serde_json::to_string(&value).map_err(|error| {
                sqlx::Error::Protocol(format!(
                    "Integer VND migration failed to serialize {}.{} row {}: {}",
                    column.table, column.column, row_id, error
                ))
            })?;
            let update_sql = format!(
                "UPDATE {table} SET {col} = ? WHERE {id} = ?",
                table = quote_identifier(column.table),
                col = quote_identifier(column.column),
                id = quote_identifier(column.id_column)
            );
            sqlx::query(&update_sql)
                .bind(converted)
                .bind(row_id)
                .execute(&mut **tx)
                .await?;
        }
    }

    Ok(())
}

async fn scan_json_issues(
    tx: &mut Transaction<'_, Sqlite>,
    columns: &[JsonColumn],
    issues: &mut Vec<MoneyMigrationIssue>,
) -> Result<(), sqlx::Error> {
    for column in columns {
        if !column_exists(tx, column.table, column.column).await? {
            continue;
        }

        let sql = format!(
            "SELECT {id} AS row_id, {col} AS raw_json
             FROM {table}
             WHERE {col} IS NOT NULL AND trim({col}) != ''",
            id = quote_identifier(column.id_column),
            col = quote_identifier(column.column),
            table = quote_identifier(column.table)
        );
        for row in sqlx::query(&sql).fetch_all(&mut **tx).await? {
            let row_id = read_row_id(&row);
            let raw_json: String = row.get("raw_json");
            match serde_json::from_str::<Value>(&raw_json) {
                Ok(mut value) => {
                    convert_json_money_value(
                        &mut value,
                        &format!("{}.{} row {}", column.table, column.column, row_id),
                        "$",
                        issues,
                    );
                }
                Err(error) => issues.push(MoneyMigrationIssue {
                    location: format!("{}.{} row {}", column.table, column.column, row_id),
                    value: raw_json,
                    reason: format!("invalid JSON: {error}"),
                }),
            }
        }
    }

    Ok(())
}

fn convert_json_money_value(
    value: &mut Value,
    location_prefix: &str,
    path: &str,
    issues: &mut Vec<MoneyMigrationIssue>,
) -> bool {
    match value {
        Value::Object(map) => {
            let mut changed = false;
            for (key, child) in map.iter_mut() {
                let child_path = json_child_path(path, key);
                if JSON_MONEY_KEYS.contains(&key.as_str()) {
                    match validate_json_money_value(child) {
                        Ok(Some(money)) => {
                            if *child != Value::Number(Number::from(money)) {
                                *child = Value::Number(Number::from(money));
                                changed = true;
                            }
                            continue;
                        }
                        Ok(None) => continue,
                        Err(reason) => {
                            issues.push(MoneyMigrationIssue {
                                location: format!("{location_prefix} path {child_path}"),
                                value: child.to_string(),
                                reason,
                            });
                            continue;
                        }
                    }
                }
                changed |= convert_json_money_value(child, location_prefix, &child_path, issues);
            }
            changed
        }
        Value::Array(values) => {
            let mut changed = false;
            for (index, child) in values.iter_mut().enumerate() {
                changed |= convert_json_money_value(
                    child,
                    location_prefix,
                    &format!("{path}[{index}]"),
                    issues,
                );
            }
            changed
        }
        _ => false,
    }
}

fn validate_json_money_value(value: &Value) -> Result<Option<MoneyVnd>, String> {
    match value {
        Value::Null => Ok(None),
        Value::Number(number) => if let Some(value) = number.as_i64() {
            validate_money_i128(value as i128)
        } else if let Some(value) = number.as_u64() {
            validate_money_i128(value as i128)
        } else {
            validate_legacy_money_value(number.as_f64())
        }
        .map(Some),
        _ => Err("money value is not numeric".to_string()),
    }
}

fn validate_money_i128(value: i128) -> Result<MoneyVnd, String> {
    if value < MIN_TRANSPORT_SAFE_MONEY_VND as i128 || value > MAX_TRANSPORT_SAFE_MONEY_VND as i128
    {
        return Err("money value is outside safe integer range".to_string());
    }
    Ok(value as MoneyVnd)
}

fn rewrite_create_table_sql(
    create_sql: &str,
    new_table: &str,
    money_columns: &HashSet<&str>,
) -> Result<String, sqlx::Error> {
    let start = create_sql.find('(').ok_or_else(|| {
        sqlx::Error::Protocol(format!("Cannot parse CREATE TABLE SQL: {create_sql}"))
    })?;
    let end = create_sql.rfind(')').ok_or_else(|| {
        sqlx::Error::Protocol(format!("Cannot parse CREATE TABLE SQL: {create_sql}"))
    })?;
    let body = &create_sql[start + 1..end];
    let definitions = split_table_definitions(body)
        .into_iter()
        .map(|definition| rewrite_column_definition(&definition, money_columns))
        .collect::<Vec<_>>()
        .join(",");

    Ok(format!(
        "CREATE TABLE {} ({})",
        quote_identifier(new_table),
        definitions
    ))
}

fn rewrite_column_definition(definition: &str, money_columns: &HashSet<&str>) -> String {
    let leading = definition.len() - definition.trim_start().len();
    let (leading_text, trimmed) = definition.split_at(leading);
    let Some((name, name_end)) = parse_identifier(trimmed) else {
        return definition.to_string();
    };
    if !money_columns.contains(name.as_str()) {
        return definition.to_string();
    }

    let after_name = &trimmed[name_end..];
    let whitespace = after_name.len() - after_name.trim_start().len();
    let (between_name_and_type, after_whitespace) = after_name.split_at(whitespace);
    let type_end = after_whitespace
        .find(char::is_whitespace)
        .unwrap_or(after_whitespace.len());
    let rest = &after_whitespace[type_end..];

    format!(
        "{leading_text}{}{between_name_and_type}INTEGER{rest}",
        &trimmed[..name_end]
    )
}

fn split_table_definitions(body: &str) -> Vec<String> {
    let mut definitions = Vec::new();
    let mut start = 0;
    let mut depth = 0;
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let chars = body.char_indices().collect::<Vec<_>>();
    let mut index = 0;

    while index < chars.len() {
        let (position, ch) = chars[index];
        match ch {
            '\'' if !in_double_quote => {
                if in_single_quote
                    && chars
                        .get(index + 1)
                        .map(|(_, next)| *next == '\'')
                        .unwrap_or(false)
                {
                    index += 1;
                } else {
                    in_single_quote = !in_single_quote;
                }
            }
            '"' if !in_single_quote => in_double_quote = !in_double_quote,
            '(' if !in_single_quote && !in_double_quote => depth += 1,
            ')' if !in_single_quote && !in_double_quote && depth > 0 => depth -= 1,
            ',' if !in_single_quote && !in_double_quote && depth == 0 => {
                definitions.push(body[start..position].to_string());
                start = position + ch.len_utf8();
            }
            _ => {}
        }
        index += 1;
    }

    definitions.push(body[start..].to_string());
    definitions
}

fn parse_identifier(value: &str) -> Option<(String, usize)> {
    let mut chars = value.char_indices();
    let (_, first) = chars.next()?;
    if matches!(first, '"' | '`' | '[') {
        let closing = if first == '[' { ']' } else { first };
        for (position, ch) in chars {
            if ch == closing {
                return Some((value[1..position].to_string(), position + ch.len_utf8()));
            }
        }
        return None;
    }

    let mut end = first.len_utf8();
    for (position, ch) in value.char_indices().skip(1) {
        if ch.is_whitespace() {
            break;
        }
        end = position + ch.len_utf8();
    }
    Some((value[..end].to_string(), end))
}

async fn table_exists(tx: &mut Transaction<'_, Sqlite>, table: &str) -> Result<bool, sqlx::Error> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM sqlite_master
         WHERE type = 'table' AND name = ?",
    )
    .bind(table)
    .fetch_one(&mut **tx)
    .await?;
    Ok(count > 0)
}

async fn column_exists(
    tx: &mut Transaction<'_, Sqlite>,
    table: &str,
    column: &str,
) -> Result<bool, sqlx::Error> {
    if !table_exists(tx, table).await? {
        return Ok(false);
    }
    let sql = format!(
        "SELECT COUNT(*) FROM pragma_table_info({}) WHERE name = ?",
        quote_sql_literal(table)
    );
    let count: i64 = sqlx::query_scalar(&sql)
        .bind(column)
        .fetch_one(&mut **tx)
        .await?;
    Ok(count > 0)
}

async fn table_columns(
    tx: &mut Transaction<'_, Sqlite>,
    table: &str,
) -> Result<Vec<String>, sqlx::Error> {
    let sql = format!(
        "SELECT name FROM pragma_table_info({}) ORDER BY cid",
        quote_sql_literal(table)
    );
    sqlx::query_scalar::<_, String>(&sql)
        .fetch_all(&mut **tx)
        .await
}

async fn table_index_sql(
    tx: &mut Transaction<'_, Sqlite>,
    table: &str,
) -> Result<Vec<String>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT sql
         FROM sqlite_master
         WHERE type = 'index' AND tbl_name = ? AND sql IS NOT NULL
         ORDER BY name",
    )
    .bind(table)
    .fetch_all(&mut **tx)
    .await
}

async fn assert_foreign_keys_valid(tx: &mut Transaction<'_, Sqlite>) -> Result<(), sqlx::Error> {
    let rows = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut **tx)
        .await?;
    if rows.is_empty() {
        return Ok(());
    }

    let details = rows
        .iter()
        .take(20)
        .map(|row| {
            let table = row.try_get::<String, _>("table").unwrap_or_default();
            let rowid = row.try_get::<i64, _>("rowid").unwrap_or_default();
            let parent = row.try_get::<String, _>("parent").unwrap_or_default();
            format!("{table} row {rowid} references missing {parent}")
        })
        .collect::<Vec<_>>()
        .join("; ");
    Err(sqlx::Error::Protocol(format!(
        "Integer VND migration left {} foreign key issue(s): {}",
        rows.len(),
        details
    )))
}

fn read_row_id(row: &sqlx::sqlite::SqliteRow) -> String {
    row.try_get::<String, _>("row_id")
        .or_else(|_| {
            row.try_get::<i64, _>("row_id")
                .map(|value| value.to_string())
        })
        .unwrap_or_else(|_| "<unknown>".to_string())
}

fn json_child_path(parent: &str, key: &str) -> String {
    if key
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        format!("{parent}.{key}")
    } else {
        format!(
            "{parent}[{}]",
            serde_json::to_string(key).unwrap_or_default()
        )
    }
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn quote_sql_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}
