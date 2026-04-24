use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use sqlx::{Pool, Sqlite};
use tauri::AppHandle;

use super::models::*;
use super::policy::{
    guard_write_tool, CANCEL_RESERVATION_META, CREATE_RESERVATION_META, MODIFY_RESERVATION_META,
};
use crate::app_identity;
use crate::commands;
use crate::models::{BookingFilter, CreateReservationRequest, InvoiceData};
use crate::services::settings_store::get_setting;

fn hotel_info_field_from_json(json_str: &str, field: &str) -> Option<String> {
    let field_name = match field {
        "hotel_name" => "name",
        "hotel_address" => "address",
        "hotel_phone" => "phone",
        _ => return None,
    };

    serde_json::from_str::<serde_json::Value>(json_str)
        .ok()?
        .get(field_name)?
        .as_str()
        .map(ToOwned::to_owned)
}

async fn resolve_hotel_info_value(
    pool: &Pool<Sqlite>,
    key: &str,
) -> Result<Option<String>, String> {
    match key {
        "hotel_name" | "hotel_address" | "hotel_phone" => {
            if let Some(json_str) = get_setting(pool, "hotel_info").await? {
                if let Some(value) = hotel_info_field_from_json(&json_str, key) {
                    return Ok(Some(value));
                }
            }

            get_setting(pool, key).await
        }
        "hotel_rules" => get_setting(pool, "hotel_rules").await,
        _ => get_setting(pool, key).await,
    }
}

fn preview_date(value: &str) -> &str {
    value.get(..10).unwrap_or(value)
}

fn format_invoice_text(inv: &InvoiceData) -> String {
    let mut text = format!(
        "=== {} ===\n{}\nPhone: {}\n\nINVOICE {}\nDate: {}\n\nGuest: {}\n",
        inv.hotel_name,
        inv.hotel_address,
        inv.hotel_phone,
        inv.invoice_number,
        preview_date(&inv.created_at),
        inv.guest_name,
    );

    if let Some(ref phone) = inv.guest_phone {
        text.push_str(&format!("Phone: {}\n", phone));
    }

    text.push_str(&format!(
        "\nRoom: {} ({})\nCheck-in: {}\nCheck-out: {}\nNights: {}\n\nPRICE BREAKDOWN\n",
        inv.room_name,
        inv.room_type,
        preview_date(&inv.check_in),
        preview_date(&inv.check_out),
        inv.nights,
    ));

    for line in &inv.pricing_breakdown {
        text.push_str(&format!("  {} -- {}d\n", line.label, line.amount as i64));
    }

    text.push_str(&format!(
        "\nSubtotal: {}d\nDeposit: {}d\nBALANCE DUE: {}d\n",
        inv.total as i64, inv.deposit_amount as i64, inv.balance_due as i64,
    ));

    if let Some(ref policy) = inv.policy_text {
        text.push_str(&format!("\nPolicies:\n{}\n", policy));
    }

    text
}

/// MCP Tool handler — exposes hotel business logic as MCP tools.
/// Each tool delegates to the shared `do_*` functions in `commands.rs`.
#[derive(Clone)]
pub struct HotelTools {
    pub pool: Pool<Sqlite>,
    pub app_handle: Option<AppHandle>,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{
        CancelReservationInput, CreateReservationInput, GetInvoiceInput, GetSettingsInput,
        HotelTools, ModifyReservationInput,
    };
    use crate::app_error::codes;
    use crate::app_identity;
    use crate::commands::invoices;
    use rmcp::handler::server::wrapper::Parameters;
    use serde_json::Value;
    use sqlx::{sqlite::SqlitePoolOptions, Pool, Row, Sqlite};
    use std::ffi::OsString;

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

