use crate::{
    agent::{
        assistant::provider::AssistantToolSchema,
        model::{AgentRole, AgentToolCapability, AgentToolMeta, DataSensitivity, MutationRisk},
    },
    app_error::{codes, CommandError, CommandResult},
    queries::{
        booking::{billing_queries, ceo_read_queries},
        rooms::assistant_queries::{
            find_guest_stays, load_free_rooms_between, load_room_status_now,
        },
    },
};
use serde_json::{json, Value};
use sqlx::{Pool, Sqlite};

pub const DRAFT_CHECK_IN_TOOL: &str = "draft_check_in";

const FRONT_DESK_ONLY: &[AgentRole] = &[AgentRole::FrontDeskAssistant];

/// Tool có executor. Mọi tool trong danh sách này chỉ đọc.
pub const FRONT_DESK_READ_TOOLS: &[AgentToolMeta] = &[
    AgentToolMeta {
        name: "list_rooms_now",
        description: "Trạng thái mọi phòng ngay lúc này, kèm khách đang ở nếu có.",
        mutation_risk: MutationRisk::ReadOnly,
        data_sensitivity: DataSensitivity::StaffOperational,
        allowed_roles: FRONT_DESK_ONLY,
        capability: AgentToolCapability::PmsRead,
    },
    AgentToolMeta {
        name: "find_guest",
        description: "Tìm khách theo tên hoặc số điện thoại, kèm lượt ở đang mở.",
        mutation_risk: MutationRisk::ReadOnly,
        data_sensitivity: DataSensitivity::StaffOperational,
        allowed_roles: FRONT_DESK_ONLY,
        capability: AgentToolCapability::PmsRead,
    },
    AgentToolMeta {
        name: "get_stay_charges",
        description: "Tiền phòng của một lượt ở: đã phát sinh, đã trả, còn nợ.",
        mutation_risk: MutationRisk::ReadOnly,
        data_sensitivity: DataSensitivity::StaffOperational,
        allowed_roles: FRONT_DESK_ONLY,
        capability: AgentToolCapability::PmsRead,
    },
    AgentToolMeta {
        name: "quote_room_price",
        description: "Báo giá một phòng cho khoảng ngày và số khách cụ thể.",
        mutation_risk: MutationRisk::ReadOnly,
        data_sensitivity: DataSensitivity::StaffOperational,
        allowed_roles: FRONT_DESK_ONLY,
        capability: AgentToolCapability::PmsRead,
    },
    AgentToolMeta {
        name: "check_room_availability",
        description: "Phòng còn trống trong một khoảng ngày.",
        mutation_risk: MutationRisk::ReadOnly,
        data_sensitivity: DataSensitivity::StaffOperational,
        allowed_roles: FRONT_DESK_ONLY,
        capability: AgentToolCapability::PmsRead,
    },
    AgentToolMeta {
        name: "get_today_operations",
        description: "Khách đến và khách trả phòng trong hôm nay.",
        mutation_risk: MutationRisk::ReadOnly,
        data_sensitivity: DataSensitivity::StaffOperational,
        allowed_roles: FRONT_DESK_ONLY,
        capability: AgentToolCapability::PmsRead,
    },
];

/// Tool **không** có executor. Model gọi tới thì vòng lặp dừng và trả thẻ xác
/// nhận ra UI. Không thêm hàm thực thi cho bất kỳ tên nào ở đây.
pub const FRONT_DESK_DRAFT_TOOLS: &[AgentToolMeta] = &[AgentToolMeta {
    name: DRAFT_CHECK_IN_TOOL,
    description: "Dựng thẻ xác nhận nhận phòng để người dùng duyệt. Không tự thực hiện.",
    mutation_risk: MutationRisk::HighWrite,
    data_sensitivity: DataSensitivity::StaffOperational,
    allowed_roles: FRONT_DESK_ONLY,
    capability: AgentToolCapability::PmsWrite,
}];

