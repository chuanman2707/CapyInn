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
pub const DRAFT_RESERVE_TOOL: &str = "draft_reserve";
pub const DRAFT_BACKFILL_TOOL: &str = "draft_backfill";

/// Loại thẻ mà một tool `draft_*` dựng ra.
///
/// Tồn tại để vòng lặp trong `mod.rs` chọn hàm dựng thẻ bằng một `match` **vét
/// cạn trên enum**, không phải bằng chuỗi kèm một nhánh `_ =>` đoán bừa. Thêm
/// một tool ghi mà quên nối vào vòng lặp thì `every_draft_tool_maps_to_a_builder`
/// đỏ ngay ở đây, chứ không đợi tới lúc lễ tân bấm *Đồng ý* và không có gì xảy
/// ra.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftToolKind {
    CheckIn,
    Reserve,
    Backfill,
}

/// `None` ⇒ tên này **không** phải tool dựng thẻ. Không có nhánh mặc định nào
/// rơi về `CheckIn`: đoán nhầm ở đây là bắn một thẻ đặt phòng qua đường nhận
/// phòng, tức đúng con bug đã mở ra cả spec này.
pub fn draft_tool_kind(name: &str) -> Option<DraftToolKind> {
    match name {
        DRAFT_CHECK_IN_TOOL => Some(DraftToolKind::CheckIn),
        DRAFT_RESERVE_TOOL => Some(DraftToolKind::Reserve),
        DRAFT_BACKFILL_TOOL => Some(DraftToolKind::Backfill),
        _ => None,
    }
}

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
        description: "Báo giá một phòng cho một khoảng ngày cụ thể, đúng số quầy thu.",
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
pub const FRONT_DESK_DRAFT_TOOLS: &[AgentToolMeta] = &[
    AgentToolMeta {
        name: DRAFT_CHECK_IN_TOOL,
        description:
            "Dựng thẻ xác nhận nhận phòng cho khách ĐANG ĐỨNG Ở QUẦY HÔM NAY, để người dùng \
                  duyệt. Không tự thực hiện. LUÔN điền `check_in_date` bằng đúng ngày người dùng \
                  nêu, nếu họ có nêu ngày — kể cả khi ngày đó không phải hôm nay. Không bao giờ tự \
                  thay ngày người dùng nêu bằng hôm nay, và không bỏ trống ô ngày để né.",
        mutation_risk: MutationRisk::HighWrite,
        data_sensitivity: DataSensitivity::StaffOperational,
        allowed_roles: FRONT_DESK_ONLY,
        capability: AgentToolCapability::PmsWrite,
    },
    AgentToolMeta {
        name: DRAFT_RESERVE_TOOL,
        description:
            "Dựng thẻ xác nhận ĐẶT PHÒNG TRƯỚC cho một ngày nhận phòng Ở TƯƠNG LAI (khách chưa \
                  tới), để người dùng duyệt. Không tự thực hiện. Đây là đích của `draft_check_in` \
                  khi ngày người dùng nêu chưa tới: LUÔN điền `check_in_date` và `check_out_date` \
                  dạng YYYY-MM-DD bằng đúng hai ngày người dùng nêu. KHÔNG có tham số số đêm — số \
                  đêm do CapyInn tính từ hai ngày đó. Chỉ ghi được MỘT tên khách: người dùng nêu \
                  nhiều khách thì vẫn điền một tên chính vào `guest_name` và NÓI RÕ với người dùng \
                  rằng đặt phòng trước chỉ ghi được một tên.",
        mutation_risk: MutationRisk::HighWrite,
        data_sensitivity: DataSensitivity::StaffOperational,
        allowed_roles: FRONT_DESK_ONLY,
        capability: AgentToolCapability::PmsWrite,
    },
    AgentToolMeta {
        name: DRAFT_BACKFILL_TOOL,
        description:
            "Dựng thẻ xác nhận GHI BÙ một lượt khách đã vào phòng từ một ngày Ở QUÁ KHỨ nhưng \
                  chưa được nhập máy, để người dùng duyệt. Không tự thực hiện. Đây là đích của \
                  `draft_check_in` khi ngày người dùng nêu đã qua: LUÔN điền `check_in_date` dạng \
                  YYYY-MM-DD bằng đúng ngày người dùng nêu. Khách ĐÃ trả phòng thì điền \
                  `check_out_date`; khách CÒN Ở thì bỏ trống `check_out_date` và BẮT BUỘC điền \
                  `expected_checkout_date` (phải sau hôm nay). KHÔNG có tham số tiền phòng — \
                  CapyInn tự tra bảng giá cho khoảng ngày đó; tuyệt đối không tự nêu ra một con \
                  số tiền phòng.",
        mutation_risk: MutationRisk::HighWrite,
        data_sensitivity: DataSensitivity::StaffOperational,
        allowed_roles: FRONT_DESK_ONLY,
        capability: AgentToolCapability::PmsWrite,
    },
];

