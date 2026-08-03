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
    let preview_result = pricing_service::calculate_room_price_preview(
        pool,
        &room_id,
        now_local_date,
        &check_out,
        &pricing_type,
        Some(guests.len() as i32),
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
}
