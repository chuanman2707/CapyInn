use crate::{
    app_error::{codes, CommandError, CommandResult},
    models::{CheckInRequest, CreateGuestRequest},
    queries::rooms::assistant_queries::load_room_status_now,
    services::booking::pricing_service,
};
use chrono::NaiveDate;
use serde::Serialize;
use serde_json::Value;
use sqlx::{Pool, Sqlite};
use std::collections::BTreeMap;

pub const CHECK_IN_ACTION_KIND: &str = "check_in";

#[derive(Debug, Clone, Serialize)]
pub struct ProposedAction {
    pub kind: String,
    pub payload: CheckInRequest,
    pub display: BTreeMap<String, String>,
    pub preview: Value,
    pub warnings: Vec<String>,
    pub built_at_ms: i64,
}

#[derive(Debug)]
pub enum DraftOutcome {
    Ready(Box<ProposedAction>),
    MissingFields(Vec<String>),
    /// Người dùng nêu một ngày nhận phòng **không phải hôm nay**. Không dựng
    /// thẻ: lệnh `check_in` đóng dấu `Local::now()` lúc bấm nút, nên một thẻ
    /// nhận phòng cho ngày khác là một thẻ nói dối.
    ///
    /// `requested` giữ **nguyên văn** chuỗi model gửi lên, để câu trả về cho
    /// model nhắc lại đúng ngày người dùng đã nêu chứ không phải một ngày đã
    /// bị chuẩn hoá lại.
    WrongDateForCheckIn {
        requested: String,
        is_future: bool,
    },
    /// `check_in_date` có mặt nhưng không đọc được thành một ngày lịch (model
    /// gửi thẳng "ngày 8" chẳng hạn).
    ///
    /// Tách khỏi `WrongDateForCheckIn` chứ không gộp vào với một `is_future`
    /// đoán bừa: gộp thì "ngày 8" sẽ bị đoán là quá khứ rồi đẩy sang
    /// `draft_backfill`, tức ghi bù cho một kỳ ở còn chưa xảy ra. Đoán sai
    /// hướng còn tệ hơn không đoán.
    UnreadableCheckInDate {
        requested: String,
    },
}

/// Đổi số đêm thành ngày trả phòng, theo lịch địa phương.
pub fn check_out_date_from_nights(check_in: &str, nights: i32) -> CommandResult<String> {
    if nights < 1 {
        return Err(CommandError::user(
            codes::VALIDATION_INVALID_INPUT,
            "Số đêm phải từ 1 trở lên.",
        ));
    }

    let start = NaiveDate::parse_from_str(check_in, "%Y-%m-%d").map_err(|_| {
        CommandError::user(
            codes::VALIDATION_INVALID_INPUT,
            "Ngày nhận phòng không hợp lệ.",
        )
    })?;

    let end = start
        .checked_add_days(chrono::Days::new(nights as u64))
        .ok_or_else(|| {
            CommandError::user(codes::VALIDATION_INVALID_INPUT, "Khoảng ngày quá xa.")
        })?;

    Ok(end.format("%Y-%m-%d").to_string())
}

/// `YYYY-MM-DD` → `DD/MM/YYYY`, cách người Việt đọc một ngày.
///
/// Không đọc được thì trả nguyên chuỗi vào. Thẻ thà hiện một chuỗi lạ còn hơn
/// hiện một ngày bịa ra hoặc nuốt luôn dòng ngày — nuốt im lặng chính là lớp
/// lỗi cả đợt này đang sửa.
fn format_vn_date(date: &str) -> String {
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|value| value.format("%d/%m/%Y").to_string())
        .unwrap_or_else(|_| date.to_string())
}

/// `check_in_date` / `check_out_date` là **khoảng ngày đã dùng để hỏi giá**,
/// không phải hai trường mới của payload — xem ghi chú trong thân hàm.
pub fn build_check_in_display(
    payload: &CheckInRequest,
    preview: &Value,
    check_in_date: &str,
    check_out_date: &str,
) -> BTreeMap<String, String> {
    let mut display = BTreeMap::new();

    display.insert("room_id".to_string(), payload.room_id.clone());

    // Hai dòng ngày là thứ đã thiếu khi con bug 06/08 xảy ra: thẻ hiện `nights`
    // nhưng không hiện ngày nào, nên lễ tân đọc kỹ tới đâu cũng không có gì để
    // đối chiếu với câu mình vừa gõ ("checkin 8 out 9"). Một cái thẻ ghi sai
    // ngày trông y hệt một cái thẻ ghi đúng.
    //
    // Hai giá trị này **dẫn xuất**, không nằm trong payload — `CheckInRequest`
    // không có trường ngày nào và giữ nguyên đúng bảy trường như cũ.
    //
    // Vì sao điều đó KHÔNG vi phạm bất biến "display không bỏ sót trường
    // payload": bất biến ấy cấm **bỏ bớt**, không cấm **thêm**. Nó đọc theo
    // chiều payload → display (mỗi trường payload phải có mặt trên thẻ), nên
    // một khoá thừa trên thẻ không làm nó sai — xem
    // `the_card_shows_every_field_of_the_payload`: nó duyệt khoá của *payload*
    // rồi tìm trên thẻ, chứ không duyệt khoá của *display* rồi tìm trong
    // payload. Thêm ≠ bớt.
    //
    // Nhãn "Hôm nay" viết cứng được là nhờ nhánh từ chối trong
    // `build_check_in_draft`: thẻ nhận phòng chỉ dựng được khi ngày nhận đúng
    // là `now_local_date`. Ai gỡ nhánh ấy thì phải quay lại sửa dòng này —
    // `a_draft_for_tomorrow_is_refused_and_points_at_the_reservation_tool` là
    // chỗ báo động.
    display.insert(
        "check_in_date".to_string(),
        format!("Hôm nay, {}", format_vn_date(check_in_date)),
    );
    display.insert("check_out_date".to_string(), format_vn_date(check_out_date));

    // Khách: một dòng đếm đầu người, rồi **mỗi khách một dòng riêng mang mọi
    // trường đã điền** — không phải chỉ họ tên ghép bằng dấu phẩy.
    //
    // Số giấy tờ (CCCD) do model đọc ra là thứ `check_in` ghi thẳng vào
    // `guests.doc_number` rồi đi vào khai báo tạm trú. Thẻ giấu nó đi nghĩa là
    // con người bấm "Đồng ý" cho một con số họ chưa từng nhìn thấy — model gõ
    // sai một chữ số, hay bịa ra cả dãy, cũng không ai chặn được.
    display.insert(
        "guests".to_string(),
        format!("{} người", payload.guests.len()),
    );

    // `display` là `BTreeMap`, thẻ hiện theo thứ tự chuỗi của khoá: không đệm
    // 0 thì "Khách 10" đứng trước "Khách 2".
    let index_width = payload.guests.len().to_string().len();
    for (index, guest) in payload.guests.iter().enumerate() {
        display.insert(
            format!("Khách {:0width$}", index + 1, width = index_width),
            guest_display_line(guest),
        );
    }

    display.insert("nights".to_string(), format!("{} đêm", payload.nights));
    display.insert(
        "source".to_string(),
        payload.source.clone().unwrap_or_else(|| "—".to_string()),
    );
    display.insert(
        "notes".to_string(),
        payload.notes.clone().unwrap_or_else(|| "—".to_string()),
    );
    display.insert(
        "paid_amount".to_string(),
        payload
            .paid_amount
            .map(format_vnd)
            .unwrap_or_else(|| "0 ₫".to_string()),
    );
    display.insert(
        "pricing_type".to_string(),
        payload
            .pricing_type
            .clone()
            .unwrap_or_else(|| "nightly".to_string()),
    );
    display.insert(
        "total".to_string(),
        preview
            .get("total")
            .and_then(Value::as_i64)
            .map(format_vnd)
            .unwrap_or_else(|| "—".to_string()),
    );

    display
}

