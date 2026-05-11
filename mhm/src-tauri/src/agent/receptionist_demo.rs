use crate::app_error::{codes, CommandError, CommandResult};
use crate::services::settings_store;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Pool, Row, Sqlite};

pub const DEFAULT_LOCAL_RECEPTIONIST_ENDPOINT: &str = "http://127.0.0.1:8080/v1/chat/completions";
pub const DEFAULT_LOCAL_RECEPTIONIST_MODEL: &str = "capyinn-gemma4-e2b-q5km";
const MAX_MESSAGE_CHARS: usize = 2_000;
const MAX_MODEL_CHARS: usize = 128;
const MAX_ENDPOINT_CHARS: usize = 2_048;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct LocalReceptionistChatRequest {
    pub endpoint: String,
    pub model: String,
    pub message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct LocalReceptionistChatResponse {
    pub answer: String,
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ValidatedLocalReceptionistRequest {
    pub(crate) endpoint: String,
    pub(crate) model: String,
    pub(crate) message: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct GuestFacingContext {
    pub hotel: HotelContext,
    pub checkin_rules: CheckinRulesContext,
    pub room_types: Vec<String>,
    pub pricing_rules: Vec<ReceptionistPricingRule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct HotelContext {
    pub name: String,
    pub address: String,
    pub phone: String,
    pub rating: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CheckinRulesContext {
    pub checkin: String,
    pub checkout: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ReceptionistPricingRule {
    pub room_type: String,
    pub hourly_rate: i64,
    pub overnight_rate: i64,
    pub daily_rate: i64,
    pub overnight_start: String,
    pub overnight_end: String,
    pub daily_checkin: String,
    pub daily_checkout: String,
    pub early_checkin_surcharge_pct: f64,
    pub late_checkout_surcharge_pct: f64,
    pub weekend_uplift_pct: f64,
}

pub(crate) async fn build_guest_facing_context(
    pool: &Pool<Sqlite>,
) -> CommandResult<GuestFacingContext> {
    Ok(GuestFacingContext {
        hotel: load_hotel_context(pool).await?,
        checkin_rules: load_checkin_rules_context(pool).await?,
        room_types: load_room_type_names(pool).await?,
        pricing_rules: load_receptionist_pricing_rules(pool).await?,
    })
}

pub(crate) fn validate_request(
    request: LocalReceptionistChatRequest,
) -> CommandResult<ValidatedLocalReceptionistRequest> {
    let endpoint = request.endpoint.trim();
    let model = request.model.trim();
    let message = request.message.trim();

    validate_local_http_endpoint(endpoint)?;

    if model.is_empty() {
        return Err(validation_error("Model is required"));
    }
    if model.chars().count() > MAX_MODEL_CHARS {
        return Err(validation_error("Model is too long"));
    }
    if model.chars().any(char::is_control) {
        return Err(validation_error("Model contains invalid characters"));
    }

    if message.is_empty() {
        return Err(validation_error("Message is required"));
    }
    if message.chars().count() > MAX_MESSAGE_CHARS {
        return Err(validation_error("Message is too long"));
    }

    Ok(ValidatedLocalReceptionistRequest {
        endpoint: endpoint.to_string(),
        model: model.to_string(),
        message: message.to_string(),
    })
}

fn validate_local_http_endpoint(endpoint: &str) -> CommandResult<()> {
    if endpoint.is_empty() {
        return Err(validation_error("Endpoint is required"));
    }
    if endpoint.chars().count() > MAX_ENDPOINT_CHARS {
        return Err(validation_error("Endpoint is too long"));
    }

    let url = reqwest::Url::parse(endpoint).map_err(|_| validation_error("Endpoint is invalid"))?;
    if url.scheme() != "http" {
        return Err(validation_error("Endpoint must use local HTTP"));
    }

    match url.host_str() {
        Some("127.0.0.1" | "localhost") => Ok(()),
        _ => Err(validation_error("Endpoint must be local")),
    }
}

fn validation_error(message: &'static str) -> CommandError {
    CommandError::user(codes::VALIDATION_INVALID_INPUT, message)
}

async fn load_hotel_context(pool: &Pool<Sqlite>) -> CommandResult<HotelContext> {
    let value = settings_store::get_setting(pool, "hotel_info")
        .await
        .map(parse_json_object)
        .map_err(|error| system_error(format!("failed to load hotel info: {error}")))?;

    Ok(HotelContext {
        name: string_field(&value, "name").unwrap_or_else(|| crate::app_identity::APP_NAME.into()),
        address: string_field(&value, "address").unwrap_or_default(),
        phone: string_field(&value, "phone").unwrap_or_default(),
        rating: string_field(&value, "rating").unwrap_or_default(),
    })
}

async fn load_checkin_rules_context(pool: &Pool<Sqlite>) -> CommandResult<CheckinRulesContext> {
    let value = settings_store::get_setting(pool, "checkin_rules")
        .await
        .map(parse_json_object)
        .map_err(|error| system_error(format!("failed to load check-in rules: {error}")))?;

    Ok(CheckinRulesContext {
        checkin: string_field(&value, "checkin")
            .or_else(|| string_field(&value, "default_checkin_time"))
            .unwrap_or_else(|| "14:00".to_string()),
        checkout: string_field(&value, "checkout")
            .or_else(|| string_field(&value, "default_checkout_time"))
            .unwrap_or_else(|| "12:00".to_string()),
    })
}

async fn load_room_type_names(pool: &Pool<Sqlite>) -> CommandResult<Vec<String>> {
    let rows = sqlx::query("SELECT name FROM room_types ORDER BY name")
        .fetch_all(pool)
        .await
        .map_err(|error| system_error(format!("failed to load room types: {error}")))?;

    Ok(rows
        .iter()
        .map(|row| row.get::<String, _>("name"))
        .collect())
}

async fn load_receptionist_pricing_rules(
    pool: &Pool<Sqlite>,
) -> CommandResult<Vec<ReceptionistPricingRule>> {
    let rows = sqlx::query(
        "SELECT room_type, hourly_rate, overnight_rate, daily_rate,
                overnight_start, overnight_end, daily_checkin, daily_checkout,
                early_checkin_surcharge_pct, late_checkout_surcharge_pct,
                weekend_uplift_pct
         FROM pricing_rules ORDER BY room_type",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| system_error(format!("failed to load pricing rules: {error}")))?;

    Ok(rows
        .iter()
        .map(|row| ReceptionistPricingRule {
            room_type: row.get::<String, _>("room_type"),
            hourly_rate: money_i64(row, "hourly_rate"),
            overnight_rate: money_i64(row, "overnight_rate"),
            daily_rate: money_i64(row, "daily_rate"),
            overnight_start: row.get::<String, _>("overnight_start"),
            overnight_end: row.get::<String, _>("overnight_end"),
            daily_checkin: row.get::<String, _>("daily_checkin"),
            daily_checkout: row.get::<String, _>("daily_checkout"),
            early_checkin_surcharge_pct: number_f64(row, "early_checkin_surcharge_pct"),
            late_checkout_surcharge_pct: number_f64(row, "late_checkout_surcharge_pct"),
            weekend_uplift_pct: number_f64(row, "weekend_uplift_pct"),
        })
        .collect())
}

fn parse_json_object(raw: Option<String>) -> Value {
    raw.and_then(|value| serde_json::from_str::<Value>(&value).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()))
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn money_i64(row: &sqlx::sqlite::SqliteRow, column: &str) -> i64 {
    row.try_get::<i64, _>(column).unwrap_or_else(|_| {
        let value = row.get::<f64, _>(column);
        assert!(
            value.is_finite() && value.fract() == 0.0,
            "money column {column} must be a whole minor-unit amount"
        );
        value as i64
    })
}

fn number_f64(row: &sqlx::sqlite::SqliteRow, column: &str) -> f64 {
    row.try_get::<f64, _>(column)
        .unwrap_or_else(|_| row.get::<i64, _>(column) as f64)
}

fn system_error(message: String) -> CommandError {
    CommandError::system(codes::SYSTEM_INTERNAL_ERROR, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};

    fn valid_request() -> LocalReceptionistChatRequest {
        LocalReceptionistChatRequest {
            endpoint: DEFAULT_LOCAL_RECEPTIONIST_ENDPOINT.to_string(),
            model: DEFAULT_LOCAL_RECEPTIONIST_MODEL.to_string(),
            message: "Do you have hourly rooms?".to_string(),
        }
    }

    fn assert_validation_error(request: LocalReceptionistChatRequest) {
        let error = validate_request(request).expect_err("request must fail validation");
        assert_eq!(error.code, codes::VALIDATION_INVALID_INPUT);
        assert!(error.support_id.is_none());
    }

    async fn test_pool() -> Pool<Sqlite> {
        let database_url = format!(
            "sqlite://file:{}?mode=memory&cache=shared",
            uuid::Uuid::new_v4()
        );

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect(&database_url)
            .await
            .expect("failed to open sqlite test pool");

        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("failed to enable foreign keys");

        crate::db::run_migrations(&pool)
            .await
            .expect("failed to run migrations");

        pool
    }

    async fn save_setting_for_test(pool: &Pool<Sqlite>, key: &str, value: &str) {
        sqlx::query("INSERT INTO settings (key, value) VALUES (?, ?)")
            .bind(key)
            .bind(value)
            .execute(pool)
            .await
            .expect("failed to save setting");
    }

    #[test]
    fn validate_request_accepts_localhost_http_endpoint() {
        let mut request = valid_request();
        request.endpoint = "http://localhost:8080/v1/chat/completions".to_string();

        let validated = validate_request(request).expect("request should pass validation");

        assert_eq!(
            validated.endpoint,
            "http://localhost:8080/v1/chat/completions"
        );
    }

    #[test]
    fn validate_request_rejects_remote_endpoint() {
        let mut request = valid_request();
        request.endpoint = "https://example.com/v1/chat/completions".to_string();

        assert_validation_error(request);
    }

    #[test]
    fn validate_request_rejects_empty_message() {
        let mut request = valid_request();
        request.message = "   ".to_string();

        assert_validation_error(request);
    }

    #[test]
    fn validate_request_rejects_too_long_message() {
        let mut request = valid_request();
        request.message = "a".repeat(MAX_MESSAGE_CHARS + 1);

        assert_validation_error(request);
    }

    #[test]
    fn validate_request_rejects_empty_model() {
        let mut request = valid_request();
        request.model = "   ".to_string();

        assert_validation_error(request);
    }

    #[test]
    fn validate_request_rejects_control_characters_in_model() {
        let mut request = valid_request();
        request.model = "capyinn\nmodel".to_string();

        assert_validation_error(request);
    }

    #[test]
    fn validate_request_rejects_too_long_endpoint() {
        let mut request = valid_request();
        request.endpoint = format!("http://127.0.0.1:8080/{}", "a".repeat(MAX_ENDPOINT_CHARS));

        assert_validation_error(request);
    }

    #[tokio::test]
    async fn guest_context_uses_only_guest_facing_settings_room_types_and_pricing() {
        let pool = test_pool().await;

        save_setting_for_test(
            &pool,
            "hotel_info",
            r#"{
                "name": "CapyInn Test",
                "address": "Da Nang",
                "phone": "0900",
                "rating": "4.7",
                "internal": "secret"
            }"#,
        )
        .await;
        save_setting_for_test(
            &pool,
            "checkin_rules",
            r#"{
                "checkin": "14:00",
                "checkout": "12:00",
                "private_note": "staff only"
            }"#,
        )
        .await;

        sqlx::query("INSERT INTO room_types (id, name, created_at) VALUES (?, ?, ?)")
            .bind("standard")
            .bind("Standard")
            .bind("2026-05-11T00:00:00+07:00")
            .execute(&pool)
            .await
            .expect("failed to insert room type");

        sqlx::query(
            "INSERT INTO pricing_rules
             (id, room_type, hourly_rate, overnight_rate, daily_rate,
              overnight_start, overnight_end, daily_checkin, daily_checkout,
              early_checkin_surcharge_pct, late_checkout_surcharge_pct,
              weekend_uplift_pct, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("price-standard")
        .bind("standard")
        .bind(80_000_i64)
        .bind(300_000_i64)
        .bind(400_000_i64)
        .bind("22:00")
        .bind("11:00")
        .bind("14:00")
        .bind("12:00")
        .bind(30.0_f64)
        .bind(30.0_f64)
        .bind(0.0_f64)
        .bind("2026-05-11T00:00:00+07:00")
        .bind("2026-05-11T00:00:00+07:00")
        .execute(&pool)
        .await
        .expect("failed to insert pricing rule");

        let context = build_guest_facing_context(&pool)
            .await
            .expect("guest context should build");
        let value = serde_json::to_value(context).expect("context should serialize");

        assert_eq!(value["hotel"]["name"], "CapyInn Test");
        assert_eq!(value["hotel"]["address"], "Da Nang");
        assert_eq!(value["hotel"]["phone"], "0900");
        assert_eq!(value["hotel"]["rating"], "4.7");
        assert_eq!(value["checkin_rules"]["checkin"], "14:00");
        assert_eq!(value["checkin_rules"]["checkout"], "12:00");
        assert_eq!(value["room_types"], serde_json::json!(["Standard"]));
        assert_eq!(value["pricing_rules"][0]["room_type"], "standard");
        assert_eq!(value["pricing_rules"][0]["hourly_rate"], 80_000);
        assert_eq!(value["pricing_rules"][0]["overnight_rate"], 300_000);
        assert_eq!(value["pricing_rules"][0]["daily_rate"], 400_000);
        assert_eq!(value["pricing_rules"][0]["overnight_start"], "22:00");
        assert_eq!(value["pricing_rules"][0]["overnight_end"], "11:00");
        assert_eq!(value["pricing_rules"][0]["daily_checkin"], "14:00");
        assert_eq!(value["pricing_rules"][0]["daily_checkout"], "12:00");
        assert_eq!(
            value["pricing_rules"][0]["early_checkin_surcharge_pct"],
            30.0
        );
        assert_eq!(
            value["pricing_rules"][0]["late_checkout_surcharge_pct"],
            30.0
        );
        assert_eq!(value["pricing_rules"][0]["weekend_uplift_pct"], 0.0);

        let serialized = serde_json::to_string(&value).expect("value should serialize");
        assert!(!serialized.contains("internal"));
        assert!(!serialized.contains("private_note"));
        assert!(!serialized.contains("price-standard"));
        assert!(!serialized.contains("created_at"));
    }
}
