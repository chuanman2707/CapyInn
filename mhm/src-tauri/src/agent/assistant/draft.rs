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

pub fn build_check_in_display(
    payload: &CheckInRequest,
    preview: &Value,
) -> BTreeMap<String, String> {
    let mut display = BTreeMap::new();

    display.insert("room_id".to_string(), payload.room_id.clone());
    display.insert(
        "guests".to_string(),
        payload
            .guests
            .iter()
            .map(|guest| guest.full_name.clone())
            .collect::<Vec<_>>()
            .join(", "),
    );
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

    let warnings = build_warnings(pool, &room_id).await?;
    let display = build_check_in_display(&payload, &preview);

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
async fn build_warnings(pool: &Pool<Sqlite>, room_id: &str) -> CommandResult<Vec<String>> {
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

    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::CreateGuestRequest;

    fn sample_guest(full_name: &str) -> CreateGuestRequest {
        CreateGuestRequest {
            guest_type: None,
            full_name: full_name.to_string(),
            doc_number: String::new(),
            dob: None,
            gender: None,
            nationality: None,
            address: None,
            visa_expiry: None,
            scan_path: None,
            phone: None,
        }
    }

    fn sample_payload() -> CheckInRequest {
        CheckInRequest {
            room_id: "R1".to_string(),
            guests: vec![sample_guest("Nguyễn Văn Nam")],
            nights: 2,
            source: Some("walk-in".to_string()),
            notes: Some("khách quen".to_string()),
            paid_amount: Some(500_000),
            pricing_type: Some("nightly".to_string()),
        }
    }

    /// Đây là luật số một của thiết kế: người dùng duyệt đúng cái sẽ được gửi.
    /// Thêm trường vào CheckInRequest mà quên hiện lên thẻ là test này đỏ.
    #[test]
    fn the_card_shows_every_field_of_the_payload() {
        let payload = sample_payload();
        let preview = serde_json::json!({ "total": 700_000 });

        let display = build_check_in_display(&payload, &preview);

        let encoded = serde_json::to_value(&payload).expect("payload phải serialize được");
        let fields = encoded.as_object().expect("payload là một object");
        for key in fields.keys() {
            assert!(
                display.contains_key(key),
                "trường `{key}` của payload không hiện trên thẻ xác nhận"
            );
        }
    }

    #[test]
    fn the_card_shows_the_preview_total_not_a_model_number() {
        let payload = sample_payload();
        let preview = serde_json::json!({ "total": 700_000 });

        let display = build_check_in_display(&payload, &preview);

        assert!(display
            .get("total")
            .expect("phải có dòng tổng tiền")
            .contains("700"));
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
            Some("Nguyễn Văn Nam")
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

        let args = serde_json::json!({
            "room_id": "room-dirty",
            "nights": 1,
            "guests": [{ "full_name": "Trần Thị Hoa" }]
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
