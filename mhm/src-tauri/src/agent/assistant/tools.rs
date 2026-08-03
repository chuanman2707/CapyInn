use crate::{
    agent::{
        assistant::provider::AssistantToolSchema,
        model::{AgentRole, AgentToolCapability, AgentToolMeta, DataSensitivity, MutationRisk},
    },
    app_error::{codes, CommandError, CommandResult},
    queries::{
        booking::{billing_queries, ceo_read_queries},
        rooms::assistant_queries::{
            find_guest_stays, load_free_rooms_between, load_room_status_now, load_stay_charges,
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
            let stay = load_stay_charges(pool, booking_id)
                .await
                .map_err(query_failure)?
                .ok_or_else(|| {
                    CommandError::user(
                        codes::VALIDATION_INVALID_INPUT,
                        format!("Không tìm thấy lượt ở với mã `{booking_id}`."),
                    )
                })?;
            let lines = billing_queries::list_folio_lines(pool, &stay.booking_id)
                .await
                .map_err(query_failure)?;
            let charged_total_vnd: i64 = lines.iter().map(|line| line.amount).sum();
            let outstanding_vnd = stay.total_price_vnd - stay.paid_amount_vnd;
            Ok(json!({
                "booking_id": stay.booking_id,
                "guest_name": stay.guest_name,
                "room_name": stay.room_name,
                "total_price_vnd": stay.total_price_vnd,
                "paid_amount_vnd": stay.paid_amount_vnd,
                "outstanding_vnd": outstanding_vnd,
                "charged_total_vnd": charged_total_vnd,
                "folio_lines": lines,
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

    // ─── Happy-path coverage for the hand-written SQL in `assistant_queries.rs` ───
    //
    // Every query below only ever ran against an empty database or not at all.
    // These tests seed real rows so a mistyped column or a broken join fails
    // the test instead of failing in front of a guest.

    #[tokio::test]
    async fn list_rooms_now_shows_the_guest_on_an_occupied_room_and_leaves_a_vacant_one_empty() {
        let pool = test_pool().await;

        seed_room(
            &pool,
            "room-occ",
            "P101",
            "Standard Room",
            300_000,
            "occupied",
        )
        .await;
        seed_room(
            &pool,
            "room-vac",
            "P102",
            "Deluxe Balcony",
            500_000,
            "vacant",
        )
        .await;
        seed_guest(&pool, "guest-occ", "Nguyễn Văn A", None).await;
        seed_booking(
            &pool,
            "book-occ",
            "room-occ",
            "guest-occ",
            "active",
            "2026-05-01",
            "2026-05-02",
            1,
            300_000,
            0,
        )
        .await;

        let output = execute_read_tool(&pool, "list_rooms_now", &serde_json::json!({}))
            .await
            .expect("tool phải chạy được");

        let rooms = output["rooms"].as_array().expect("rooms phải là mảng");
        assert_eq!(rooms.len(), 2);

        let occupied = find_by(&output["rooms"], "room_id", "room-occ");
        assert_eq!(occupied["guest_name"], json!("Nguyễn Văn A"));
        assert_eq!(occupied["booking_id"], json!("book-occ"));
        assert_eq!(occupied["room_type"], json!("Standard Room"));
        assert_eq!(occupied["status"], json!("occupied"));

        let vacant = find_by(&output["rooms"], "room_id", "room-vac");
        assert!(vacant["guest_name"].is_null(), "{vacant}");
        assert!(vacant["booking_id"].is_null(), "{vacant}");
        assert_eq!(vacant["room_type"], json!("Deluxe Balcony"));
        assert_eq!(vacant["status"], json!("vacant"));
    }

    #[tokio::test]
    async fn find_guest_matches_by_partial_name_or_phone_and_reports_the_open_stay() {
        let pool = test_pool().await;

        seed_room(
            &pool,
            "room-fg",
            "P201",
            "Standard Room",
            300_000,
            "occupied",
        )
        .await;
        seed_guest(&pool, "guest-fg", "Trần Thị Bích", Some("0909123456")).await;
        seed_booking(
            &pool,
            "book-fg",
            "room-fg",
            "guest-fg",
            "active",
            "2026-05-01",
            "2026-05-03",
            2,
            600_000,
            200_000,
        )
        .await;
        seed_guest(&pool, "guest-fg-no-stay", "Trần Văn Không", None).await;

        let output = execute_read_tool(&pool, "find_guest", &serde_json::json!({ "term": "Trần" }))
            .await
            .expect("tool phải chạy được");

        let guests = output["guests"].as_array().expect("guests phải là mảng");
        assert_eq!(guests.len(), 2, "{guests:?}");

        let with_stay = find_by(&output["guests"], "guest_id", "guest-fg");
        assert_eq!(with_stay["booking_id"], json!("book-fg"));
        assert_eq!(with_stay["room_name"], json!("P201"));

        let without_stay = find_by(&output["guests"], "guest_id", "guest-fg-no-stay");
        assert!(without_stay["booking_id"].is_null(), "{without_stay}");
        assert!(without_stay["room_name"].is_null(), "{without_stay}");

        let by_phone = execute_read_tool(
            &pool,
            "find_guest",
            &serde_json::json!({ "term": "0909123456" }),
        )
        .await
        .expect("tool phải chạy được");
        let phone_matches = by_phone["guests"].as_array().expect("guests phải là mảng");
        assert_eq!(phone_matches.len(), 1, "{phone_matches:?}");
        assert_eq!(phone_matches[0]["guest_id"], json!("guest-fg"));
    }

    #[tokio::test]
    async fn get_stay_charges_returns_total_paid_outstanding_and_charged_total() {
        let pool = test_pool().await;

        seed_room(
            &pool,
            "room-gsc",
            "P301",
            "Deluxe Balcony",
            600_000,
            "occupied",
        )
        .await;
        seed_guest(&pool, "guest-gsc", "Lê Văn Cường", None).await;
        seed_booking(
            &pool,
            "book-gsc",
            "room-gsc",
            "guest-gsc",
            "active",
            "2026-05-01",
            "2026-05-03",
            2,
            1_200_000,
            500_000,
        )
        .await;
        seed_folio_line(&pool, "folio-1", "book-gsc", 150_000).await;
        seed_folio_line(&pool, "folio-2", "book-gsc", 50_000).await;

        let output = execute_read_tool(
            &pool,
            "get_stay_charges",
            &serde_json::json!({ "booking_id": "book-gsc" }),
        )
        .await
        .expect("tool phải chạy được");

        assert_eq!(output["booking_id"], json!("book-gsc"));
        assert_eq!(output["guest_name"], json!("Lê Văn Cường"));
        assert_eq!(output["room_name"], json!("P301"));
        assert_eq!(output["total_price_vnd"], json!(1_200_000));
        assert_eq!(output["paid_amount_vnd"], json!(500_000));
        assert_eq!(output["outstanding_vnd"], json!(700_000));
        assert_eq!(output["charged_total_vnd"], json!(200_000));
        assert_eq!(
            output["folio_lines"]
                .as_array()
                .expect("folio_lines phải là mảng")
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn get_stay_charges_rejects_an_unknown_booking_id() {
        let pool = test_pool().await;

        let error = execute_read_tool(
            &pool,
            "get_stay_charges",
            &serde_json::json!({ "booking_id": "does-not-exist" }),
        )
        .await
        .expect_err("mã lượt ở không tồn tại phải báo lỗi");

        assert_eq!(error.code, codes::VALIDATION_INVALID_INPUT);
        assert!(
            error.message.contains("does-not-exist"),
            "thông báo lỗi phải nêu rõ mã không tìm thấy: {}",
            error.message
        );
    }

    #[tokio::test]
    async fn quote_room_price_prices_a_seeded_room_over_two_weekday_nights() {
        let pool = test_pool().await;

        seed_room(&pool, "room-qp", "P401", "Standard Room", 300_000, "vacant").await;

        // 2026-06-01 is a Monday and 2026-06-03 a Wednesday, so this stay has
        // no Saturday/Sunday night in it — the 20% weekend uplift that a room
        // with no `pricing_rules` row falls back to must stay at zero, and the
        // total must come out to exactly 2 nights of `base_price`.
        let output = execute_read_tool(
            &pool,
            "quote_room_price",
            &serde_json::json!({
                "room_id": "room-qp",
                "check_in": "2026-06-01",
                "check_out": "2026-06-03"
            }),
        )
        .await
        .expect("tool phải chạy được");

        assert_eq!(output["pricing_type"], json!("nightly"));
        assert_eq!(output["base_amount"], json!(600_000));
        assert_eq!(output["weekend_amount"], json!(0));
        assert_eq!(output["total"], json!(600_000));
    }

    #[tokio::test]
    async fn check_room_availability_excludes_a_booked_room_and_keeps_a_free_one() {
        let pool = test_pool().await;

        seed_room(
            &pool,
            "room-free",
            "P501",
            "Deluxe Balcony",
            400_000,
            "vacant",
        )
        .await;
        seed_room(
            &pool,
            "room-busy",
            "P502",
            "Standard Room",
            300_000,
            "vacant",
        )
        .await;
        seed_guest(&pool, "guest-avail", "Phạm Thị Dung", None).await;
        seed_booking(
            &pool,
            "book-busy",
            "room-busy",
            "guest-avail",
            "active",
            "2026-06-10",
            "2026-06-12",
            2,
            600_000,
            0,
        )
        .await;
        seed_room_calendar_day(&pool, "room-busy", "2026-06-10", "book-busy", "occupied").await;
        seed_room_calendar_day(&pool, "room-busy", "2026-06-11", "book-busy", "occupied").await;

        let output = execute_read_tool(
            &pool,
            "check_room_availability",
            &serde_json::json!({ "check_in": "2026-06-10", "check_out": "2026-06-12" }),
        )
        .await
        .expect("tool phải chạy được");

        let free_rooms = output["free_rooms"]
            .as_array()
            .expect("free_rooms phải là mảng");
        let free_ids: Vec<&str> = free_rooms
            .iter()
            .map(|room| room["room_id"].as_str().expect("room_id"))
            .collect();

        assert_eq!(free_rooms.len(), 1, "{free_ids:?}");
        assert!(free_ids.contains(&"room-free"), "{free_ids:?}");
        assert!(!free_ids.contains(&"room-busy"), "{free_ids:?}");
    }

    #[tokio::test]
    async fn get_today_operations_lists_todays_arrival_and_todays_checkout() {
        let pool = test_pool().await;
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let yesterday = (chrono::Local::now() - chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let tomorrow = (chrono::Local::now() + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();

        seed_room(
            &pool,
            "room-arr",
            "P601",
            "Standard Room",
            300_000,
            "vacant",
        )
        .await;
        seed_room(
            &pool,
            "room-out",
            "P602",
            "Deluxe Balcony",
            500_000,
            "occupied",
        )
        .await;
        seed_guest(&pool, "guest-arr", "Đỗ Thị Hoa", None).await;
        seed_guest(&pool, "guest-out", "Vũ Văn Em", None).await;

        seed_booking(
            &pool,
            "book-arr",
            "room-arr",
            "guest-arr",
            "booked",
            &today,
            &tomorrow,
            1,
            300_000,
            100_000,
        )
        .await;
        seed_booking(
            &pool,
            "book-out",
            "room-out",
            "guest-out",
            "active",
            &yesterday,
            &today,
            1,
            500_000,
            500_000,
        )
        .await;

        let output = execute_read_tool(&pool, "get_today_operations", &serde_json::json!({}))
            .await
            .expect("tool phải chạy được");

        assert_eq!(output["business_date"], json!(today));

        let arrivals = output["arrivals"]
            .as_array()
            .expect("arrivals phải là mảng");
        assert_eq!(arrivals.len(), 1, "{arrivals:?}");
        assert_eq!(arrivals[0]["booking_id"], json!("book-arr"));
        assert_eq!(arrivals[0]["guest_name"], json!("Đỗ Thị Hoa"));

        let checkouts = output["checkouts"]
            .as_array()
            .expect("checkouts phải là mảng");
        assert_eq!(checkouts.len(), 1, "{checkouts:?}");
        assert_eq!(checkouts[0]["booking_id"], json!("book-out"));
        assert_eq!(checkouts[0]["guest_name"], json!("Vũ Văn Em"));
    }

    /// Tìm phần tử trong một mảng JSON theo giá trị một field — tránh phụ
    /// thuộc vào thứ tự `ORDER BY` của SQL khi assert.
    fn find_by<'a>(array: &'a Value, field: &str, needle: &str) -> &'a Value {
        array
            .as_array()
            .expect("kết quả phải là mảng")
            .iter()
            .find(|entry| entry[field] == json!(needle))
            .unwrap_or_else(|| panic!("không có phần tử nào có {field} = {needle}"))
    }

    async fn seed_room(
        pool: &Pool<Sqlite>,
        id: &str,
        name: &str,
        room_type: &str,
        base_price: i64,
        status: &str,
    ) {
        sqlx::query(
            "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price, status)
             VALUES (?, ?, ?, 1, 0, ?, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(room_type)
        .bind(base_price)
        .bind(status)
        .execute(pool)
        .await
        .expect("seed room");
    }

    async fn seed_guest(pool: &Pool<Sqlite>, id: &str, full_name: &str, phone: Option<&str>) {
        sqlx::query(
            "INSERT INTO guests (id, guest_type, full_name, doc_number, phone, created_at)
             VALUES (?, 'domestic', ?, ?, ?, '2026-05-01T08:00:00+07:00')",
        )
        .bind(id)
        .bind(full_name)
        .bind(format!("DOC-{id}"))
        .bind(phone)
        .execute(pool)
        .await
        .expect("seed guest");
    }

    #[allow(clippy::too_many_arguments)]
    async fn seed_booking(
        pool: &Pool<Sqlite>,
        id: &str,
        room_id: &str,
        guest_id: &str,
        status: &str,
        check_in_at: &str,
        expected_checkout: &str,
        nights: i64,
        total_price: i64,
        paid_amount: i64,
    ) {
        sqlx::query(
            "INSERT INTO bookings (
                id, room_id, primary_guest_id, check_in_at, expected_checkout,
                nights, total_price, paid_amount, status, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(room_id)
        .bind(guest_id)
        .bind(check_in_at)
        .bind(expected_checkout)
        .bind(nights)
        .bind(total_price)
        .bind(paid_amount)
        .bind(status)
        .bind(check_in_at)
        .execute(pool)
        .await
        .expect("seed booking");
    }

    async fn seed_room_calendar_day(
        pool: &Pool<Sqlite>,
        room_id: &str,
        date: &str,
        booking_id: &str,
        status: &str,
    ) {
        sqlx::query(
            "INSERT INTO room_calendar (room_id, date, booking_id, status) VALUES (?, ?, ?, ?)",
        )
        .bind(room_id)
        .bind(date)
        .bind(booking_id)
        .bind(status)
        .execute(pool)
        .await
        .expect("seed room_calendar");
    }

    async fn seed_folio_line(pool: &Pool<Sqlite>, id: &str, booking_id: &str, amount: i64) {
        sqlx::query(
            "INSERT INTO folio_lines (id, booking_id, category, description, amount, created_by, created_at)
             VALUES (?, ?, 'mini-bar', 'Seed folio', ?, 'seed-user', '2026-05-01T09:00:00+07:00')",
        )
        .bind(id)
        .bind(booking_id)
        .bind(amount)
        .execute(pool)
        .await
        .expect("seed folio line");
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
