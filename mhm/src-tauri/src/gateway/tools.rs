use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use sqlx::{Pool, Sqlite};
use tauri::AppHandle;

use super::models::*;
use crate::app_identity;
use crate::commands;
use crate::models::{BookingFilter, CreateReservationRequest};
use crate::services::settings_store::get_setting;

/// MCP Tool handler — exposes hotel business logic as MCP tools.
/// Each tool delegates to the appropriate shared helper for its data path.
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
    use super::{GetSettingsInput, HotelTools};
    use crate::app_identity;
    use rmcp::handler::server::wrapper::Parameters;
    use serde_json::Value;
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
        match get_setting(&self.pool, &input.key).await {
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
        description = "Get or generate an invoice for a booking. Returns invoice data including pricing breakdown, hotel info, and guest details. Use this to send invoice info to guests via chat."
    )]
    async fn get_invoice(&self, Parameters(input): Parameters<GetInvoiceInput>) -> String {
        // Try to get existing, or generate new
        match commands::do_generate_invoice(&self.pool, &input.booking_id).await {
            Ok(inv) => {
                // Return a human-readable text format for LLM
                let mut text = format!(
                    "=== {} ===\n{}\nPhone: {}\n\nINVOICE {}\nDate: {}\n\nGuest: {}\n",
                    inv.hotel_name,
                    inv.hotel_address,
                    inv.hotel_phone,
                    inv.invoice_number,
                    &inv.created_at[..10],
                    inv.guest_name,
                );
                if let Some(ref phone) = inv.guest_phone {
                    text.push_str(&format!("Phone: {}\n", phone));
                }
                text.push_str(&format!(
                    "\nRoom: {} ({})\nCheck-in: {}\nCheck-out: {}\nNights: {}\n\nPRICE BREAKDOWN\n",
                    inv.room_name,
                    inv.room_type,
                    &inv.check_in[..10],
                    &inv.check_out[..10],
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
