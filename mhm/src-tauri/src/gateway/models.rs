use rmcp::schemars::{self, JsonSchema};
use serde::{Deserialize, Serialize};

// ─── MCP Tool Input Schemas ───

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckAvailabilityInput {
    /// Room ID to check (e.g. "1A", "2B")
    pub room_id: String,
    /// Start date in YYYY-MM-DD format
    pub from_date: String,
    /// End date in YYYY-MM-DD format
    pub to_date: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetRoomDetailInput {
    /// Room ID (e.g. "1A", "2B")
    pub room_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetSettingsInput {
    /// Settings key (e.g. "hotel_name", "hotel_address")
    pub key: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetBookingsInput {
    /// Filter by status: "active", "completed", "booked", or omit for all
    pub status: Option<String>,
    /// Filter start date (ISO datetime)
    pub from: Option<String>,
    /// Filter end date (ISO datetime)
    pub to: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CalculatePriceInput {
    /// Room type name (e.g. "standard", "deluxe")
    pub room_type: String,
    /// Check-in datetime (ISO 8601)
    pub check_in: String,
    /// Check-out datetime (ISO 8601)
    pub check_out: String,
    /// Pricing type: "nightly", "hourly", "overnight", "daily"
    pub pricing_type: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CreateReservationInput {
    /// Room ID to reserve
    pub room_id: String,
    /// Guest full name
    pub guest_name: String,
    /// Guest phone number
    pub guest_phone: Option<String>,
    /// Guest ID document number (CCCD/Passport)
    pub guest_doc_number: Option<String>,
    /// Check-in date in YYYY-MM-DD format
    pub check_in_date: String,
    /// Check-out date in YYYY-MM-DD format
    pub check_out_date: String,
    /// Number of nights
    pub nights: i32,
    /// Deposit amount (VND)
    pub deposit_amount: Option<f64>,
    /// Booking source (e.g. "phone", "online", "walk-in")
    pub source: Option<String>,
    /// Additional notes
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CancelReservationInput {
    /// Booking ID to cancel
    pub booking_id: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ModifyReservationInput {
    /// Booking ID to modify
    pub booking_id: String,
    /// New check-in date in YYYY-MM-DD format
    pub new_check_in_date: String,
    /// New check-out date in YYYY-MM-DD format
    pub new_check_out_date: String,
    /// New number of nights
    pub new_nights: i32,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetInvoiceInput {
    /// Booking ID to get invoice for
    pub booking_id: String,
}