pub fn assistant_tool_schemas() -> Vec<AssistantToolSchema> {
    let mut schemas: Vec<AssistantToolSchema> = FRONT_DESK_READ_TOOLS
        .iter()
        .map(|tool| AssistantToolSchema {
            name: tool.name,
            description: tool.description,
            parameters: read_tool_parameters(tool.name),
        })
        .collect();

    schemas.push(AssistantToolSchema {
        name: DRAFT_CHECK_IN_TOOL,
        description: FRONT_DESK_DRAFT_TOOLS[0].description,
        parameters: json!({
            "type": "object",
            "properties": {
                "room_id": { "type": "string", "description": "Mã phòng trong PMS" },
                "nights": { "type": "integer", "minimum": 1 },
                "guests": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "properties": {
                            "full_name": { "type": "string" },
                            "doc_number": { "type": "string" },
                            "phone": { "type": "string" }
                        },
                        "required": ["full_name"]
                    }
                },
                "paid_amount": { "type": "integer", "minimum": 0, "description": "Số tiền khách trả trước, VND" },
                "source": { "type": "string" },
                "notes": { "type": "string" }
            },
            "required": ["room_id", "nights", "guests"]
        }),
    });

    schemas
}

fn read_tool_parameters(name: &str) -> Value {
    match name {
        "find_guest" => json!({
            "type": "object",
            "properties": { "term": { "type": "string", "description": "Tên hoặc số điện thoại" } },
            "required": ["term"]
        }),
        "get_stay_charges" => json!({
            "type": "object",
            "properties": { "booking_id": { "type": "string" } },
            "required": ["booking_id"]
        }),
        "quote_room_price" => json!({
            "type": "object",
            "properties": {
                "room_id": { "type": "string" },
                "check_in": { "type": "string", "description": "YYYY-MM-DD" },
                "check_out": { "type": "string", "description": "YYYY-MM-DD" },
                "guests": { "type": "integer", "minimum": 1 }
            },
            "required": ["room_id", "check_in", "check_out"]
        }),
        "check_room_availability" => json!({
            "type": "object",
            "properties": {
                "check_in": { "type": "string", "description": "YYYY-MM-DD" },
                "check_out": { "type": "string", "description": "YYYY-MM-DD" }
            },
            "required": ["check_in", "check_out"]
        }),
        _ => json!({ "type": "object", "properties": {} }),
    }
}

/// Chỉ tool đọc mới có nhánh ở đây. Tên nào không khớp — kể cả tên tool ghi —
/// đều bị từ chối. Đây là bảo đảm bằng cấu trúc rằng trợ lý không ghi được.
pub async fn execute_read_tool(
    pool: &Pool<Sqlite>,
    name: &str,
    args: &Value,
) -> CommandResult<Value> {
    match name {
        "list_rooms_now" => {
            let rooms = load_room_status_now(pool).await.map_err(query_failure)?;
            Ok(json!({ "rooms": rooms }))
        }
        "find_guest" => {
            let term = required_str(args, "term")?;
            let matches = find_guest_stays(pool, term).await.map_err(query_failure)?;
            Ok(json!({ "guests": matches }))
        }
        "get_stay_charges" => {
            let booking_id = required_str(args, "booking_id")?;
            let lines = billing_queries::list_folio_lines(pool, booking_id)
                .await
                .map_err(query_failure)?;
            let charged: i64 = lines.iter().map(|line| line.amount).sum();
            Ok(json!({
                "booking_id": booking_id,
                "folio_lines": lines,
                "charged_total_vnd": charged,
            }))
        }
        "quote_room_price" => {
            let room_id = required_str(args, "room_id")?;
            let check_in = required_str(args, "check_in")?;
            let check_out = required_str(args, "check_out")?;
            let guests = args.get("guests").and_then(Value::as_i64).map(|n| n as i32);

            let quote = crate::services::booking::pricing_service::calculate_room_price_preview(
                pool, room_id, check_in, check_out, "nightly", guests,
            )
            .await
            .map_err(|error| {
                CommandError::user(
                    codes::AGENT_PREVIEW_UNAVAILABLE,
                    format!("Không tra được giá: {error}"),
                )
            })?;

            Ok(serde_json::to_value(quote).map_err(|error| {
                CommandError::system(
                    codes::SYSTEM_INTERNAL_ERROR,
                    format!("Không mã hoá được báo giá: {error}"),
                )
            })?)
        }
        "check_room_availability" => {
            let check_in = required_str(args, "check_in")?;
            let check_out = required_str(args, "check_out")?;
            let rooms = load_free_rooms_between(pool, check_in, check_out)
                .await
                .map_err(query_failure)?;
            Ok(json!({ "free_rooms": rooms }))
        }
        "get_today_operations" => {
            let today = chrono::Local::now().format("%Y-%m-%d").to_string();
            let arrivals = ceo_read_queries::list_today_arrivals(pool, &today)
                .await
                .map_err(query_failure)?;
            let checkouts = ceo_read_queries::list_today_checkouts(pool, &today)
                .await
                .map_err(query_failure)?;
            Ok(json!({
                "business_date": today,
                "arrivals": arrivals,
                "checkouts": checkouts,
            }))
        }
        _ => Err(CommandError::user(
            codes::AGENT_TOOL_NOT_ALLOWED,
            "Trợ lý không có công cụ nào cho việc này.",
        )),
    }
}

