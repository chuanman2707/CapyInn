use chrono::Local;
use serde::Serialize;
use serde_json::{json, Value};
use sqlx::{Pool, Row, Sqlite};

use crate::{
    agent::registry::CEO_PHASE_A_TOOLS,
    app_error::{codes, CommandError, CommandResult},
    queries::booking::{audit_queries, ceo_read_queries, revenue_queries},
};

#[derive(Debug, Clone, Serialize)]
pub struct CeoReadToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct CeoToolEnvelope {
    pub ok: bool,
    pub tool: String,
    pub data: Value,
    pub metadata: Value,
}

pub fn ceo_read_tool_schemas() -> Vec<CeoReadToolSchema> {
    CEO_PHASE_A_TOOLS
        .iter()
        .map(|tool| CeoReadToolSchema {
            name: tool.name.to_string(),
            description: tool.description.to_string(),
            parameters: no_args_schema(),
        })
        .collect()
}

pub async fn dispatch_ceo_read_tool(
    pool: &Pool<Sqlite>,
    name: &str,
    args: Value,
) -> CommandResult<CeoToolEnvelope> {
    validate_no_args(&args)?;

    let today = Local::now().format("%Y-%m-%d").to_string();
    let data = match name {
        "get_hotel_status" => serde_json::to_value(
            revenue_queries::load_dashboard_stats_for_date(pool, &today)
                .await
                .map_err(query_error)?,
        ),
        "list_room_status" => {
            serde_json::to_value(load_room_status(pool).await.map_err(query_error)?)
        }
        "list_today_arrivals" => serde_json::to_value(
            ceo_read_queries::list_today_arrivals(pool, &today)
                .await
                .map_err(query_error)?,
        ),
        "list_today_checkouts" => serde_json::to_value(
            ceo_read_queries::list_today_checkouts(pool, &today)
                .await
                .map_err(query_error)?,
        ),
        "list_unpaid_balances" => serde_json::to_value(
            ceo_read_queries::list_unpaid_balances(pool)
                .await
                .map_err(query_error)?,
        ),
        "get_revenue_snapshot" => serde_json::to_value(
            revenue_queries::load_revenue_stats(pool, &today, &today)
                .await
                .map_err(query_error)?,
        ),
        "get_audit_readiness" => {
            let snapshot = audit_queries::load_night_audit_snapshot(pool, &today)
                .await
                .map_err(query_error)?;
            Ok(json!({
                "audit_date": snapshot.audit_date,
                "total_revenue": snapshot.total_revenue,
                "room_revenue": snapshot.room_revenue,
                "folio_revenue": snapshot.folio_revenue,
                "total_expenses": snapshot.total_expenses,
                "occupancy_pct": snapshot.occupancy_pct,
                "rooms_sold": snapshot.rooms_sold,
                "total_rooms": snapshot.total_rooms,
            }))
        }
        "summarize_operational_risks" => serde_json::to_value(
            ceo_read_queries::summarize_operational_risks(pool, &today)
                .await
                .map_err(query_error)?,
        ),
        _ => {
            return Err(CommandError::user(
                codes::AGENT_TOOL_NOT_ALLOWED,
                "CEO read tool is not allowed.",
            ));
        }
    }
    .map_err(|_| {
        CommandError::system(codes::SYSTEM_INTERNAL_ERROR, "Cannot serialize tool data")
    })?;

    Ok(CeoToolEnvelope {
        ok: true,
        tool: name.to_string(),
        metadata: json!({
            "business_date": today,
            "record_count": record_count(&data),
        }),
        data,
    })
}

fn query_error(_: sqlx::Error) -> CommandError {
    CommandError::system(
        codes::SYSTEM_INTERNAL_ERROR,
        "Cannot execute CEO read tool.",
    )
}

fn no_args_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false,
    })
}

fn validate_no_args(args: &Value) -> CommandResult<()> {
    let is_empty_object = args.as_object().is_some_and(|object| object.is_empty());
    if is_empty_object {
        return Ok(());
    }

    Err(CommandError::user(
        codes::VALIDATION_INVALID_INPUT,
        "CEO read tools do not accept arguments.",
    ))
}

fn record_count(data: &Value) -> usize {
    data.as_array().map_or(1, Vec::len)
}

#[derive(Debug, Clone, Serialize)]
struct CeoRoomStatus {
    room_id: String,
    name: String,
    room_type: String,
    status: String,
}

async fn load_room_status(pool: &Pool<Sqlite>) -> Result<Vec<CeoRoomStatus>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id AS room_id, name, type AS room_type, status
         FROM rooms
         ORDER BY floor ASC, id ASC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| CeoRoomStatus {
            room_id: row.get("room_id"),
            name: row.get("name"),
            room_type: row.get("room_type"),
            status: row.get("status"),
        })
        .collect())
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
            .expect("failed to open sqlite test pool");
        crate::db::run_migrations(&pool)
            .await
            .expect("run migrations");
        pool
    }

    #[test]
    fn ceo_tool_schemas_match_static_registry_names() {
        let schemas = ceo_read_tool_schemas();
        let schema_names = schemas
            .iter()
            .map(|tool| tool.name.as_str())
            .collect::<Vec<_>>();
        let registry_names = crate::agent::registry::CEO_PHASE_A_TOOLS
            .iter()
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        assert_eq!(schema_names, registry_names);
    }

    #[tokio::test]
    async fn unknown_tool_is_rejected() {
        let pool = test_pool().await;
        let error = dispatch_ceo_read_tool(&pool, "create_reservation", serde_json::json!({}))
            .await
            .expect_err("write tool must not be dispatched");
        assert_eq!(error.code, crate::app_error::codes::AGENT_TOOL_NOT_ALLOWED);
    }
}