/// Gói mọi trường **đã điền** của một khách thành một dòng đọc được.
///
/// Đây là điểm sửa duy nhất khi `CreateGuestRequest` mọc thêm trường: thêm
/// trường mà quên thêm nhãn ở đây thì `the_card_shows_every_field_of_the_payload`
/// đỏ. Trường rỗng hoặc `None` bị bỏ qua — không có gì để người duyệt nhìn, và
/// một dòng đầy nhãn trống chỉ làm số giấy tờ khó thấy hơn. Riêng `doc_number`
/// rỗng thì `build_warnings` nói hộ.
///
/// Nhãn bám theo form nhận phòng làm tay (`CheckinSheet.tsx`) để lễ tân đọc
/// thẻ và đọc form thấy cùng một thứ tiếng.
fn guest_display_line(guest: &CreateGuestRequest) -> String {
    let mut parts = vec![guest.full_name.trim().to_string()];

    let mut push = |label: &str, value: Option<&str>| {
        if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
            parts.push(format!("{label}: {value}"));
        }
    };

    // Thứ tự cố ý: hai thứ lễ tân đối chiếu với giấy tờ trên tay đứng trước.
    push("CCCD", Some(guest.doc_number.as_str()));
    push("SĐT", guest.phone.as_deref());
    push("Loại khách", guest.guest_type.as_deref());
    push("Ngày sinh", guest.dob.as_deref());
    push("Giới tính", guest.gender.as_deref());
    push("Quốc tịch", guest.nationality.as_deref());
    push("Địa chỉ", guest.address.as_deref());
    push("Visa hết hạn", guest.visa_expiry.as_deref());
    push("Ảnh giấy tờ", guest.scan_path.as_deref());

    parts.join(" · ")
}

fn format_vnd(amount: i64) -> String {
    let digits = amount.abs().to_string();
    let mut grouped = String::new();
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            grouped.push('.');
        }
        grouped.push(ch);
    }
    if amount < 0 {
        format!("-{grouped} ₫")
    } else {
        format!("{grouped} ₫")
    }
}

pub async fn build_check_in_draft(
    pool: &Pool<Sqlite>,
    args: &Value,
    now_local_date: &str,
) -> CommandResult<DraftOutcome> {
    let mut missing = Vec::new();

    let room_id = args
        .get("room_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if room_id.is_none() {
        missing.push("room_id".to_string());
    }

    let guests: Vec<CreateGuestRequest> = args
        .get("guests")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| {
                    let full_name = entry.get("full_name")?.as_str()?.trim();
                    if full_name.is_empty() {
                        return None;
                    }
                    Some(CreateGuestRequest {
                        guest_type: None,
                        full_name: full_name.to_string(),
                        doc_number: entry
                            .get("doc_number")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        dob: None,
                        gender: None,
                        nationality: None,
                        address: None,
                        visa_expiry: None,
                        scan_path: None,
                        phone: entry
                            .get("phone")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    if guests.is_empty() {
        missing.push("guests".to_string());
    }

    let nights = args.get("nights").and_then(Value::as_i64).unwrap_or(0) as i32;
    if nights < 1 {
        missing.push("nights".to_string());
    }

    // ─── Ngày nhận phòng: soi ô ngày trước cả việc thiếu trường ───
    //
    // `check_in` đóng dấu `Local::now()` lúc bấm nút (`stay_lifecycle.rs`), nên
    // thẻ nhận phòng chỉ đúng cho HÔM NAY. Ngày 06/08/2026 lễ tân gõ "checkin 8
    // out 9 tháng 8": model không có ô nào để đặt số 8 vào nên bỏ luôn, thẻ ghi
    // ngày hôm nay, phòng 4B bị khoá khỏi tra phòng trống hai ngày và một đêm
    // chưa xảy ra bị thu 400.000₫. Giờ có ô, và đây là chỗ soi cái ô đó.
    //
    // Kiểm **trước** `missing` là cố ý: chọn nhầm tool là lỗi to hơn thiếu một
    // trường. Hỏi lại tên khách cho một cái thẻ sẽ không bao giờ dựng được vừa
    // tiêu mất một vòng trong ngân sách `MAX_TOOL_ROUNDS` (4), vừa xác nhận
    // ngược cho model rằng `draft_check_in` là đường đúng — nó chỉ cần điền nốt
    // là xong.
    if let Some(requested) = args
        .get("check_in_date")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        // So theo NGÀY LỊCH, không so chuỗi và không so timestamp. "Hôm nay" là
        // một ngày, không phải một thời điểm; và `2026-8-6` với `2026-08-06` là
        // cùng một ngày, một dấu 0 thiếu không được biến thành lời từ chối.
        let today = NaiveDate::parse_from_str(now_local_date, "%Y-%m-%d").map_err(|_| {
            CommandError::system(
                codes::SYSTEM_INTERNAL_ERROR,
                format!("Ngày hôm nay `{now_local_date}` không đọc được."),
            )
        })?;

        let Ok(requested_date) = NaiveDate::parse_from_str(requested, "%Y-%m-%d") else {
            // Không đoán, và tuyệt đối không rơi về hôm nay. "ngày 8" có thể là
            // mùng 8 tháng này, tháng sau hay năm sau — rơi về hôm nay đúng là
            // con bug này.
            return Ok(DraftOutcome::UnreadableCheckInDate {
                requested: requested.to_string(),
            });
        };

        if requested_date != today {
            return Ok(DraftOutcome::WrongDateForCheckIn {
                requested: requested.to_string(),
                is_future: requested_date > today,
            });
        }
    }

    if !missing.is_empty() {
        return Ok(DraftOutcome::MissingFields(missing));
    }

    let room_id = room_id.expect("đã kiểm ở trên").to_string();
    let check_out = check_out_date_from_nights(now_local_date, nights)?;
    let pricing_type = args
        .get("pricing_type")
        .and_then(Value::as_str)
        .unwrap_or("nightly")
        .to_string();

    // Số tiền trên thẻ đến từ preview, không từ model. Preview hỏng thì không
    // có thẻ — không có số mặc định nào.
    //
    // `None` cho số khách, **không** phải `guests.len()`: nút "Đồng ý" gọi
    // `check_in`, và `stay_lifecycle::check_in` truyền `None` xuống engine
    // (`stay_lifecycle.rs`, chỗ gọi `calculate_stay_price_tx`), tức quầy không
    // thu phụ thu thêm người. Gửi số khách ở đây thì thẻ báo cao hơn số thực
    // thu — lễ tân đọc con số đó cho khách nghe. Cùng luật với form làm tay
    // (`CheckinSheet.tsx` truyền `guests: null`) và với ghi chú ở
    // `hooks/usePricePreview.ts`.
    let preview_result = pricing_service::calculate_room_price_preview(
        pool,
        &room_id,
        now_local_date,
        &check_out,
        &pricing_type,
        None,
    )
    .await
    .map_err(|error| {
        CommandError::user(
            codes::AGENT_PREVIEW_UNAVAILABLE,
            format!("Không tra được giá phòng nên chưa dựng được thẻ: {error}"),
        )
    })?;

    let preview = serde_json::to_value(&preview_result).map_err(|error| {
        CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            format!("Không mã hoá được báo giá: {error}"),
        )
    })?;

    let payload = CheckInRequest {
        room_id: room_id.clone(),
        guests,
        nights,
        source: args
            .get("source")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| Some("walk-in".to_string())),
        notes: args
            .get("notes")
            .and_then(Value::as_str)
            .map(str::to_string),
        paid_amount: args.get("paid_amount").and_then(Value::as_i64),
        pricing_type: Some(pricing_type),
    };

    let warnings = build_warnings(pool, &room_id, &payload.guests).await?;
    // Đúng khoảng ngày vừa dùng để hỏi giá — tiền trên thẻ và ngày trên thẻ
    // phải nói về cùng một kỳ ở, không phải hai nguồn sự thật tính riêng.
    let display = build_check_in_display(&payload, &preview, now_local_date, &check_out);

    Ok(DraftOutcome::Ready(Box::new(ProposedAction {
        kind: CHECK_IN_ACTION_KIND.to_string(),
        payload,
        display,
        preview,
        warnings,
        built_at_ms: chrono::Utc::now().timestamp_millis(),
    })))
}