fn required_str<'a>(args: &'a Value, key: &str) -> CommandResult<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            CommandError::user(
                codes::VALIDATION_INVALID_INPUT,
                format!("Thiếu tham số `{key}`."),
            )
        })
}

fn query_failure(error: sqlx::Error) -> CommandError {
    CommandError::system(
        codes::SYSTEM_INTERNAL_ERROR,
        format!("Truy vấn dữ liệu thất bại: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::model::{AgentRole, DataSensitivity, MutationRisk};

    #[test]
    fn every_read_tool_is_read_only_and_staff_scoped() {
        assert_eq!(FRONT_DESK_READ_TOOLS.len(), 6);

        for tool in FRONT_DESK_READ_TOOLS {
            assert_eq!(tool.mutation_risk, MutationRisk::ReadOnly, "{}", tool.name);
            assert_eq!(
                tool.data_sensitivity,
                DataSensitivity::StaffOperational,
                "{}",
                tool.name
            );
            assert!(
                tool.allowed_for(AgentRole::FrontDeskAssistant),
                "{}",
                tool.name
            );
            assert!(!tool.allowed_for(AgentRole::CeoSecretary), "{}", tool.name);
        }
    }

    #[test]
    fn no_ceo_sensitive_tool_leaks_into_the_front_desk_registry() {
        for tool in FRONT_DESK_READ_TOOLS.iter().chain(FRONT_DESK_DRAFT_TOOLS) {
            assert_ne!(
                tool.data_sensitivity,
                DataSensitivity::CeoSensitive,
                "{} không được mang dữ liệu CEO",
                tool.name
            );
        }
    }

    #[test]
    fn the_schema_list_sent_to_the_model_covers_read_and_draft_tools() {
        let schemas = assistant_tool_schemas();
        let names: Vec<&str> = schemas.iter().map(|schema| schema.name).collect();

        for tool in FRONT_DESK_READ_TOOLS.iter().chain(FRONT_DESK_DRAFT_TOOLS) {
            assert!(names.contains(&tool.name), "thiếu schema cho {}", tool.name);
        }
        assert_eq!(
            schemas.len(),
            FRONT_DESK_READ_TOOLS.len() + FRONT_DESK_DRAFT_TOOLS.len()
        );
    }

    #[tokio::test]
    async fn draft_tools_have_no_executor() {
        let pool = test_pool().await;

        for tool in FRONT_DESK_DRAFT_TOOLS {
            let error = execute_read_tool(&pool, tool.name, &serde_json::json!({}))
                .await
                .expect_err("tool ghi không được có executor");
            assert_eq!(error.code, codes::AGENT_TOOL_NOT_ALLOWED, "{}", tool.name);
        }
    }

    #[tokio::test]
    async fn an_unknown_tool_name_is_rejected() {
        let pool = test_pool().await;

        let error = execute_read_tool(&pool, "rm_minus_rf", &serde_json::json!({}))
            .await
            .expect_err("tên lạ phải bị từ chối");

        assert_eq!(error.code, codes::AGENT_TOOL_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn list_rooms_now_returns_every_room_on_an_empty_database() {
        let pool = test_pool().await;

        let output = execute_read_tool(&pool, "list_rooms_now", &serde_json::json!({}))
            .await
            .expect("tool phải chạy được");

        assert!(output["rooms"].is_array());
    }

    #[tokio::test]
    async fn find_guest_requires_a_search_term() {
        let pool = test_pool().await;

        let error = execute_read_tool(&pool, "find_guest", &serde_json::json!({}))
            .await
            .expect_err("thiếu term phải báo lỗi");

        assert_eq!(error.code, codes::VALIDATION_INVALID_INPUT);
    }

    async fn test_pool() -> sqlx::Pool<sqlx::Sqlite> {
        use sqlx::sqlite::SqlitePoolOptions;

        let database_url = format!(
            "sqlite://file:{}?mode=memory&cache=shared",
            uuid::Uuid::new_v4()
        );
        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .expect("failed to open sqlite test pool");
        crate::db::run_migrations(&pool)
            .await
            .expect("failed to run migrations");
        pool
    }
}