pub fn assistant_tool_schemas() -> Vec<AssistantToolSchema> {
    let mut schemas: Vec<AssistantToolSchema> = FRONT_DESK_READ_TOOLS
        .iter()
        .map(|tool| AssistantToolSchema {
            name: tool.name,
            description: tool.description,
            parameters: read_tool_parameters(tool.name),
        })
        .collect();

    schemas.extend(
        FRONT_DESK_DRAFT_TOOLS
            .iter()
            .map(|tool| AssistantToolSchema {
                name: tool.name,
                description: tool.description,
                parameters: draft_tool_parameters(tool.name),
            }),
    );

    schemas
}

/// Schema JSON của một tool dựng thẻ.
///
/// Tách khỏi `read_tool_parameters` chứ không gộp: hai họ tool này có luật
/// khác hẳn nhau, và `every_draft_tool_offers_the_model_a_real_schema` canh
/// riêng họ này để một tên tool mới không lặng lẽ nhận `{}` rỗng.
fn draft_tool_parameters(name: &str) -> Value {
    match name {
        DRAFT_CHECK_IN_TOOL => json!({
            "type": "object",
            "properties": {
                "room_id": { "type": "string", "description": "Mã phòng trong PMS" },
                "nights": { "type": "integer", "minimum": 1 },
                // Cái ô này **là** bản vá của con bug đã xảy ra ngày 06/08/2026.
                // Model nghe "checkin 8 out 9 tháng 8" nhưng không có trường nào
                // để đặt số 8 vào, nên nó bỏ đi và gọi tool với phần còn lại;
                // thẻ dựng ra ghi ngày hôm nay, phòng bị khoá sớm hai ngày và
                // một đêm chưa xảy ra bị tính tiền. Có ô rồi thì `draft.rs` mới
                // soi được — không có ô thì luật "không thay ngày người dùng
                // nêu" chỉ là một câu trong system prompt, và prompt không phải
                // hàng rào.
                //
                // **Tuỳ chọn**, cố ý: bắt buộc thì mọi lượt nhận phòng bình
                // thường (khách đứng ở quầy, không ai nêu ngày) phải qua một
                // vòng `missing_fields` thừa.
                "check_in_date": {
                    "type": "string",
                    "description": "Ngày nhận phòng người dùng nêu, dạng YYYY-MM-DD. LUÔN điền nếu người dùng có nêu ngày, kể cả khi ngày đó KHÔNG phải hôm nay. Không tự đổi thành hôm nay, không bỏ trống để né. Chỉ bỏ trống khi người dùng hoàn toàn không nêu ngày nào."
                },
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
        DRAFT_RESERVE_TOOL => json!({
            "type": "object",
            "properties": {
                "room_id": { "type": "string", "description": "Mã phòng trong PMS" },
                // MỘT tên, kiểu chuỗi — khác `draft_check_in` (nhận cả danh
                // sách khách). `create_reservation` chỉ lưu một khách chính,
                // nên một ô `array` ở đây là lời hứa mà lệnh phía sau không
                // giữ được: model điền ba tên rồi hai tên biến mất không ai
                // báo. Ô là chuỗi thì giới hạn ấy nhìn thấy được, và
                // `build_reserve_draft` còn cảnh báo lên thẻ khi ô này trông
                // như đang cõng nhiều tên.
                "guest_name": {
                    "type": "string",
                    "description": "Họ tên MỘT khách chính. Đặt phòng trước chỉ ghi được một tên; nếu người dùng nêu nhiều khách, điền tên khách chính vào đây rồi nói rõ với người dùng rằng chỉ tên đó được ghi."
                },
                "guest_phone": { "type": "string" },
                "guest_doc_number": { "type": "string", "description": "Số giấy tờ (CCCD/hộ chiếu) nếu người dùng có nêu" },
                "check_in_date": {
                    "type": "string",
                    "description": "Ngày nhận phòng, dạng YYYY-MM-DD. BẮT BUỘC, và phải ở tương lai — hôm nay thì dùng `draft_check_in`, quá khứ thì dùng `draft_backfill`. Điền đúng ngày người dùng nêu, không tự đổi."
                },
                "check_out_date": {
                    "type": "string",
                    "description": "Ngày trả phòng, dạng YYYY-MM-DD. BẮT BUỘC, và phải sau `check_in_date`. Điền đúng ngày người dùng nêu, không suy ra từ số đêm bạn tự đoán."
                },
                "deposit_amount": { "type": "integer", "minimum": 0, "description": "Tiền đặt cọc khách đã trả, VND" },
                "notes": { "type": "string" }
            },
            // KHÔNG có `nights`, cố ý. Model đưa cả hai ngày lẫn số đêm thì ba
            // con số có thể mâu thuẫn và không có cách nào biết vế nào đúng;
            // hai ngày là nguồn sự thật, số đêm dẫn xuất trong
            // `build_reserve_draft`. `create_reservation_tx` cũng tự kiểm lại
            // (`validate_requested_nights`) nên một số đêm lệch sẽ bị lệnh từ
            // chối — nhưng thà không có ô còn hơn có ô rồi phải canh.
            //
            // Cũng KHÔNG có `source`: `create_reservation_tx` mặc định
            // "phone", và một ô nữa cho model điền là một ô nữa để nó bịa.
            "required": ["room_id", "guest_name", "check_in_date", "check_out_date"]
        }),
        DRAFT_BACKFILL_TOOL => json!({
            "type": "object",
            "properties": {
                "room_id": { "type": "string", "description": "Mã phòng trong PMS" },
                // DANH SÁCH khách, giống `draft_check_in` — khác `draft_reserve`
                // (một tên, kiểu chuỗi). `backfill_stay` ghi trọn một lượt ở
                // thật, gồm cả hồ sơ từng khách, nên nó nhận đủ danh sách.
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
                "check_in_date": {
                    "type": "string",
                    "description": "Ngày khách vào phòng, dạng YYYY-MM-DD. BẮT BUỘC, và phải ở QUÁ KHỨ — hôm nay thì dùng `draft_check_in`, tương lai thì `draft_reserve`. Điền đúng ngày người dùng nêu, không tự đổi."
                },
                "check_out_date": {
                    "type": "string",
                    "description": "Ngày khách ĐÃ trả phòng, dạng YYYY-MM-DD. Chỉ điền khi khách đã rời phòng, và ngày đó phải sau `check_in_date` và không ở tương lai. BỎ TRỐNG nếu khách vẫn còn đang ở."
                },
                "expected_checkout_date": {
                    "type": "string",
                    "description": "Ngày trả phòng dự kiến, dạng YYYY-MM-DD. BẮT BUỘC khi khách CÒN Ở (tức khi `check_out_date` bỏ trống), và phải sau hôm nay. Không đoán: người dùng chưa nói thì hỏi lại."
                },
                "paid_amount": { "type": "integer", "minimum": 0, "description": "Số tiền khách ĐÃ trả cho lượt ở này, VND. Bỏ trống nghĩa là chưa trả đồng nào." },
                "notes": { "type": "string" }
            },
            // KHÔNG có `total_price`, cố ý — và đây là ô nguy hiểm nhất trong cả
            // ba tool. Khác `check_in`/`create_reservation` (tự tính tiền),
            // `backfill_stay` BẮT người gọi đưa tiền phòng vào, nên một ô ở đây
            // là mời một mô hình ngôn ngữ quyết định khách nợ bao nhiêu.
            // `build_backfill_draft` lấy số ấy từ `calculate_room_price_preview`
            // và không đọc tham số nào của model cho nó.
            //
            // Cũng KHÔNG có `source`: `backfill_stay_tx` mặc định "walk-in", và
            // một ô nữa cho model điền là một ô nữa để nó bịa.
            //
            // `expected_checkout_date` bắt buộc **có điều kiện** (chỉ khi khách
            // còn ở) nên nó không nằm trong `required` — JSON schema nói được
            // câu ấy bằng `if/then`, nhưng không nhà cung cấp nào bảo đảm tôn
            // trọng, và `build_backfill_draft` đằng nào cũng phải canh lại. Một
            // ô `required` mà nửa số ca hợp lệ bỏ trống là một ô dạy model bịa.
            "required": ["room_id", "guests", "check_in_date"]
        }),
        // Tên lạ: schema rỗng. Không đoán schema của một tool không biết —
        // `every_draft_tool_offers_the_model_a_real_schema` bắt mọi tool có
        // thật phải có nhánh ở trên, nên nhánh này chỉ với tới được bằng một
        // lời gọi tay.
        _ => json!({ "type": "object", "properties": {} }),
    }
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
        // Không có `guests`: giá quầy thu không phụ thuộc số khách (xem nhánh
        // `quote_room_price` trong `execute_read_tool`). Một tham số nằm trong
        // schema mà không đổi được kết quả là lời mời model tin rằng nó vừa
        // hỏi giá cho đúng số khách đó, rồi diễn giải con số theo cách sai.
        "quote_room_price" => json!({
            "type": "object",
            "properties": {
                "room_id": { "type": "string" },
                "check_in": { "type": "string", "description": "YYYY-MM-DD" },
                "check_out": { "type": "string", "description": "YYYY-MM-DD" }
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

            // `None` cho số khách, cùng lý do với `draft::build_check_in_draft`:
            // `stay_lifecycle::check_in` truyền `None`, nên phụ thu thêm người
            // không nằm trong số quầy thu. Trợ lý đọc con số này thành lời
            // trong khung chat, nên nó phải là số thu được, không phải số cao
            // hơn. `guests` cũng đã bị gỡ khỏi schema — model không còn được
            // mời gửi tham số này nữa; nếu có gửi thì cứ bỏ qua, đừng báo lỗi.
            let quote = crate::services::booking::pricing_service::calculate_room_price_preview(
                pool, room_id, check_in, check_out, "nightly", None,
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

    /// Con bug ngày 06/08/2026 không phải một dòng code sai: model nghe thấy
    /// "checkin 8 out 9 tháng 8" mà **không có ô nào** để đặt con số ấy vào,
    /// nên nó bỏ đi và gọi tool với phần còn lại.
    ///
    /// Cái ô này chính là bản vá. Gỡ nó khỏi schema thì nhánh từ chối trong
    /// `draft.rs` không bao giờ được gọi tới nữa — bug quay lại y nguyên trong
    /// khi mọi test của `draft.rs` vẫn xanh, vì chúng gọi thẳng
    /// `build_check_in_draft` chứ không đi qua schema. Test này là chỗ duy nhất
    /// canh mối nối đó.
    #[test]
    fn the_draft_check_in_tool_gives_the_model_a_slot_for_the_date() {
        let schemas = assistant_tool_schemas();
        let draft = schemas
            .iter()
            .find(|schema| schema.name == DRAFT_CHECK_IN_TOOL)
            .expect("phải có schema cho draft_check_in");

        let field = draft.parameters["properties"]
            .get("check_in_date")
            .unwrap_or_else(|| {
                panic!(
                    "schema thiếu ô `check_in_date`; model không có chỗ đặt ngày người dùng nêu: {}",
                    draft.parameters
                )
            });
        assert_eq!(field["type"], json!("string"), "{field}");

        let field_description = field["description"].as_str().unwrap_or_default();
        assert!(
            field_description.contains("YYYY-MM-DD"),
            "mô tả ô ngày phải nêu định dạng: {field_description}"
        );
        assert!(
            field_description.contains("LUÔN"),
            "mô tả ô ngày phải bảo model LUÔN điền khi người dùng có nêu ngày: {field_description}"
        );

        // **Tuỳ chọn**, không nằm trong `required`: bắt buộc thì mọi lượt nhận
        // phòng bình thường phải qua một vòng `missing_fields` thừa.
        let required = draft.parameters["required"]
            .as_array()
            .expect("schema phải có danh sách `required`");
        assert!(
            !required.contains(&json!("check_in_date")),
            "ô ngày phải là tuỳ chọn: {required:?}"
        );

        // Mô tả tool là chỗ model đọc trước khi quyết định gọi gì. Nó phải nói
        // ra luật, không chỉ tên trường.
        assert!(
            draft.description.contains("check_in_date") && draft.description.contains("LUÔN điền"),
            "mô tả tool phải yêu cầu LUÔN điền ngày người dùng nêu: {}",
            draft.description
        );
    }

    /// Bài học đã trả giá ở Task 1, lặp lại nguyên xi cho `draft_reserve`: mọi
    /// test của `build_reserve_draft` đều gọi **thẳng** hàm ấy, không đi qua
    /// schema. Gỡ `check_in_date`/`check_out_date` khỏi schema thì model không
    /// còn ô nào để đặt ngày vào — bug cũ quay lại y nguyên — mà cả tá test
    /// trong `draft.rs` vẫn xanh. Test này là chỗ **duy nhất** canh mối nối đó.
    #[test]
    fn the_draft_reserve_tool_requires_both_stay_dates_and_offers_no_night_count() {
        let schemas = assistant_tool_schemas();
        let draft = schemas
            .iter()
            .find(|schema| schema.name == DRAFT_RESERVE_TOOL)
            .expect("phải có schema cho draft_reserve");

        let properties = draft.parameters["properties"]
            .as_object()
            .expect("schema phải có `properties`");
        let required: Vec<&str> = draft.parameters["required"]
            .as_array()
            .expect("schema phải có danh sách `required`")
            .iter()
            .filter_map(Value::as_str)
            .collect();

        // Bốn ô bắt buộc. Thiếu một ô trong `properties` thì model không có chỗ
        // đặt giá trị; thiếu trong `required` thì nó được mời bỏ trống.
        for field in ["room_id", "guest_name", "check_in_date", "check_out_date"] {
            assert!(
                properties.contains_key(field),
                "schema thiếu ô `{field}`: {}",
                draft.parameters
            );
            assert!(
                required.contains(&field),
                "`{field}` phải nằm trong `required`: {required:?}"
            );
        }

        // Hai ô ngày phải nói ra định dạng, không thì model gửi "ngày 8" và
        // `build_reserve_draft` chỉ còn cách từ chối.
        for field in ["check_in_date", "check_out_date"] {
            let description = properties[field]["description"]
                .as_str()
                .unwrap_or_default();
            assert_eq!(properties[field]["type"], json!("string"), "{field}");
            assert!(
                description.contains("YYYY-MM-DD"),
                "mô tả `{field}` phải nêu định dạng: {description}"
            );
        }

        // **`nights` KHÔNG được có mặt.** Hai ngày là nguồn sự thật; một ô số
        // đêm nữa là một con số thứ ba để mâu thuẫn với chúng, và không có cách
        // nào biết vế nào đúng.
        assert!(
            properties.get("nights").is_none(),
            "schema vẫn mời model tự đưa số đêm: {}",
            draft.parameters
        );

        // Bốn ô tuỳ chọn có thật, nhưng KHÔNG nằm trong `required` — bắt buộc
        // chúng là biến mọi lượt đặt phòng bình thường thành một vòng
        // `missing_fields` thừa trong ngân sách 4 vòng.
        for field in ["guest_phone", "guest_doc_number", "deposit_amount", "notes"] {
            assert!(
                properties.contains_key(field),
                "schema thiếu ô tuỳ chọn `{field}`: {}",
                draft.parameters
            );
            assert!(
                !required.contains(&field),
                "`{field}` phải là tuỳ chọn: {required:?}"
            );
        }

        // Mô tả tool là chỗ model đọc TRƯỚC khi quyết định gọi gì — nó phải nói
        // ra luật một-tên, vì đó là giới hạn duy nhất của tool này mà schema
        // không tự nói được.
        assert!(
            draft.description.contains("MỘT tên khách"),
            "mô tả tool phải nói rõ chỉ ghi được một tên: {}",
            draft.description
        );
    }

    /// Cùng cái seam Task 1 và Task 3 đã trả giá để tìm ra, lần thứ ba: mọi test
    /// của `build_backfill_draft` gọi **thẳng** hàm ấy, không đi qua schema. Gỡ
    /// `check_in_date` khỏi schema thì model không còn ô nào để đặt ngày vào —
    /// bug cũ quay lại y nguyên — mà cả tá test trong `draft.rs` vẫn xanh.
    ///
    /// Và ở đây có thêm một thứ **không** tool nào khác phải canh: schema này
    /// tuyệt đối không được có ô tiền phòng.
    #[test]
    fn the_draft_backfill_tool_asks_for_dates_and_never_for_a_price() {
        let schemas = assistant_tool_schemas();
        let draft = schemas
            .iter()
            .find(|schema| schema.name == DRAFT_BACKFILL_TOOL)
            .expect("phải có schema cho draft_backfill");

        let properties = draft.parameters["properties"]
            .as_object()
            .expect("schema phải có `properties`");
        let required: Vec<&str> = draft.parameters["required"]
            .as_array()
            .expect("schema phải có danh sách `required`")
            .iter()
            .filter_map(Value::as_str)
            .collect();

        // ─── ĐIỀU CẤM SỐ MỘT CỦA TOOL NÀY ───
        //
        // `backfill_stay` là lệnh ghi DUY NHẤT bắt người gọi đưa tiền phòng vào.
        // Một ô ở đây là mời một mô hình ngôn ngữ quyết định khách nợ bao nhiêu,
        // và `BackfillStayRequest.total_price` sẵn sàng nhận con số ấy — kiểu dữ
        // liệu mời làm sai. Số tiền chỉ đến từ `calculate_room_price_preview`.
        for money_field in ["total_price", "total", "price", "amount"] {
            assert!(
                properties.get(money_field).is_none(),
                "schema mời model tự đưa tiền phòng qua ô `{money_field}`: {}",
                draft.parameters
            );
        }
        assert!(
            draft.description.contains("KHÔNG có tham số tiền phòng"),
            "mô tả tool phải nói thẳng là không có ô tiền phòng: {}",
            draft.description
        );

        // Ba ô bắt buộc **vô điều kiện**.
        for field in ["room_id", "guests", "check_in_date"] {
            assert!(
                properties.contains_key(field),
                "schema thiếu ô `{field}`: {}",
                draft.parameters
            );
            assert!(
                required.contains(&field),
                "`{field}` phải nằm trong `required`: {required:?}"
            );
        }

        // Ba ô ngày phải nói ra định dạng, không thì model gửi "ngày 4" và
        // `build_backfill_draft` chỉ còn cách từ chối.
        for field in ["check_in_date", "check_out_date", "expected_checkout_date"] {
            let description = properties[field]["description"]
                .as_str()
                .unwrap_or_default();
            assert_eq!(properties[field]["type"], json!("string"), "{field}");
            assert!(
                description.contains("YYYY-MM-DD"),
                "mô tả `{field}` phải nêu định dạng: {description}"
            );
        }

        // `check_in_date` phải nói ra hướng thời gian, không thì model coi
        // `draft_backfill` là một `draft_check_in` khác tên.
        let check_in_description = properties["check_in_date"]["description"]
            .as_str()
            .unwrap_or_default();
        assert!(
            check_in_description.contains("QUÁ KHỨ"),
            "mô tả ngày vào phải nói rõ là quá khứ: {check_in_description}"
        );

        // Hai ô ngày trả **tuỳ chọn ở tầng schema** — cái nào bắt buộc phụ thuộc
        // khách còn ở hay đã đi, mà JSON schema không nói được câu điều kiện ấy
        // theo cách mọi nhà cung cấp đều tôn trọng. Nên mô tả phải nói ra luật,
        // và `build_backfill_draft` là chỗ canh thật.
        for field in [
            "check_out_date",
            "expected_checkout_date",
            "paid_amount",
            "notes",
        ] {
            assert!(
                properties.contains_key(field),
                "schema thiếu ô tuỳ chọn `{field}`: {}",
                draft.parameters
            );
            assert!(
                !required.contains(&field),
                "`{field}` phải là tuỳ chọn: {required:?}"
            );
        }
        let expected_description = properties["expected_checkout_date"]["description"]
            .as_str()
            .unwrap_or_default();
        assert!(
            expected_description.contains("BẮT BUỘC khi khách CÒN Ở"),
            "mô tả `expected_checkout_date` phải nói ra luật bắt buộc có điều kiện: \
             {expected_description}"
        );

        // `guests` là DANH SÁCH ở đây (giống `draft_check_in`), không phải một ô
        // tên kiểu chuỗi như `draft_reserve` — `backfill_stay` ghi trọn hồ sơ
        // từng khách.
        assert_eq!(
            properties["guests"]["type"],
            json!("array"),
            "{}",
            draft.parameters
        );
    }

    /// Một tool có tên trong danh sách nhưng schema `{}` rỗng là một tool model
    /// không gọi nổi — nó không biết truyền gì. Rơi vào nhánh `_` của
    /// `draft_tool_parameters` là im lặng, nên bắt ở đây.
    #[test]
    fn every_draft_tool_offers_the_model_a_real_schema() {
        for tool in FRONT_DESK_DRAFT_TOOLS {
            let parameters = draft_tool_parameters(tool.name);
            let properties = parameters["properties"]
                .as_object()
                .unwrap_or_else(|| panic!("{} thiếu `properties`", tool.name));
            assert!(
                !properties.is_empty(),
                "{} nhận schema rỗng — model không có gì để điền",
                tool.name
            );
        }
    }

    /// Vòng lặp trong `mod.rs` chọn hàm dựng thẻ qua `draft_tool_kind`. Một tool
    /// ghi có mặt trong danh sách mà không có `DraftToolKind` thì model gọi được
    /// nó nhưng vòng lặp coi nó như tool đọc, rồi `execute_read_tool` trả
    /// `AGENT_TOOL_NOT_ALLOWED` — trợ lý nói "không có công cụ nào cho việc này"
    /// về đúng công cụ nó vừa gọi.
    #[test]
    fn every_draft_tool_maps_to_a_builder() {
        for tool in FRONT_DESK_DRAFT_TOOLS {
            assert!(
                draft_tool_kind(tool.name).is_some(),
                "{} không có nhánh trong `draft_tool_kind`",
                tool.name
            );
        }

        // Chiều ngược: tool đọc không được lọt vào đường dựng thẻ.
        for tool in FRONT_DESK_READ_TOOLS {
            assert!(
                draft_tool_kind(tool.name).is_none(),
                "{} là tool đọc mà bị coi là tool dựng thẻ",
                tool.name
            );
        }
        assert!(draft_tool_kind("rm_minus_rf").is_none());
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

    /// Trợ lý đọc số này ra thành câu nói trong khung chat. Nó phải là số quầy
    /// thu, tức số `stay_lifecycle::check_in` tính — và lệnh đó truyền `None`
    /// cho số khách, không bao giờ tính phụ thu thêm người. Phòng dưới đây có
    /// phụ thu 150.000₫/khách/đêm nên một tham số `guests` lọt xuống engine sẽ
    /// đội tổng lên 300.000₫.
    #[tokio::test]
    async fn quote_room_price_ignores_a_guest_count_the_desk_will_not_charge_for() {
        let pool = test_pool().await;

        seed_room_charging_extra_guests(&pool, "room-qp-extra", "P402", "Family Room", 300_000)
            .await;

        let output = execute_read_tool(
            &pool,
            "quote_room_price",
            &serde_json::json!({
                "room_id": "room-qp-extra",
                "check_in": "2026-06-01",
                "check_out": "2026-06-03",
                "guests": 4
            }),
        )
        .await
        .expect("tool phải chạy được");

        assert_eq!(output["total"], json!(600_000));
    }

    /// Tham số nào còn trong schema là lời mời model điền vào. `guests` không
    /// còn ảnh hưởng tới kết quả nữa nên phải biến mất khỏi schema, không thì
    /// model sẽ tưởng mình đang hỏi giá cho đúng số khách đó.
    #[test]
    fn the_quote_tool_no_longer_offers_the_model_a_guest_count() {
        let schema = read_tool_parameters("quote_room_price");

        assert!(
            schema["properties"].get("guests").is_none(),
            "schema vẫn mời model gửi `guests`: {schema}"
        );
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

    /// `seed_room` để schema điền `max_guests`/`extra_person_fee` mặc định
    /// (2, 0), nên khoản phụ thu thêm người luôn bằng 0 và không fixture nào ở
    /// trên phân biệt được "có gửi số khách" với "không gửi số khách".
    async fn seed_room_charging_extra_guests(
        pool: &Pool<Sqlite>,
        id: &str,
        name: &str,
        room_type: &str,
        base_price: i64,
    ) {
        sqlx::query(
            "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status)
             VALUES (?, ?, ?, 1, 0, ?, 2, 150000, 'vacant')",
        )
        .bind(id)
        .bind(name)
        .bind(room_type)
        .bind(base_price)
        .execute(pool)
        .await
        .expect("seed room có phụ thu thêm người");
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