    async fn seed_room(pool: &Pool<Sqlite>, room_id: &str, room_type: &str) {
        sqlx::query(
            "INSERT INTO rooms (
                id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status
            ) VALUES (?, ?, ?, 1, 0, 250000, 2, 0, 'vacant')",
        )
        .bind(room_id)
        .bind(format!("Room {room_id}"))
        .bind(room_type)
        .execute(pool)
        .await
        .expect("seed room");
    }

    async fn seed_pricing_rule(pool: &Pool<Sqlite>, room_type: &str, daily_rate: f64) {
        let now = "2026-04-15T10:00:00+07:00";

        sqlx::query(
            "INSERT INTO pricing_rules (
                id, room_type, hourly_rate, overnight_rate, daily_rate,
                overnight_start, overnight_end, daily_checkin, daily_checkout,
                early_checkin_surcharge_pct, late_checkout_surcharge_pct,
                weekend_uplift_pct, created_at, updated_at
            ) VALUES (?, ?, 0, 0, ?, '22:00', '11:00', '14:00', '12:00', 0, 0, 0, ?, ?)",
        )
        .bind(format!("rule-{room_type}"))
        .bind(room_type)
        .bind(daily_rate)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .expect("seed pricing rule");
    }

    async fn seed_booked_reservation(pool: &Pool<Sqlite>, booking_id: &str, room_id: &str) {
        let guest_id = format!("guest-{booking_id}");
        let now = "2026-04-15T10:00:00+07:00";
        let phone = "0901234567";
        let check_in = "2026-04-20";
        let check_out = "2026-04-22";
        let deposit = 50_000.0_f64;

        sqlx::query(
            "INSERT INTO guests (
                id, guest_type, full_name, doc_number, dob, gender, nationality,
                address, visa_expiry, scan_path, phone, created_at
            ) VALUES (?, 'domestic', ?, ?, NULL, NULL, NULL, NULL, NULL, NULL, ?, ?)",
        )
        .bind(&guest_id)
        .bind(format!("Reserved Guest {booking_id}"))
        .bind(format!("DOC-{booking_id}"))
        .bind(phone)
        .bind(now)
        .execute(pool)
        .await
        .expect("seed guest");

        sqlx::query(
            "INSERT INTO bookings (
                id, room_id, primary_guest_id, check_in_at, expected_checkout,
                actual_checkout, nights, total_price, paid_amount, status,
                source, notes, created_by, booking_type, pricing_type,
                deposit_amount, guest_phone, scheduled_checkin, scheduled_checkout,
                pricing_snapshot, created_at
            ) VALUES (?, ?, ?, ?, ?, NULL, 2, 500000, ?, 'booked', ?, ?, NULL, 'reservation', 'nightly', ?, ?, ?, ?, NULL, ?)",
        )
        .bind(booking_id)
        .bind(room_id)
        .bind(&guest_id)
        .bind(check_in)
        .bind(check_out)
        .bind(deposit)
        .bind("phone")
        .bind("seed reservation")
        .bind(deposit)
        .bind(phone)
        .bind(check_in)
        .bind(check_out)
        .bind(now)
        .execute(pool)
        .await
        .expect("seed booked reservation");

        sqlx::query("INSERT INTO booking_guests (booking_id, guest_id) VALUES (?, ?)")
            .bind(booking_id)
            .bind(&guest_id)
            .execute(pool)
            .await
            .expect("seed booking guest");

        for date in ["2026-04-20", "2026-04-21"] {
            sqlx::query(
                "INSERT INTO room_calendar (room_id, date, booking_id, status) VALUES (?, ?, ?, 'booked')",
            )
            .bind(room_id)
            .bind(date)
            .bind(booking_id)
            .execute(pool)
            .await
            .expect("seed room calendar");
        }

        sqlx::query(
            "INSERT INTO transactions (id, booking_id, amount, type, note, created_at)
             VALUES (?, ?, ?, 'deposit', ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(booking_id)
        .bind(deposit)
        .bind("Reservation deposit")
        .bind(now)
        .execute(pool)
        .await
        .expect("seed deposit transaction");
    }

    async fn seed_invoice_booking(pool: &Pool<Sqlite>, booking_id: &str) {
        let room_id = "room-invoice";
        let guest_id = "guest-invoice";
        let now = "2026-04-19T10:00:00+07:00";

        seed_room(pool, room_id, "standard").await;

        sqlx::query(
            "INSERT INTO guests (
                id, guest_type, full_name, doc_number, phone, created_at
            ) VALUES (?, 'domestic', ?, ?, ?, ?)",
        )
        .bind(guest_id)
        .bind("Guest 1")
        .bind("DOC-1")
        .bind("0901234567")
        .bind(now)
        .execute(pool)
        .await
        .expect("seed invoice guest");

        sqlx::query(
            "INSERT INTO bookings (
                id, room_id, primary_guest_id, check_in_at, expected_checkout,
                actual_checkout, nights, total_price, paid_amount, status,
                source, notes, created_by, booking_type, pricing_type,
                deposit_amount, guest_phone, scheduled_checkin, scheduled_checkout,
                pricing_snapshot, created_at
            ) VALUES (?, ?, ?, ?, ?, NULL, 1, 250000, 0, 'active', ?, ?, NULL, 'walk-in', 'nightly', NULL, ?, NULL, NULL, NULL, ?)",
        )
        .bind(booking_id)
        .bind(room_id)
        .bind(guest_id)
        .bind(now)
        .bind("2026-04-20T10:00:00+07:00")
        .bind("walk-in")
        .bind("seed booking")
        .bind("0901234567")
        .bind(now)
        .execute(pool)
        .await
        .expect("seed invoice booking");
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }

        fn remove(key: &'static str) -> Self {
            let previous = std::env::var_os(key);
            std::env::remove_var(key);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[tokio::test]
    async fn get_hotel_context_falls_back_when_hotel_info_is_missing() {
        let pool = test_pool().await;
        let tools = HotelTools::new(pool, None);

        let context: Value = serde_json::from_str(&tools.get_hotel_context().await)
            .expect("context should be valid json");

        assert_eq!(context["hotel_name"].as_str(), Some(app_identity::APP_NAME));
        assert_eq!(context["hotel_address"].as_str(), Some(""));
    }

    #[tokio::test]
    async fn get_hotel_context_falls_back_when_hotel_info_is_malformed() {
        let pool = test_pool().await;
        sqlx::query("INSERT INTO settings (key, value) VALUES ('hotel_info', '{not-json')")
            .execute(&pool)
            .await
            .expect("seed malformed setting");

        let tools = HotelTools::new(pool, None);
        let context: Value = serde_json::from_str(&tools.get_hotel_context().await)
            .expect("context should be valid json");

        assert_eq!(context["hotel_name"].as_str(), Some(app_identity::APP_NAME));
        assert_eq!(context["hotel_address"].as_str(), Some(""));
    }

    #[tokio::test]
    async fn get_hotel_info_returns_missing_setting_message_when_absent() {
        let pool = test_pool().await;
        let tools = HotelTools::new(pool, None);

        let output = tools
            .get_hotel_info(Parameters(GetSettingsInput {
                key: "missing_key".to_string(),
            }))
            .await;

        assert_eq!(output, "Setting 'missing_key' not found");
    }

    #[tokio::test]
    async fn get_hotel_info_returns_error_string_when_settings_table_is_missing() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("failed to open sqlite test pool");
        let tools = HotelTools::new(pool, None);

        let output = tools
            .get_hotel_info(Parameters(GetSettingsInput {
                key: "hotel_info".to_string(),
            }))
            .await;

        assert!(output.starts_with("Error: "));
    }

    #[tokio::test]
    async fn get_hotel_info_returns_documented_aliases_from_hotel_info_json() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES ('hotel_info', ?), ('hotel_rules', ?)",
        )
        .bind(r#"{"name":"Alias Hotel","address":"42 River Rd","phone":"0909999999"}"#)
        .bind("No smoking after 22:00")
        .execute(&pool)
        .await
        .expect("seed settings");

        let tools = HotelTools::new(pool, None);

        let hotel_name = tools
            .get_hotel_info(Parameters(GetSettingsInput {
                key: "hotel_name".to_string(),
            }))
            .await;
        let hotel_address = tools
            .get_hotel_info(Parameters(GetSettingsInput {
                key: "hotel_address".to_string(),
            }))
            .await;
        let hotel_phone = tools
            .get_hotel_info(Parameters(GetSettingsInput {
                key: "hotel_phone".to_string(),
            }))
            .await;
        let hotel_rules = tools
            .get_hotel_info(Parameters(GetSettingsInput {
                key: "hotel_rules".to_string(),
            }))
            .await;

        assert_eq!(hotel_name, "Alias Hotel");
        assert_eq!(hotel_address, "42 River Rd");
        assert_eq!(hotel_phone, "0909999999");
        assert_eq!(hotel_rules, "No smoking after 22:00");
    }

    #[tokio::test]
    async fn get_hotel_info_aliases_fall_back_to_raw_settings_when_json_is_missing() {
        let pool = test_pool().await;
        sqlx::query("INSERT INTO settings (key, value) VALUES ('hotel_name', 'Fallback Hotel')")
            .execute(&pool)
            .await
            .expect("seed fallback setting");

        let tools = HotelTools::new(pool, None);

        let hotel_name = tools
            .get_hotel_info(Parameters(GetSettingsInput {
                key: "hotel_name".to_string(),
            }))
            .await;

        assert_eq!(hotel_name, "Fallback Hotel");
    }

    #[tokio::test]
    async fn get_hotel_info_aliases_fall_back_to_raw_settings_when_json_is_malformed() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES ('hotel_info', '{not-json'), ('hotel_phone', '0901111111')",
        )
        .execute(&pool)
        .await
        .expect("seed malformed hotel info");

        let tools = HotelTools::new(pool, None);

        let hotel_phone = tools
            .get_hotel_info(Parameters(GetSettingsInput {
                key: "hotel_phone".to_string(),
            }))
            .await;

        assert_eq!(hotel_phone, "0901111111");
    }

    #[tokio::test]
    async fn create_reservation_is_default_denied_without_creating_booking() {
        let _env_lock = crate::runtime_config::env_lock().lock().unwrap();
        let _env = EnvVarGuard::remove("CAPYINN_ENABLE_HIGH_RISK_MCP_WRITES");

        let pool = test_pool().await;
        seed_room(&pool, "R199", "standard").await;
        seed_pricing_rule(&pool, "standard", 600_000.0).await;
        let tools = HotelTools::new(pool.clone(), None);

        let output = tools
            .create_reservation(Parameters(CreateReservationInput {
                room_id: "R199".to_string(),
                guest_name: "Nguyen Van A".to_string(),
                guest_phone: Some("0900000000".to_string()),
                guest_doc_number: Some("079123456789".to_string()),
                check_in_date: "2026-04-20".to_string(),
                check_out_date: "2026-04-22".to_string(),
                nights: 2,
                deposit_amount: Some(50_000.0),
                source: Some("phone".to_string()),
                notes: Some("test reservation".to_string()),
            }))
            .await;

        let envelope: Value = serde_json::from_str(&output).expect("error envelope json");
        assert_eq!(envelope["ok"].as_bool(), Some(false));
        assert_eq!(
            envelope["error"]["code"].as_str(),
            Some(codes::WRITE_TOOL_DISABLED)
        );
        assert_eq!(envelope["error"]["kind"].as_str(), Some("policy"));

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM bookings WHERE room_id = ?")
            .bind("R199")
            .fetch_one(&pool)
            .await
            .expect("booking count");
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn get_hotel_context_still_works_when_high_risk_writes_are_disabled() {
        let _env_lock = crate::runtime_config::env_lock().lock().unwrap();
        let _env = EnvVarGuard::remove("CAPYINN_ENABLE_HIGH_RISK_MCP_WRITES");

        let pool = test_pool().await;
        let tools = HotelTools::new(pool, None);

        let context: Value = serde_json::from_str(&tools.get_hotel_context().await)
            .expect("context should be valid json");

        assert_eq!(context["hotel_name"].as_str(), Some(app_identity::APP_NAME));
        assert!(context["current_date"].as_str().is_some());
    }

    #[tokio::test]
    async fn create_reservation_tool_returns_booking_json() {
        let _env_lock = crate::runtime_config::env_lock().lock().unwrap();
        let _env = EnvVarGuard::set("CAPYINN_ENABLE_HIGH_RISK_MCP_WRITES", "1");

        let pool = test_pool().await;
        seed_room(&pool, "R200", "standard").await;
        seed_pricing_rule(&pool, "standard", 600_000.0).await;
        let tools = HotelTools::new(pool.clone(), None);

        let output = tools
            .create_reservation(Parameters(CreateReservationInput {
                room_id: "R200".to_string(),
                guest_name: "Nguyen Van A".to_string(),
                guest_phone: Some("0900000000".to_string()),
                guest_doc_number: Some("079123456789".to_string()),
                check_in_date: "2026-04-20".to_string(),
                check_out_date: "2026-04-22".to_string(),
                nights: 2,
                deposit_amount: Some(50_000.0),
                source: Some("phone".to_string()),
                notes: Some("test reservation".to_string()),
            }))
            .await;

        let booking: Value = serde_json::from_str(&output).expect("booking json");
        assert_eq!(booking["room_id"].as_str(), Some("R200"));
        assert_eq!(booking["status"].as_str(), Some("booked"));

        let stored: String = sqlx::query("SELECT status FROM bookings WHERE room_id = ?")
            .bind("R200")
            .fetch_one(&pool)
            .await
            .expect("booking row")
            .get("status");
        assert_eq!(stored, "booked");
    }

    #[tokio::test]
    async fn cancel_reservation_tool_returns_success_message_and_updates_status() {
        let _env_lock = crate::runtime_config::env_lock().lock().unwrap();
        let _env = EnvVarGuard::set("CAPYINN_ENABLE_HIGH_RISK_MCP_WRITES", "1");

        let pool = test_pool().await;
        seed_room(&pool, "R201", "standard").await;
        seed_booked_reservation(&pool, "B201", "R201").await;
        let tools = HotelTools::new(pool.clone(), None);

        let output = tools
            .cancel_reservation(Parameters(CancelReservationInput {
                booking_id: "B201".to_string(),
            }))
            .await;

        assert_eq!(output, "Reservation B201 cancelled successfully");

        let stored: String = sqlx::query("SELECT status FROM bookings WHERE id = ?")
            .bind("B201")
            .fetch_one(&pool)
            .await
            .expect("booking row")
            .get("status");
        assert_eq!(stored, "cancelled");
    }

    #[tokio::test]
    async fn modify_reservation_tool_returns_updated_booking_json() {
        let _env_lock = crate::runtime_config::env_lock().lock().unwrap();
        let _env = EnvVarGuard::set("CAPYINN_ENABLE_HIGH_RISK_MCP_WRITES", "1");

        let pool = test_pool().await;
        seed_room(&pool, "R202", "standard").await;
        seed_pricing_rule(&pool, "standard", 600_000.0).await;
        seed_booked_reservation(&pool, "B202", "R202").await;
        let tools = HotelTools::new(pool.clone(), None);

        let output = tools
            .modify_reservation(Parameters(ModifyReservationInput {
                booking_id: "B202".to_string(),
                new_check_in_date: "2026-04-24".to_string(),
                new_check_out_date: "2026-04-26".to_string(),
                new_nights: 2,
            }))
            .await;

        let booking: Value = serde_json::from_str(&output).expect("booking json");
        assert_eq!(booking["status"].as_str(), Some("booked"));
        assert_eq!(booking["check_in_at"].as_str(), Some("2026-04-24"));
        assert_eq!(booking["expected_checkout"].as_str(), Some("2026-04-26"));
    }

    #[tokio::test]
    async fn get_invoice_returns_existing_invoice_text_without_creating_new_rows() {
        let pool = test_pool().await;
        seed_invoice_booking(&pool, "booking-1").await;
        let generated = invoices::do_generate_invoice(&pool, "booking-1")
            .await
            .expect("generate invoice");
        let tools = HotelTools::new(pool.clone(), None);

        let output = tools
            .get_invoice(Parameters(GetInvoiceInput {
                booking_id: "booking-1".to_string(),
            }))
            .await;

        assert!(output.contains(&generated.invoice_number));
        assert!(output.contains("Guest: Guest 1"));

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM invoices WHERE booking_id = ?")
            .bind("booking-1")
            .fetch_one(&pool)
            .await
            .expect("invoice count");
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn get_invoice_returns_not_found_without_creating_invoice() {
        let pool = test_pool().await;
        seed_invoice_booking(&pool, "booking-2").await;
        let tools = HotelTools::new(pool.clone(), None);

        let output = tools
            .get_invoice(Parameters(GetInvoiceInput {
                booking_id: "booking-2".to_string(),
            }))
            .await;

        assert_eq!(output, "Invoice for booking 'booking-2' not found");

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM invoices WHERE booking_id = ?")
            .bind("booking-2")
            .fetch_one(&pool)
            .await
            .expect("invoice count");
        assert_eq!(count, 0);
    }
}
impl HotelTools {
    pub fn new(pool: Pool<Sqlite>, app_handle: Option<AppHandle>) -> Self {
        Self {
            pool,
            app_handle,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl HotelTools {
    // ─── Read Tools (11) ───

    #[tool(
        description = "Get the current date/time, timezone, and hotel context. ALWAYS call this first to ground your responses in reality and avoid date hallucinations."
    )]
    async fn get_hotel_context(&self) -> String {
        let now = chrono::Local::now();

        let (hotel_name, hotel_address) =
            match get_setting(&self.pool, "hotel_info").await.unwrap_or(None) {
                Some(json_str) => {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json_str) {
                        (
                            v.get("name")
                                .and_then(|s| s.as_str())
                                .unwrap_or(app_identity::APP_NAME)
                                .to_string(),
                            v.get("address")
                                .and_then(|s| s.as_str())
                                .unwrap_or("")
                                .to_string(),
                        )
                    } else {
                        (app_identity::APP_NAME.to_string(), String::new())
                    }
                }
                None => (app_identity::APP_NAME.to_string(), String::new()),
            };

        let context = serde_json::json!({
            "current_datetime": now.to_rfc3339(),
            "current_date": now.format("%Y-%m-%d").to_string(),
            "current_time": now.format("%H:%M:%S").to_string(),
            "timezone": "Asia/Ho_Chi_Minh",
            "hotel_name": hotel_name,
            "hotel_address": hotel_address,
        });

        serde_json::to_string_pretty(&context).unwrap()
    }

    #[tool(
        description = "Check room availability for a specific date range. Returns conflicts if any."
    )]
    async fn check_availability(
        &self,
        Parameters(input): Parameters<CheckAvailabilityInput>,
    ) -> String {
        match commands::do_check_availability(
            &self.pool,
            &input.room_id,
            &input.from_date,
            &input.to_date,
        )
        .await
        {
            Ok(result) => serde_json::to_string_pretty(&result).unwrap(),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(
        description = "Get list of all rooms with their current status (vacant, occupied, cleaning, booked)."
    )]
    async fn get_rooms(&self) -> String {
        match commands::do_get_rooms(&self.pool).await {
            Ok(rooms) => serde_json::to_string_pretty(&rooms).unwrap(),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(
        description = "Get detailed info for a specific room including current booking and guests."
    )]
    async fn get_room_detail(&self, Parameters(input): Parameters<GetRoomDetailInput>) -> String {
        match commands::do_get_room_detail(&self.pool, &input.room_id).await {
            Ok(detail) => serde_json::to_string_pretty(&detail).unwrap(),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get all room types and their names (e.g. standard, deluxe).")]
    async fn get_room_types(&self) -> String {
        match commands::do_get_room_types(&self.pool).await {
            Ok(types) => serde_json::to_string_pretty(&types).unwrap(),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(
        description = "Get hotel dashboard statistics: total rooms, occupied, vacant, cleaning, revenue today."
    )]
    async fn get_dashboard_stats(&self) -> String {
        match commands::do_get_dashboard_stats(&self.pool).await {
            Ok(stats) => serde_json::to_string_pretty(&stats).unwrap(),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(description = "Get all bookings with optional filters by status, date range.")]
    async fn get_all_bookings(&self, Parameters(input): Parameters<GetBookingsInput>) -> String {
        let filter = if input.status.is_some() || input.from.is_some() || input.to.is_some() {
            Some(BookingFilter {
                status: input.status,
                from: input.from,
                to: input.to,
            })
        } else {
            None
        };

        match commands::do_get_all_bookings(&self.pool, filter).await {
            Ok(bookings) => serde_json::to_string_pretty(&bookings).unwrap(),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(
        description = "Get all room availability overview including upcoming reservations for each room."
    )]
    async fn get_rooms_availability(&self) -> String {
        match commands::do_get_rooms_availability(&self.pool).await {
            Ok(rooms) => serde_json::to_string_pretty(&rooms).unwrap(),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(
        description = "Get pricing rules for all room types (hourly, overnight, daily rates and surcharges)."
    )]
    async fn get_pricing_rules(&self) -> String {
        match commands::do_get_pricing_rules(&self.pool).await {
            Ok(rules) => serde_json::to_string_pretty(&rules).unwrap(),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(
        description = "Get a hotel setting by key. Common keys: hotel_name, hotel_address, hotel_phone, hotel_rules."
    )]
    async fn get_hotel_info(&self, Parameters(input): Parameters<GetSettingsInput>) -> String {
        match resolve_hotel_info_value(&self.pool, &input.key).await {
            Ok(Some(value)) => value,
            Ok(None) => format!("Setting '{}' not found", input.key),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(
        description = "Calculate estimated price for a stay. Supports nightly, hourly, overnight, and daily pricing types."
    )]
    async fn calculate_price(&self, Parameters(input): Parameters<CalculatePriceInput>) -> String {
        match commands::do_calculate_price_preview(
            &self.pool,
            &input.room_type,
            &input.check_in,
            &input.check_out,
            &input.pricing_type,
        )
        .await
        {
            Ok(result) => serde_json::to_string_pretty(&result).unwrap(),
            Err(e) => format!("Error: {}", e),
        }
    }

    // ─── Write Tools (3) ───

    #[tool(
        description = "Create a new reservation (booking with status 'booked'). The reservation must be confirmed by hotel staff before check-in."
    )]
    async fn create_reservation(
        &self,
        Parameters(input): Parameters<CreateReservationInput>,
    ) -> String {
        if let Err(envelope) = guard_write_tool(&CREATE_RESERVATION_META) {
            return envelope.to_json_string();
        }

        let req = CreateReservationRequest {
            room_id: input.room_id,
            guest_name: input.guest_name,
            guest_phone: input.guest_phone,
            guest_doc_number: input.guest_doc_number,
            check_in_date: input.check_in_date,
            check_out_date: input.check_out_date,
            nights: input.nights,
            deposit_amount: input.deposit_amount,
            source: input.source.or(Some("ai-agent".to_string())),
            notes: input.notes,
        };

        match commands::do_create_reservation(&self.pool, self.app_handle.as_ref(), req).await {
            Ok(booking) => {
                if let Some(ref handle) = self.app_handle {
                    use tauri::Emitter;
                    let _ = handle.emit(
                        "mcp_reservation_created",
                        serde_json::json!({
                            "booking_id": booking.id,
                            "room_id": booking.room_id,
                        }),
                    );
                }

                serde_json::to_string_pretty(&booking).unwrap()
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(
        description = "Cancel an existing reservation. Only reservations with status 'booked' can be cancelled."
    )]
    async fn cancel_reservation(
        &self,
        Parameters(input): Parameters<CancelReservationInput>,
    ) -> String {
        if let Err(envelope) = guard_write_tool(&CANCEL_RESERVATION_META) {
            return envelope.to_json_string();
        }

        match commands::do_cancel_reservation(
            &self.pool,
            self.app_handle.as_ref(),
            &input.booking_id,
        )
        .await
        {
            Ok(()) => format!("Reservation {} cancelled successfully", input.booking_id),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(
        description = "Modify an existing reservation's dates. Only reservations with status 'booked' can be modified."
    )]
    async fn modify_reservation(
        &self,
        Parameters(input): Parameters<ModifyReservationInput>,
    ) -> String {
        if let Err(envelope) = guard_write_tool(&MODIFY_RESERVATION_META) {
            return envelope.to_json_string();
        }

        let req = crate::models::ModifyReservationRequest {
            booking_id: input.booking_id,
            new_check_in_date: input.new_check_in_date,
            new_check_out_date: input.new_check_out_date,
            new_nights: input.new_nights,
        };

        match commands::do_modify_reservation(&self.pool, self.app_handle.as_ref(), req).await {
            Ok(booking) => serde_json::to_string_pretty(&booking).unwrap(),
            Err(e) => format!("Error: {}", e),
        }
    }

    #[tool(
        description = "Get the most recently issued invoice for a booking. Returns a human-readable invoice summary if one already exists."
    )]
    async fn get_invoice(&self, Parameters(input): Parameters<GetInvoiceInput>) -> String {
        match commands::do_get_invoice(&self.pool, &input.booking_id).await {
            Ok(Some(inv)) => format_invoice_text(&inv),
            Ok(None) => format!("Invoice for booking '{}' not found", input.booking_id),
            Err(e) => format!("Error: {}", e),
        }
    }
}

#[tool_handler]
impl ServerHandler for HotelTools {
    fn get_info(&self) -> ServerInfo {
        let mut caps = ServerCapabilities::default();
        caps.tools = Some(ToolsCapability::default());

        ServerInfo::new(caps)
            .with_server_info(Implementation::new("capyinn", "0.1.0"))
            .with_instructions(
                "CapyInn MCP Server. Provides tools to query room availability, \
                 pricing, bookings, and create/modify/cancel reservations. \
                 ALWAYS call get_hotel_context first to get the current date/time.",
            )
    }
}
