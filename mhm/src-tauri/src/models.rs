use serde::{Deserialize, Serialize};

// ─── Status Constants (single source of truth) ───

#[allow(dead_code)]
pub mod status {
    pub mod booking {
        pub const ACTIVE: &str = "active";
        pub const CHECKED_OUT: &str = "checked_out";
        pub const BOOKED: &str = "booked";
        pub const CANCELLED: &str = "cancelled";
        pub const NO_SHOW: &str = "no_show";
    }
    pub mod room {
        pub const VACANT: &str = "vacant";
        pub const OCCUPIED: &str = "occupied";
        pub const CLEANING: &str = "cleaning";
        pub const BOOKED: &str = "booked";
    }
    pub mod calendar {
        pub const BOOKED: &str = "booked";
        pub const OCCUPIED: &str = "occupied";
        pub const BLOCKED: &str = "blocked";
        pub const MAINTENANCE: &str = "maintenance";
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Room {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub room_type: String,
    pub floor: i32,
    pub has_balcony: bool,
    pub base_price: f64,
    pub max_guests: i32,
    pub extra_person_fee: f64,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RoomType {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BootstrapStatus {
    pub setup_completed: bool,
    pub app_lock_enabled: bool,
    pub current_user: Option<User>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OnboardingRoomTypeInput {
    pub name: String,
    pub base_price: f64,
    pub max_guests: i32,
    pub extra_person_fee: f64,
    pub default_has_balcony: bool,
    pub bed_note: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OnboardingRoomInput {
    pub id: String,
    pub name: String,
    pub floor: i32,
    pub room_type_name: String,
    pub has_balcony: bool,
    pub base_price: f64,
    pub max_guests: i32,
    pub extra_person_fee: f64,
}

#[derive(Debug, Deserialize)]
pub struct CreateRoomRequest {
    pub id: String,
    pub name: String,
    pub room_type: String,
    pub floor: i32,
    pub has_balcony: bool,
    pub base_price: f64,
    pub max_guests: i32,
    pub extra_person_fee: f64,
}

#[derive(Debug, Deserialize)]
pub struct CreateRoomTypeRequest {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OnboardingHotelInfoInput {
    pub name: String,
    pub address: String,
    pub phone: String,
    pub rating: Option<String>,
    pub default_checkin_time: String,
    pub default_checkout_time: String,
    pub locale: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OnboardingAppLockInput {
    pub enabled: bool,
    pub admin_name: Option<String>,
    pub pin: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OnboardingCompleteRequest {
    pub hotel: OnboardingHotelInfoInput,
    pub room_types: Vec<OnboardingRoomTypeInput>,
    pub rooms: Vec<OnboardingRoomInput>,
    pub app_lock: OnboardingAppLockInput,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Guest {
    pub id: String,
    pub guest_type: String,
    pub full_name: String,
    pub doc_number: String,
    pub dob: Option<String>,
    pub gender: Option<String>,
    pub nationality: Option<String>,
    pub address: Option<String>,
    pub visa_expiry: Option<String>,
    pub scan_path: Option<String>,
    pub phone: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Booking {
    pub id: String,
    pub room_id: String,
    pub primary_guest_id: String,
    pub check_in_at: String,
    pub expected_checkout: String,
    pub actual_checkout: Option<String>,
    pub nights: i32,
    pub total_price: f64,
    pub paid_amount: f64,
    pub status: String,
    pub source: Option<String>,
    pub notes: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Expense {
    pub id: String,
    pub category: String,
    pub amount: f64,
    pub note: Option<String>,
    pub expense_date: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HousekeepingTask {
    pub id: String,
    pub room_id: String,
    pub status: String,
    pub note: Option<String>,
    pub triggered_at: String,
    pub cleaned_at: Option<String>,
    pub created_at: String,
}

// --- Request/Response DTOs ---

#[derive(Debug, Deserialize, Clone)]
pub struct CreateGuestRequest {
    pub guest_type: Option<String>,
    pub full_name: String,
    pub doc_number: String,
    pub dob: Option<String>,
    pub gender: Option<String>,
    pub nationality: Option<String>,
    pub address: Option<String>,
    pub visa_expiry: Option<String>,
    pub scan_path: Option<String>,
    pub phone: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CheckInRequest {
    pub room_id: String,
    pub guests: Vec<CreateGuestRequest>,
    pub nights: i32,
    pub source: Option<String>,
    pub notes: Option<String>,
    pub paid_amount: Option<f64>,
    pub pricing_type: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckoutSettlementMode {
    ActualNights,
    Hourly,
    BookedNights,
}

#[derive(Debug, Deserialize)]
pub struct CheckOutRequest {
    pub booking_id: String,
    pub settlement_mode: CheckoutSettlementMode,
    pub final_total: f64,
}

#[derive(Debug, Deserialize)]
pub struct CheckoutSettlementPreviewRequest {
    pub booking_id: String,
    pub settlement_mode: CheckoutSettlementMode,
}

#[derive(Debug, Serialize)]
pub struct CheckoutSettlementPreview {
    pub settlement_mode: CheckoutSettlementMode,
    pub settled_nights: i32,
    pub recommended_total: f64,
    pub explanation: String,
}

#[derive(Debug, Serialize)]
pub struct RoomWithBooking {
    pub room: Room,
    pub booking: Option<Booking>,
    pub guests: Vec<Guest>,
}

#[derive(Debug, Deserialize)]
pub struct CreateExpenseRequest {
    pub category: String,
    pub amount: f64,
    pub note: Option<String>,
    pub expense_date: String,
}

#[derive(Debug, Serialize)]
pub struct DashboardStats {
    pub total_rooms: i32,
    pub occupied: i32,
    pub vacant: i32,
    pub cleaning: i32,
    pub revenue_today: f64,
}

#[derive(Debug, Serialize)]
pub struct RevenueStats {
    pub total_revenue: f64,
    pub rooms_sold: i32,
    pub occupancy_rate: f64,
    pub daily_revenue: Vec<DailyRevenue>,
}

#[derive(Debug, Serialize, Clone)]
pub struct FolioLine {
    pub id: String,
    pub booking_id: String,
    pub category: String,
    pub description: String,
    pub amount: f64,
    pub created_by: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct AuditLog {
    pub id: String,
    pub audit_date: String,
    pub total_revenue: f64,
    pub room_revenue: f64,
    pub folio_revenue: f64,
    pub total_expenses: f64,
    pub occupancy_pct: f64,
    pub rooms_sold: i32,
    pub total_rooms: i32,
    pub notes: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct NightAuditSnapshot {
    pub audit_date: String,
    pub total_revenue: f64,
    pub room_revenue: f64,
    pub folio_revenue: f64,
    pub total_expenses: f64,
    pub occupancy_pct: f64,
    pub rooms_sold: i32,
    pub total_rooms: i32,
}

#[derive(Debug, Clone)]
pub struct BookingExportRow {
    pub id: String,
    pub room_id: String,
    pub guest_name: String,
    pub doc_number: String,
    pub phone: String,
    pub check_in_at: String,
    pub expected_checkout: String,
    pub actual_checkout: String,
    pub nights: i32,
    pub room_price: f64,
    pub charge_total: f64,
    pub cancellation_fee_total: f64,
    pub folio_total: f64,
    pub recognized_revenue: f64,
    pub paid_amount: f64,
    pub status: String,
    pub pricing_type: String,
    pub source: String,
}

// --- Phase A: New DTOs ---

#[derive(Debug, Serialize)]
pub struct BookingWithGuest {
    pub id: String,
    pub room_id: String,
    pub room_name: String,
    pub guest_name: String,
    pub check_in_at: String,
    pub expected_checkout: String,
    pub actual_checkout: Option<String>,
    pub nights: i32,
    pub total_price: f64,
    pub paid_amount: f64,
    pub status: String,
    pub source: Option<String>,
    pub booking_type: Option<String>,
    pub deposit_amount: Option<f64>,
    pub scheduled_checkin: Option<String>,
    pub scheduled_checkout: Option<String>,
    pub guest_phone: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct BookingFilter {
    pub status: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GuestSummary {
    pub id: String,
    pub full_name: String,
    pub doc_number: String,
    pub nationality: Option<String>,
    pub total_stays: i32,
    pub total_spent: f64,
    pub last_visit: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct BookingWithRoom {
    pub booking_id: String,
    pub room_id: String,
    pub check_in_at: String,
    pub expected_checkout: String,
    pub total_price: f64,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct GuestHistoryResponse {
    pub guest: Guest,
    pub bookings: Vec<BookingWithRoom>,
}

#[derive(Debug, Serialize)]
pub struct SourceRevenue {
    pub name: String,
    pub value: f64,
}

#[derive(Debug, Serialize)]
pub struct CategoryExpense {
    pub category: String,
    pub amount: f64,
}

#[derive(Debug, Serialize)]
pub struct RoomRevenue {
    pub room: String,
    pub revenue: f64,
}

#[derive(Debug, Serialize)]
pub struct AnalyticsData {
    pub total_revenue: f64,
    pub occupancy_rate: f64,
    pub adr: f64,
    pub revpar: f64,
    pub daily_revenue: Vec<DailyRevenue>,
    pub revenue_by_source: Vec<SourceRevenue>,
    pub expenses_by_category: Vec<CategoryExpense>,
    pub top_rooms: Vec<RoomRevenue>,
}

#[derive(Debug, Serialize)]
pub struct DailyRevenue {
    pub date: String,
    pub revenue: f64,
}

#[derive(Debug, Serialize)]
pub struct ActivityItem {
    pub icon: String,
    pub text: String,
    pub time: String,
    pub color: String,
    pub kind: String,
    pub room_id: Option<String>,
    pub guest_name: Option<String>,
    pub occurred_at: String,
    pub status_label: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRoomRequest {
    pub room_id: String,
    pub name: Option<String>,
    pub room_type: Option<String>,
    pub floor: Option<i32>,
    pub has_balcony: Option<bool>,
    pub base_price: Option<f64>,
    pub max_guests: Option<i32>,
    pub extra_person_fee: Option<f64>,
}

// ── Phase 1: Auth & RBAC DTOs ──

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct User {
    pub id: String,
    pub name: String,
    pub role: String,
    pub active: bool,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub pin: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub user: User,
}

#[derive(Debug, Deserialize)]
pub struct CreateUserRequest {
    pub name: String,
    pub pin: String,
    pub role: String,
}

// ── Reservation Calendar DTOs ──

#[derive(Debug, Deserialize)]
pub struct CreateReservationRequest {
    pub room_id: String,
    pub guest_name: String,
    pub guest_phone: Option<String>,
    pub guest_doc_number: Option<String>,
    pub check_in_date: String,
    pub check_out_date: String,
    pub nights: i32,
    pub deposit_amount: Option<f64>,
    pub source: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ModifyReservationRequest {
    pub booking_id: String,
    pub new_check_in_date: String,
    pub new_check_out_date: String,
    pub new_nights: i32,
}

#[derive(Debug, Serialize)]
pub struct AvailabilityResult {
    pub available: bool,
    pub conflicts: Vec<CalendarConflict>,
    pub max_nights: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct CalendarConflict {
    pub date: String,
    pub status: String,
    pub guest_name: Option<String>,
    pub booking_id: String,
}

#[derive(Debug, Serialize)]
pub struct CalendarEntry {
    pub room_id: String,
    pub date: String,
    pub booking_id: Option<String>,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct RoomWithAvailability {
    pub room: Room,
    pub current_booking: Option<Booking>,
    pub upcoming_reservations: Vec<UpcomingReservation>,
    pub next_available_until: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct UpcomingReservation {
    pub booking_id: String,
    pub guest_name: String,
    pub scheduled_checkin: String,
    pub scheduled_checkout: String,
    pub deposit_amount: f64,
    pub status: String,
}

// ── Invoice PDF DTOs ──

#[derive(Debug, Serialize, Clone)]
pub struct InvoiceData {
    pub id: String,
    pub invoice_number: String,
    pub booking_id: String,
    pub hotel_name: String,
    pub hotel_address: String,
    pub hotel_phone: String,
    pub guest_name: String,
    pub guest_phone: Option<String>,
    pub room_name: String,
    pub room_type: String,
    pub check_in: String,
    pub check_out: String,
    pub nights: i32,
    pub pricing_breakdown: Vec<crate::pricing::PricingLine>,
    pub subtotal: f64,
    pub deposit_amount: f64,
    pub total: f64,
    pub balance_due: f64,
    pub policy_text: Option<String>,
    pub notes: Option<String>,
    pub status: String,
    pub created_at: String,
}

// ── Group Booking DTOs ──

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BookingGroup {
    pub id: String,
    pub group_name: String,
    pub master_booking_id: Option<String>,
    pub organizer_name: String,
    pub organizer_phone: Option<String>,
    pub total_rooms: i32,
    pub status: String,
    pub notes: Option<String>,
    pub created_by: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GroupService {
    pub id: String,
    pub group_id: String,
    pub booking_id: Option<String>,
    pub name: String,
    pub quantity: i32,
    pub unit_price: f64,
    pub total_price: f64,
    pub note: Option<String>,
    pub created_by: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct GroupCheckinRequest {
    pub group_name: String,
    pub organizer_name: String,
    pub organizer_phone: Option<String>,
    pub check_in_date: Option<String>, // "YYYY-MM-DD", None = today
    pub room_ids: Vec<String>,
    pub master_room_id: String,
    pub guests_per_room: std::collections::HashMap<String, Vec<CreateGuestRequest>>,
    pub nights: i32,
    pub source: Option<String>,
    pub notes: Option<String>,
    pub paid_amount: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct GroupCheckoutRequest {
    pub group_id: String,
    pub booking_ids: Vec<String>,
    pub final_paid: Option<f64>,
}

#[derive(Debug, Deserialize)]
pub struct AddGroupServiceRequest {
    pub group_id: String,
    pub booking_id: Option<String>,
    pub name: String,
    pub quantity: i32,
    pub unit_price: f64,
    pub note: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GroupDetailResponse {
    pub group: BookingGroup,
    pub bookings: Vec<BookingWithGuest>,
    pub services: Vec<GroupService>,
    pub total_room_cost: f64,
    pub total_service_cost: f64,
    pub grand_total: f64,
    pub paid_amount: f64,
}

#[derive(Debug, Deserialize)]
pub struct AutoAssignRequest {
    pub room_count: i32,
    pub room_type: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AutoAssignResult {
    pub assignments: Vec<RoomAssignment>,
}

#[derive(Debug, Serialize)]
pub struct RoomAssignment {
    pub room: Room,
    pub floor: i32,
}

#[derive(Debug, Serialize, Clone)]
pub struct GroupInvoiceData {
    pub group: BookingGroup,
    pub rooms: Vec<GroupInvoiceRoomLine>,
    pub services: Vec<GroupService>,
    pub subtotal_rooms: f64,
    pub subtotal_services: f64,
    pub grand_total: f64,
    pub paid_amount: f64,
    pub balance_due: f64,
    pub hotel_name: String,
    pub hotel_address: String,
    pub hotel_phone: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct GroupInvoiceRoomLine {
    pub room_name: String,
    pub room_type: String,
    pub nights: i32,
    pub price_per_night: f64,
    pub total: f64,
    pub guest_name: String,
}