/// Cảnh báo tra từ PMS, không phải lời model viết ra.
async fn build_warnings(
    pool: &Pool<Sqlite>,
    room_id: &str,
    guests: &[CreateGuestRequest],
) -> CommandResult<Vec<String>> {
    let rooms = load_room_status_now(pool).await.map_err(|error| {
        CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            format!("Không đọc được trạng thái phòng: {error}"),
        )
    })?;

    let mut warnings = Vec::new();
    if let Some(room) = rooms.iter().find(|room| room.room_id == room_id) {
        if room.status.eq_ignore_ascii_case("dirty") {
            warnings.push("Phòng đang ở trạng thái bẩn, chưa dọn.".to_string());
        }
        if room.booking_id.is_some() {
            warnings.push("Phòng đang có khách ở.".to_string());
        }
    }

    // Trợ lý dựng được một khách mà chính form của PMS sẽ từ chối: ở trên,
    // `doc_number` vắng mặt được mặc định thành `""`, và
    // `stay_lifecycle::validate_check_in_request` không kiểm trường đó — trong
    // khi `CheckinSheet.tsx` khoá nút lưu khi thiếu. Cố ý **chỉ cảnh báo**,
    // không từ chối: con người là bước duyệt theo đúng thiết kế, còn siết
    // thành lỗi cứng là quyết định sản phẩm của chủ nhà, không phải của chỗ
    // này.
    for guest in guests {
        if guest.doc_number.trim().is_empty() {
            warnings.push(format!(
                "Khách «{}» chưa có số giấy tờ. Form nhận phòng làm tay không cho lưu như vậy \
                 (chế độ nhanh phải có số điện thoại thay thế), và hồ sơ khai báo tạm trú sẽ thiếu.",
                guest.full_name.trim()
            ));
        }
    }

    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CreateGuestRequest;

    /// Khách điền **đủ mười trường** của `CreateGuestRequest`, mỗi trường một
    /// giá trị không trùng nhau và không là chuỗi con của nhau. Đây là điều
    /// kiện để `the_card_shows_every_field_of_the_payload` có nghĩa: nó dò
    /// từng giá trị lá trên thẻ, nên một trường bỏ trống trong fixture sẽ được
    /// bỏ qua và che mất đúng cái lỗi cần bắt.
    ///
    /// Thêm trường mới vào `CreateGuestRequest` sẽ làm literal này **không
    /// biên dịch được** — buộc người thêm phải cho nó một giá trị, rồi test
    /// bên dưới bắt tiếp nếu thẻ không hiện nó ra.
    fn sample_guest(full_name: &str) -> CreateGuestRequest {
        CreateGuestRequest {
            guest_type: Some("domestic".to_string()),
            full_name: full_name.to_string(),
            doc_number: "079201001234".to_string(),
            dob: Some("1992-03-15".to_string()),
            gender: Some("Nữ".to_string()),
            nationality: Some("Việt Nam".to_string()),
            address: Some("12 Lê Lợi, Đà Nẵng".to_string()),
            visa_expiry: Some("2027-01-31".to_string()),
            scan_path: Some("/anh/giay-to-1.jpg".to_string()),
            phone: Some("0909000111".to_string()),
        }
    }

    /// Khách thứ hai chỉ có ba trường như model thật vẫn gửi, và không trường
    /// nào trùng khách thứ nhất — để một thẻ chỉ hiện khách đầu tiên bị bắt.
    fn second_sample_guest() -> CreateGuestRequest {
        CreateGuestRequest {
            guest_type: None,
            full_name: "Lê Văn Cường".to_string(),
            doc_number: "079088007766".to_string(),
            dob: None,
            gender: None,
            nationality: None,
            address: None,
            visa_expiry: None,
            scan_path: None,
            phone: Some("0912345678".to_string()),
        }
    }

    fn sample_payload() -> CheckInRequest {
        CheckInRequest {
            room_id: "R1".to_string(),
            guests: vec![sample_guest("Trần Thị Bích"), second_sample_guest()],
            nights: 2,
            source: Some("walk-in".to_string()),
            notes: Some("khách quen".to_string()),
            paid_amount: Some(500_000),
            pricing_type: Some("nightly".to_string()),
        }
    }

    /// Mọi giá trị lá nằm dưới một trường lồng của payload phải xuất hiện
    /// nguyên văn ở đâu đó trên thẻ.
    ///
    /// `null` bị bỏ qua vì không mang thông tin nào để hiện. Chuỗi rỗng cũng
    /// vậy — `doc_number` rỗng đã có cảnh báo riêng lo, xem
    /// `build_warnings`.
    fn assert_nested_leaves_are_on_the_card(path: &str, value: &Value, shown: &str) {
        match value {
            Value::Object(fields) => {
                for (key, nested) in fields {
                    assert_nested_leaves_are_on_the_card(&format!("{path}.{key}"), nested, shown);
                }
            }
            Value::Array(entries) => {
                for (index, nested) in entries.iter().enumerate() {
                    assert_nested_leaves_are_on_the_card(
                        &format!("{path}[{index}]"),
                        nested,
                        shown,
                    );
                }
            }
            Value::Null => {}
            leaf => {
                let text = leaf
                    .as_str()
                    .map(str::to_string)
                    .unwrap_or_else(|| leaf.to_string());
                if text.trim().is_empty() {
                    return;
                }
                assert!(
                    shown.contains(&text),
                    "giá trị `{path}` = `{text}` của payload không hiện trên thẻ xác nhận.\nThẻ đang hiện:\n{shown}"
                );
            }
        }
    }

    /// Đây là luật số một của thiết kế: người dùng duyệt đúng cái sẽ được gửi.
    /// Thêm trường vào `CheckInRequest` **hoặc vào `CreateGuestRequest`** mà
    /// quên hiện lên thẻ là test này đỏ.
    ///
    /// Bản cũ chỉ duyệt bảy khoá tầng ngoài của `CheckInRequest`. `guests` có
    /// mặt trong `display` nên nó xanh, trong khi số giấy tờ và số điện thoại
    /// của từng khách — thứ sẽ được ghi thẳng vào `guests.doc_number` rồi đi
    /// vào khai báo tạm trú — không hề hiện ra. Giờ nó đệ quy xuống từng lá.
    #[test]
    fn the_card_shows_every_field_of_the_payload() {
        let payload = sample_payload();
        let preview = serde_json::json!({ "total": 700_000 });

        let display = build_check_in_display(&payload, &preview, "2026-08-06", "2026-08-08");

        let encoded = serde_json::to_value(&payload).expect("payload phải serialize được");
        let fields = encoded.as_object().expect("payload là một object");
        let shown = display
            .values()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n");

        for (key, value) in fields {
            assert!(
                display.contains_key(key),
                "trường `{key}` của payload không hiện trên thẻ xác nhận"
            );
            if value.is_object() || value.is_array() {
                assert_nested_leaves_are_on_the_card(key, value, &shown);
            }
        }
    }

    /// Nói thẳng ra cái mà test đệ quy ở trên chỉ nói gián tiếp: con số CCCD
    /// mà con người sắp cho phép ghi vào hồ sơ khai báo tạm trú phải nằm ngay
    /// trên thẻ họ đang nhìn.
    #[test]
    fn the_card_shows_the_document_number_it_is_about_to_write() {
        let payload = sample_payload();
        let preview = serde_json::json!({ "total": 700_000 });

        let display = build_check_in_display(&payload, &preview, "2026-08-06", "2026-08-08");

        let first = display
            .get("Khách 1")
            .expect("phải có dòng riêng cho khách thứ nhất");
        assert!(first.contains("Trần Thị Bích"), "{first}");
        assert!(first.contains("079201001234"), "{first}");
        assert!(first.contains("0909000111"), "{first}");

        let second = display
            .get("Khách 2")
            .expect("phải có dòng riêng cho khách thứ hai");
        assert!(second.contains("Lê Văn Cường"), "{second}");
        assert!(second.contains("079088007766"), "{second}");
    }

    /// Khoá của `display` là `BTreeMap`, tức thẻ hiện theo thứ tự chuỗi. Không
    /// đệm 0 thì "Khách 10" đứng trước "Khách 2" và danh sách khách đọc lộn
    /// xộn đúng lúc đông người nhất.
    #[test]
    fn ten_guests_stay_in_order_on_the_card() {
        let payload = CheckInRequest {
            guests: (1..=10)
                .map(|index| {
                    let mut guest = second_sample_guest();
                    guest.full_name = format!("Khách số {index}");
                    guest.doc_number = format!("0790000000{index:02}");
                    guest.phone = None;
                    guest
                })
                .collect(),
            ..sample_payload()
        };

        let display = build_check_in_display(
            &payload,
            &serde_json::json!({ "total": 0 }),
            "2026-08-06",
            "2026-08-08",
        );

        let order: Vec<&str> = display
            .keys()
            .filter(|key| key.starts_with("Khách "))
            .map(String::as_str)
            .collect();
        assert_eq!(order.first(), Some(&"Khách 01"), "{order:?}");
        assert_eq!(order.last(), Some(&"Khách 10"), "{order:?}");
    }

    #[test]
    fn the_card_shows_the_preview_total_not_a_model_number() {
        let payload = sample_payload();
        let preview = serde_json::json!({ "total": 700_000 });

        let display = build_check_in_display(&payload, &preview, "2026-08-06", "2026-08-08");

        assert!(display
            .get("total")
            .expect("phải có dòng tổng tiền")
            .contains("700"));
    }

    /// Lễ tân phải đối chiếu được ngày trên thẻ với câu mình vừa gõ, **trước**
    /// khi bấm Đồng ý. Đây đúng là thứ đã thiếu hôm 06/08: thẻ hiện `nights`
    /// nhưng không hiện ngày nào, nên "checkin 8 out 9" và một thẻ ghi hôm nay
    /// nhìn giống hệt nhau.
    ///
    /// Định dạng Việt Nam `DD/MM/YYYY`, không phải `YYYY-MM-DD` của máy: người
    /// đọc thẻ là người, và `08/06` với `06/08` là hai ngày khác nhau.
    #[test]
    fn the_card_shows_the_stay_dates_in_vietnamese_format() {
        let payload = sample_payload();
        let preview = serde_json::json!({ "total": 700_000 });

        let display = build_check_in_display(&payload, &preview, "2026-08-06", "2026-08-07");

        assert_eq!(
            display.get("check_in_date").map(String::as_str),
            Some("Hôm nay, 06/08/2026")
        );
        assert_eq!(
            display.get("check_out_date").map(String::as_str),
            Some("07/08/2026")
        );
    }

    #[tokio::test]
    async fn a_draft_without_guests_reports_the_missing_field() {
        let pool = test_pool().await;
        let args = serde_json::json!({ "room_id": "R1", "nights": 2 });

        let outcome = build_check_in_draft(&pool, &args, "2026-08-03")
            .await
            .expect("thiếu trường không phải lỗi hệ thống");

        match outcome {
            DraftOutcome::MissingFields(fields) => assert!(fields.contains(&"guests".to_string())),
            other => panic!("mong đợi MissingFields, nhận {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_draft_without_a_room_reports_the_missing_field() {
        let pool = test_pool().await;
        let args = serde_json::json!({ "nights": 2, "guests": [{ "full_name": "Nam" }] });

        let outcome = build_check_in_draft(&pool, &args, "2026-08-03")
            .await
            .expect("thiếu trường không phải lỗi hệ thống");

        match outcome {
            DraftOutcome::MissingFields(fields) => {
                assert!(fields.contains(&"room_id".to_string()))
            }
            other => panic!("mong đợi MissingFields, nhận {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_draft_without_nights_reports_the_missing_field() {
        let pool = test_pool().await;
        let args = serde_json::json!({ "room_id": "R1", "guests": [{ "full_name": "Nam" }] });

        let outcome = build_check_in_draft(&pool, &args, "2026-08-03")
            .await
            .expect("thiếu trường không phải lỗi hệ thống");

        match outcome {
            DraftOutcome::MissingFields(fields) => {
                assert!(fields.contains(&"nights".to_string()))
            }
            other => panic!("mong đợi MissingFields, nhận {other:?}"),
        }
    }

    /// `nights < 1` đi chung nhánh với vắng mặt — `unwrap_or(0)` biến "0" thành
    /// cùng một con số 0 rồi cùng rớt vào điều kiện `nights < 1`.
    #[tokio::test]
    async fn a_draft_with_zero_nights_reports_the_missing_field() {
        let pool = test_pool().await;
        let args = serde_json::json!({
            "room_id": "R1",
            "nights": 0,
            "guests": [{ "full_name": "Nam" }]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-08-03")
            .await
            .expect("thiếu trường không phải lỗi hệ thống");

        match outcome {
            DraftOutcome::MissingFields(fields) => {
                assert!(fields.contains(&"nights".to_string()))
            }
            other => panic!("mong đợi MissingFields, nhận {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_draft_for_an_unknown_room_fails_instead_of_quoting_a_default() {
        let pool = test_pool().await;
        let args = serde_json::json!({
            "room_id": "khong-ton-tai",
            "nights": 2,
            "guests": [{ "full_name": "Nam" }]
        });

        let error = build_check_in_draft(&pool, &args, "2026-08-03")
            .await
            .expect_err("không tra được giá thì không được dựng thẻ");

        assert_eq!(error.code, codes::AGENT_PREVIEW_UNAVAILABLE);
    }

    // ─── Happy-path coverage for `DraftOutcome::Ready` ───
    //
    // Every test above stops at `MissingFields` or the unknown-room `Err`, so
    // `build_warnings` and the way `payload`/`preview`/`warnings` get assembled
    // into a `ProposedAction` never actually ran. These seed a real room,
    // following the same minimum recipe proven in `tools.rs`'s
    // `quote_room_price_prices_a_seeded_room_over_two_weekday_nights`: just a
    // `rooms` row, no `pricing_rules`/`room_types` row required for
    // `calculate_room_price_preview` to succeed.

    /// Chứng minh cả đường `Ready`: thẻ hiện đúng phòng/khách đã seed, tổng
    /// tiền trên thẻ là tổng của preview thật (không phải 0, không phải mặc
    /// định house 350k/400k), và `payload` mang đúng những gì đã truyền vào.
    /// Phòng seed sạch và còn trống nên `warnings` phải rỗng — đối chứng cho
    /// test cảnh báo ngay bên dưới, để test đó không thể đúng một cách vô
    /// nghĩa (lúc nào cũng có cảnh báo bất kể trạng thái phòng).
    #[tokio::test]
    async fn a_draft_with_a_seeded_room_and_guest_is_ready_with_the_preview_total() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-ready",
            "P701",
            "Deluxe Balcony",
            500_000,
            "vacant",
        )
        .await;

        // 2026-06-01 là thứ Hai, 2026-06-03 là thứ Tư (đã kiểm bằng `date`/
        // `datetime`, không chỉ đọc comment) — kỳ ở này không dính đêm cuối
        // tuần nào, nên mức uplift cuối tuần mặc định 20% mà một phòng không
        // có `pricing_rules` sẽ rơi vào phải ra 0, và tổng phải đúng bằng 2
        // đêm x base_price, không hơn không kém.
        let args = serde_json::json!({
            "room_id": "room-ready",
            "nights": 2,
            "guests": [
                {
                    "full_name": "Nguyễn Văn Nam",
                    "doc_number": "079201001234",
                    "phone": "0909000111"
                }
            ],
            "source": "OTA",
            "notes": "khách quen",
            "paid_amount": 300_000,
            "pricing_type": "nightly"
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("dữ liệu hợp lệ với phòng có thật không được lỗi");

        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };

        assert_eq!(action.kind, CHECK_IN_ACTION_KIND);

        // Thẻ hiện đúng phòng và khách đã seed.
        assert_eq!(
            action.display.get("room_id").map(String::as_str),
            Some("room-ready")
        );
        assert_eq!(
            action.display.get("guests").map(String::as_str),
            Some("1 người")
        );
        assert_eq!(
            action.display.get("Khách 1").map(String::as_str),
            Some("Nguyễn Văn Nam · CCCD: 079201001234 · SĐT: 0909000111")
        );

        // Tổng trên thẻ phải là tổng của preview thật: 2 đêm x 500.000 base
        // price, không cuối tuần, không phụ thu — không phải 0 và không phải
        // một trong hai số mặc định house (350k hay daily_rate mặc định 400k
        // của `PricingRule::default()`) mà một rule bị rớt về default sẽ lộ ra.
        let preview_total = action
            .preview
            .get("total")
            .and_then(Value::as_i64)
            .expect("preview phải có total");
        assert_eq!(preview_total, 1_000_000);
        assert_eq!(
            action.display.get("total").map(String::as_str),
            Some("1.000.000 ₫")
        );

        // payload round-trip đúng những gì đã truyền vào.
        assert_eq!(action.payload.room_id, "room-ready");
        assert_eq!(action.payload.nights, 2);
        assert_eq!(action.payload.guests.len(), 1);
        assert_eq!(action.payload.guests[0].full_name, "Nguyễn Văn Nam");
        assert_eq!(action.payload.guests[0].doc_number, "079201001234");
        assert_eq!(
            action.payload.guests[0].phone.as_deref(),
            Some("0909000111")
        );
        assert_eq!(action.payload.source.as_deref(), Some("OTA"));
        assert_eq!(action.payload.notes.as_deref(), Some("khách quen"));
        assert_eq!(action.payload.paid_amount, Some(300_000));
        assert_eq!(action.payload.pricing_type.as_deref(), Some("nightly"));

        // Phòng sạch, không ai đang ở — không được có cảnh báo nào.
        assert!(
            action.warnings.is_empty(),
            "phòng sạch và trống không được có cảnh báo: {:?}",
            action.warnings
        );
    }

    // ─── Số trên thẻ phải bằng số lệnh nhận phòng thật sẽ ghi ───
    //
    // Mọi fixture `seed_room` ở trên bỏ trống `max_guests`/`extra_person_fee`,
    // nên schema điền mặc định (2, 0) và khoản phụ thu thêm người **luôn bằng
    // 0** — chênh lệch giữa hai cách gọi preview bị triệt tiêu về cấu trúc, dù
    // có sai. Hai test dưới đây dựng đúng cái phòng làm khoản đó khác 0.

    /// Preview của thẻ phải hỏi giá y như `stay_lifecycle::check_in` hỏi, tức
    /// **không** kèm số khách. Phòng dưới đây chuẩn 2 khách, phụ thu 150.000₫
    /// mỗi khách vượt mốc mỗi đêm; 3 khách × 2 đêm sẽ đội thêm 300.000₫ nếu
    /// thẻ lỡ gửi số khách đi.
    #[tokio::test]
    async fn the_card_does_not_quote_an_extra_person_fee_the_check_in_will_not_charge() {
        let pool = test_pool().await;
        seed_room_charging_extra_guests(
            &pool,
            "room-extra",
            "P703",
            "Family Room",
            500_000,
            "vacant",
        )
        .await;

        // 2026-06-01 thứ Hai → 2026-06-03 thứ Tư: không đêm cuối tuần nào, nên
        // 1.000.000₫ là con số duy nhất đúng.
        let args = serde_json::json!({
            "room_id": "room-extra",
            "nights": 2,
            "guests": [
                { "full_name": "Nguyễn Văn Nam" },
                { "full_name": "Trần Thị Hoa" },
                { "full_name": "Lê Văn Cường" }
            ]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("dữ liệu hợp lệ không được lỗi");

        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };

        assert_eq!(
            action.preview.get("total").and_then(Value::as_i64),
            Some(1_000_000),
            "preview của thẻ đang tính phụ thu thêm người mà quầy không thu"
        );
        assert_eq!(
            action.display.get("total").map(String::as_str),
            Some("1.000.000 ₫")
        );
    }

    /// Đường nối thật: lấy đúng `payload` mà thẻ mang, chạy qua chính
    /// `stay_lifecycle::check_in` — hàm mà nút "Đồng ý" gọi tới — rồi so tổng
    /// trên thẻ với tổng ghi vào `bookings.total_price`.
    ///
    /// Đây là chỗ khách hàng nghe một con số và sổ sách ghi một con số khác,
    /// nên nó phải có một test bám vào cả hai đầu, không phải hai test rời
    /// nhau mỗi bên tự khẳng định mình đúng.
    #[tokio::test]
    async fn the_card_total_is_the_total_the_real_check_in_records() {
        let pool = test_pool().await;
        seed_room_charging_extra_guests(
            &pool,
            "room-seam",
            "P704",
            "Family Room",
            500_000,
            "vacant",
        )
        .await;

        // `check_in_tx` chốt kỳ ở theo `Local::now()`, không nhận ngày truyền
        // vào, nên thẻ phải được dựng cho đúng hôm nay thì hai bên mới báo giá
        // cùng một khoảng ngày. Cả hai đi qua cùng phụ thu cuối tuần / ngày lễ
        // nên khẳng định "bằng nhau" đúng bất kể hôm nay là thứ mấy.
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let args = serde_json::json!({
            "room_id": "room-seam",
            "nights": 2,
            "guests": [
                { "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" },
                { "full_name": "Trần Thị Hoa", "doc_number": "079301005678" },
                { "full_name": "Lê Văn Cường", "doc_number": "079201009999" }
            ]
        });

        let outcome = build_check_in_draft(&pool, &args, &today)
            .await
            .expect("dữ liệu hợp lệ không được lỗi");

        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };
        let card_total = action
            .preview
            .get("total")
            .and_then(Value::as_i64)
            .expect("thẻ phải có tổng tiền");

        let booking = crate::services::booking::stay_lifecycle::check_in(
            &pool,
            action.payload.clone(),
            Some("user-test".to_string()),
        )
        .await
        .expect("payload của thẻ phải nhận phòng được");

        assert_eq!(
            card_total, booking.total_price,
            "thẻ báo {card_total} nhưng lượt ở ghi {}",
            booking.total_price
        );
        assert_eq!(
            action.display.get("total").map(String::as_str),
            Some(format_vnd(booking.total_price).as_str()),
            "dòng tổng tiền lễ tân đọc cho khách phải là số sổ sách ghi"
        );
    }

    /// `build_warnings` đọc `rooms.status` thật từ PMS, không phải câu do model
    /// tự viết ra — test này là bằng chứng tự động cho đúng luật đó, thay vì
    /// chỉ dựa vào người đọc code. Không so sánh rỗng/không-rỗng chung chung:
    /// so khớp đúng nội dung tiếng Việt và đúng số lượng, để một cảnh báo giả
    /// hoặc một cảnh báo thứ hai lọt vào cũng bị bắt.
    #[tokio::test]
    async fn a_ready_draft_carries_a_pms_warning_when_the_room_is_dirty() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-dirty",
            "P702",
            "Standard Room",
            300_000,
            "dirty",
        )
        .await;

        // Khách có số giấy tờ: test này canh đúng một cảnh báo trạng thái
        // phòng, không được vô tình cõng thêm cảnh báo thiếu giấy tờ.
        let args = serde_json::json!({
            "room_id": "room-dirty",
            "nights": 1,
            "guests": [{ "full_name": "Trần Thị Hoa", "doc_number": "079301005678" }]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("phòng bẩn vẫn tra được giá — không phải lỗi hệ thống");

        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };

        assert_eq!(
            action.warnings,
            vec!["Phòng đang ở trạng thái bẩn, chưa dọn.".to_string()]
        );
    }

    /// Trợ lý dựng được một khách mà chính form nhận phòng của PMS sẽ từ chối:
    /// `draft.rs` mặc định `doc_number` thành `""` và
    /// `validate_check_in_request` không kiểm trường đó. Đây **không** phải
    /// chặn cứng — con người vẫn là bước duyệt theo đúng thiết kế — nhưng thẻ
    /// phải nói ra sự chênh lệch đó thay vì để nó lặng lẽ trôi qua.
    #[tokio::test]
    async fn a_guest_without_a_document_number_gets_a_warning_naming_the_manual_form() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-nodoc",
            "P705",
            "Standard Room",
            300_000,
            "vacant",
        )
        .await;

        let args = serde_json::json!({
            "room_id": "room-nodoc",
            "nights": 1,
            "guests": [
                { "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" },
                { "full_name": "Phạm Thị Dung" }
            ]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("thiếu giấy tờ không phải lỗi hệ thống");

        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };

        // Đúng một cảnh báo, và nó phải gọi tên đúng người thiếu giấy tờ —
        // không phải người đã có.
        assert_eq!(action.warnings.len(), 1, "{:?}", action.warnings);
        let warning = &action.warnings[0];
        assert!(warning.contains("Phạm Thị Dung"), "{warning}");
        assert!(!warning.contains("Nguyễn Văn Nam"), "{warning}");
        assert!(warning.contains("giấy tờ"), "{warning}");
        assert!(
            warning.contains("làm tay"),
            "cảnh báo phải nói rõ form làm tay không nhận: {warning}"
        );
    }

    // ─── Ô ngày nhận phòng, và luật từ chối ───
    //
    // Ngày 06/08/2026 lễ tân gõ "có booking mới phòng 4B, checkin 8 out 9 tháng
    // 8". Trợ lý nhận phòng ngay lúc đó — `check_in_at` trùng `created_at` tới
    // micro-giây. Ngày 8 không đi tới đâu cả.
    //
    // Mọi test dưới đây **seed phòng thật**, kể cả những test chỉ mong một lời
    // từ chối. Phòng không tồn tại thì preview hỏng và test vẫn đỏ ngay cả khi
    // luật ngày bị gỡ sạch — đỏ vì `AGENT_PREVIEW_UNAVAILABLE`, tức đỏ vì một
    // vế khác và không canh gì. Có phòng thì nhánh ngày là thứ **duy nhất**
    // đứng giữa tool call và một cái thẻ.

    /// Ngày nhận đúng hôm nay: đường thường, thẻ dựng bình thường, và hai dòng
    /// ngày trên thẻ nói đúng kỳ ở vừa dùng để hỏi giá.
    #[tokio::test]
    async fn a_draft_for_today_still_builds_the_card() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-today",
            "P801",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let args = serde_json::json!({
            "room_id": "room-today",
            "nights": 2,
            "check_in_date": "2026-06-01",
            "guests": [{ "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" }]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("ngày nhận đúng hôm nay không phải lỗi");

        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };
        assert_eq!(action.kind, CHECK_IN_ACTION_KIND);
        assert_eq!(
            action.display.get("check_in_date").map(String::as_str),
            Some("Hôm nay, 01/06/2026")
        );
        assert_eq!(
            action.display.get("check_out_date").map(String::as_str),
            Some("03/06/2026")
        );
    }

    /// Đường cũ không gãy. Khách đứng ở quầy và không ai nêu ngày nào là ca
    /// thường nhất của một quầy lễ tân; một trường tuỳ chọn không được biến nó
    /// thành một vòng `missing_fields` thừa trong ngân sách 4 vòng.
    #[tokio::test]
    async fn a_draft_without_a_date_still_builds_the_card_the_old_way() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-nodate",
            "P802",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        // Không có `check_in_date` — đúng hình dạng tool call trước bản vá này.
        let args = serde_json::json!({
            "room_id": "room-nodate",
            "nights": 2,
            "guests": [{ "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" }]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("không nêu ngày là ca hợp lệ");

        let action = match outcome {
            DraftOutcome::Ready(action) => action,
            other => panic!("mong đợi Ready, nhận {other:?}"),
        };
        // Vắng ngày thì thẻ vẫn phải nói ra nó đang nhận phòng cho hôm nay —
        // im lặng chính là chỗ con bug cũ trốn được.
        assert_eq!(
            action.display.get("check_in_date").map(String::as_str),
            Some("Hôm nay, 01/06/2026")
        );
    }

    /// Chiều tương lai. Ngày mai **không** dựng được thẻ nhận phòng, và ngày
    /// người dùng nêu đi ra nguyên văn để vòng lặp nhắc lại đúng nó cho model
    /// khi chỉ sang `draft_reserve`.
    #[tokio::test]
    async fn a_draft_for_tomorrow_is_refused_and_points_at_the_reservation_tool() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-tomorrow",
            "P803",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let args = serde_json::json!({
            "room_id": "room-tomorrow",
            "nights": 1,
            "check_in_date": "2026-06-02",
            "guests": [{ "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" }]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("ngày tương lai không phải lỗi hệ thống");

        // Nhánh `Ready` panic ở đây: không có `ProposedAction` nào được dựng.
        match outcome {
            DraftOutcome::WrongDateForCheckIn {
                requested,
                is_future,
            } => {
                assert_eq!(requested, "2026-06-02");
                assert!(is_future, "ngày mai phải được nhận ra là tương lai");
            }
            other => panic!("mong đợi WrongDateForCheckIn, nhận {other:?}"),
        }
    }

    /// Chiều quá khứ — cấm ngang chiều tương lai. Lấy hôm nay thay cho hôm qua
    /// cũng là thay một ngày người dùng đã nêu, và nó ghi sai luôn cả số đêm đã
    /// ở thật.
    #[tokio::test]
    async fn a_draft_for_yesterday_is_refused_as_a_past_date() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-yesterday",
            "P804",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let args = serde_json::json!({
            "room_id": "room-yesterday",
            "nights": 1,
            "check_in_date": "2026-05-31",
            "guests": [{ "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" }]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("ngày quá khứ không phải lỗi hệ thống");

        match outcome {
            DraftOutcome::WrongDateForCheckIn {
                requested,
                is_future,
            } => {
                assert_eq!(requested, "2026-05-31");
                assert!(!is_future, "hôm qua không được coi là tương lai");
            }
            other => panic!("mong đợi WrongDateForCheckIn, nhận {other:?}"),
        }
    }

    /// "ngày 8" là đúng chuỗi model có thể gửi khi nó chép lại lời lễ tân. Đọc
    /// không ra thì từ chối — và **không** rơi về hôm nay.
    ///
    /// Cũng ghim luôn việc nó không bị gộp vào `WrongDateForCheckIn`: gộp thì
    /// `is_future` phải đoán, đoán "quá khứ" sẽ đẩy sang `draft_backfill`, tức
    /// ghi bù cho một kỳ ở còn chưa xảy ra.
    #[tokio::test]
    async fn an_unreadable_date_is_refused_instead_of_falling_back_to_today() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-junkdate",
            "P805",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let args = serde_json::json!({
            "room_id": "room-junkdate",
            "nights": 1,
            "check_in_date": "ngày 8",
            "guests": [{ "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" }]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("ngày rác không phải lỗi hệ thống");

        match outcome {
            DraftOutcome::UnreadableCheckInDate { requested } => {
                assert_eq!(requested, "ngày 8");
            }
            other => panic!("mong đợi UnreadableCheckInDate, nhận {other:?}"),
        }
    }

    /// So theo **ngày lịch**, không so chuỗi và không so timestamp. `2026-6-1`
    /// và `2026-06-01` là cùng một ngày; so chuỗi thì một dấu 0 thiếu biến một
    /// lượt nhận phòng hợp lệ thành lời từ chối, và lễ tân không có cách nào
    /// hiểu vì sao.
    #[tokio::test]
    async fn a_date_written_without_leading_zeros_is_still_today() {
        let pool = test_pool().await;
        seed_room(
            &pool,
            "room-shortdate",
            "P806",
            "Standard Room",
            400_000,
            "vacant",
        )
        .await;

        let args = serde_json::json!({
            "room_id": "room-shortdate",
            "nights": 1,
            "check_in_date": "2026-6-1",
            "guests": [{ "full_name": "Nguyễn Văn Nam", "doc_number": "079201001234" }]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-06-01")
            .await
            .expect("cùng một ngày lịch thì không phải lỗi");

        assert!(
            matches!(outcome, DraftOutcome::Ready(_)),
            "cùng một ngày lịch viết thiếu số 0 vẫn phải dựng được thẻ: {outcome:?}"
        );
    }

    /// **Ca thật đã xảy ra**, ghim chiều nguy hiểm: ngày ở tương lai thì không
    /// có action nào mang ngày hôm nay.
    ///
    /// Không dừng ở "outcome khác `Ready`". Soi cả `Debug` của outcome và bắt
    /// nó không được chứa hôm nay dưới bất kỳ dạng nào — `2026-08-06` (payload,
    /// preview) hay `06/08/2026` (thẻ). Gỡ nhánh từ chối thì `Ready` mang cả
    /// hai chuỗi ấy, và test đỏ ngay tại dòng nói đúng lý do nó tồn tại, chứ
    /// không đỏ nhờ một khẳng định phụ nào khác.
    #[tokio::test]
    async fn a_future_date_never_becomes_a_card_stamped_with_today() {
        let pool = test_pool().await;
        seed_room(&pool, "room-4b", "4B", "Standard Room", 400_000, "vacant").await;

        // Đúng câu lễ tân đã gõ: "có booking mới phòng 4B, checkin 8 out 9
        // tháng 8" — trong khi hôm nay là 06/08/2026.
        let args = serde_json::json!({
            "room_id": "room-4b",
            "nights": 1,
            "check_in_date": "2026-08-08",
            "guests": [{ "full_name": "Hyungchul Lee", "doc_number": "M12345678" }]
        });

        let outcome = build_check_in_draft(&pool, &args, "2026-08-06")
            .await
            .expect("ngày tương lai không phải lỗi hệ thống");

        let dump = format!("{outcome:?}");
        assert!(
            !dump.contains("2026-08-06") && !dump.contains("06/08/2026"),
            "ngày 08/08 bị nuốt mất và hôm nay chui vào kết quả:\n{dump}"
        );
        assert!(
            dump.contains("2026-08-08"),
            "ngày người dùng nêu phải đi tiếp nguyên vẹn:\n{dump}"
        );
        match outcome {
            DraftOutcome::WrongDateForCheckIn { is_future, .. } => {
                assert!(is_future, "08/08 sau 06/08 nên phải là tương lai");
            }
            other => panic!("mong đợi WrongDateForCheckIn, nhận {other:?}"),
        }
    }

    #[test]
    fn nights_become_a_local_date_range() {
        assert_eq!(
            check_out_date_from_nights("2026-08-03", 2).expect("hợp lệ"),
            "2026-08-05"
        );
        assert_eq!(
            check_out_date_from_nights("2026-12-31", 1).expect("hợp lệ"),
            "2027-01-01"
        );
        assert!(check_out_date_from_nights("2026-08-03", 0).is_err());
        assert!(check_out_date_from_nights("khong-phai-ngay", 1).is_err());
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

    /// Cùng công thức seed đã dùng ở `tools.rs`'s `seed_room`: chỉ một dòng
    /// `rooms` là đủ cho `calculate_room_price_preview` chạy được, không cần
    /// `room_types`/`pricing_rules`. `type` cố ý mang tên nhiều từ như thật
    /// (`"Standard Room"`, `"Deluxe Balcony"`) — `rooms.type` là tên hiển thị
    /// có khoảng trắng, một fixture một từ có thể che mất lỗi ghép chuỗi.
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

    /// Phòng có phụ thu thêm người khác 0 — thứ `seed_room` ở trên **không**
    /// dựng được vì nó để schema điền `max_guests`/`extra_person_fee` mặc định
    /// (2, 0). Với mặc định đó, gọi preview kèm số khách hay không kèm đều ra
    /// cùng một số, nên không fixture nào ở trên nhìn thấy được sai lệch.
    async fn seed_room_charging_extra_guests(
        pool: &Pool<Sqlite>,
        id: &str,
        name: &str,
        room_type: &str,
        base_price: i64,
        status: &str,
    ) {
        sqlx::query(
            "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status)
             VALUES (?, ?, ?, 1, 0, ?, 2, 150000, ?)",
        )
        .bind(id)
        .bind(name)
        .bind(room_type)
        .bind(base_price)
        .bind(status)
        .execute(pool)
        .await
        .expect("seed room có phụ thu thêm người");
    }
}
