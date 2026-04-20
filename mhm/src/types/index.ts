export type RoomStatus = "vacant" | "occupied" | "cleaning" | "booked";
export type BookingStatus =
  | "active"
  | "checked_out"
  | "booked"
  | "cancelled"
  | "no_show";
export type BookingSource =
  | "walk-in"
  | "phone"
  | "agoda"
  | "booking.com"
  | "online"
  | "ai-agent";

export interface Room {
  id: string;
  name: string;
  type: string;
  floor: number;
  has_balcony: boolean;
  base_price: number;
  status: RoomStatus;
}

export interface Guest {
  id: string;
  guest_type: string;
  full_name: string;
  doc_number: string;
  dob?: string;
  gender?: string;
  nationality?: string;
  address?: string;
  visa_expiry?: string;
  scan_path?: string;
  phone?: string;
  notes?: string;
  created_at: string;
}

export interface Booking {
  id: string;
  room_id: string;
  primary_guest_id: string;
  check_in_at: string;
  expected_checkout: string;
  actual_checkout?: string;
  nights: number;
  total_price: number;
  paid_amount: number;
  status: BookingStatus;
  source?: BookingSource | null;
  notes?: string;
  created_at: string;
}

export type CheckoutSettlementMode = "actual_nights" | "hourly" | "booked_nights";

export interface CheckoutSettlementPreview {
  settlement_mode: CheckoutSettlementMode;
  settled_nights: number;
  recommended_total: number;
  explanation: string;
}

export interface CheckoutSettlementPayload {
  settlementMode: CheckoutSettlementMode;
  finalTotal: number;
}

export interface RoomWithBooking {
  room: Room;
  booking: Booking | null;
  guests: Guest[];
}

export interface DashboardStats {
  total_rooms: number;
  occupied: number;
  vacant: number;
  cleaning: number;
  revenue_today: number;
}

export interface HousekeepingTask {
  id: string;
  room_id: string;
  status: string;
  note?: string;
  triggered_at: string;
  cleaned_at?: string;
  created_at: string;
}

export interface Expense {
  id: string;
  category: string;
  amount: number;
  note?: string;
  expense_date: string;
  created_at: string;
}

export interface RevenueStats {
  total_revenue: number;
  rooms_sold: number;
  occupancy_rate: number;
  daily_revenue: { date: string; revenue: number }[];
}

export type HotelTab =
  | "dashboard"
  | "rooms"
  | "reservations"
  | "guests"
  | "groups"
  | "housekeeping"
  | "analytics"
  | "settings"
  | "audit";

export interface CheckInGuestInput {
  guest_type?: string;
  full_name: string;
  doc_number: string;
  dob?: string;
  gender?: string;
  nationality?: string;
  address?: string;
  visa_expiry?: string;
  scan_path?: string;
  phone?: string;
}

export interface CccdInfo {
  doc_number: string;
  full_name: string;
  dob: string;
  gender: string;
  nationality: string;
  address: string;
  raw_text: string[];
}

export interface GuestInput {
  full_name: string;
  doc_number: string;
  phone: string;
  dob: string;
  gender: string;
  nationality: string;
  address: string;
}

export interface GuestSummary {
  id: string;
  full_name: string;
  doc_number: string;
  nationality: string | null;
  total_stays: number;
  total_spent: number;
  last_visit: string | null;
}

export type GuestSuggestion = GuestSummary;

export interface AvailabilityResult {
  available: boolean;
  conflicts: { date: string; status: string; guest_name: string; booking_id: string }[];
  max_nights: number | null;
}

export interface EditableBooking {
  id: string;
  room_id: string;
  guest_name: string;
  guest_phone: string | null;
  scheduled_checkin: string | null;
  scheduled_checkout: string | null;
  check_in_at: string;
  expected_checkout: string;
  nights: number;
  total_price: number;
  deposit_amount: number | null;
  source: string | null;
  notes?: string | null;
}

export interface RoomTypeItem {
  id: string;
  name: string;
  created_at: string;
}

export interface ConfigurableRoom extends Room {
  max_guests: number;
  extra_person_fee: number;
}

export interface PricingRuleData {
  room_type: string;
  hourly_rate: number;
  overnight_rate: number;
  daily_rate: number;
  early_checkin_surcharge_pct: number;
  late_checkout_surcharge_pct: number;
  weekend_uplift_pct: number;
}

export interface GatewayStatus {
  running: boolean;
  port: number | null;
  has_api_keys: boolean;
}

export type BackupIndicatorPhase = "saving" | "saved" | "failed";

export type AppUpdatePhase =
  | "idle"
  | "checking"
  | "available"
  | "downloading"
  | "downloaded"
  | "installing"
  | "error";

export interface AppUpdateState {
  phase: AppUpdatePhase;
  currentVersion: string;
  availableVersion: string | null;
  restartPromptOpen: boolean;
  errorMessage: string | null;
}

export type BackupReason =
  | "settings"
  | "checkout"
  | "group_checkout"
  | "night_audit"
  | "app_exit"
  | "manual";

export type BackupStatusState = "started" | "completed" | "failed";

export interface BackupStatusPayload {
  job_id: string;
  state: BackupStatusState;
  reason: BackupReason;
  pending_jobs: number;
  path?: string;
  message?: string;
}

export interface BootstrapStatus {
  setup_completed: boolean;
  app_lock_enabled: boolean;
  current_user: import("@/stores/useAuthStore").User | null;
}

export interface BookingWithGuest {
  id: string;
  room_id: string;
  room_name: string;
  guest_name: string;
  check_in_at: string;
  expected_checkout: string;
  actual_checkout: string | null;
  nights: number;
  total_price: number;
  paid_amount: number;
  status: BookingStatus;
  source: BookingSource | null;
  booking_type: string | null;
  deposit_amount: number | null;
  scheduled_checkin: string | null;
  scheduled_checkout: string | null;
  guest_phone: string | null;
}

export interface ActivityItem {
  icon: string;
  text: string;
  time: string;
  color: string;
  kind?: "check_in" | "check_out" | "housekeeping";
  room_id?: string | null;
  guest_name?: string | null;
  occurred_at?: string;
  status_label?: string;
}

export interface ExpenseItem {
  category: string;
  amount: number;
}

export interface ChartDataPoint {
  name: string;
  revenue: number;
}

export interface RoomAvailability {
  room: { id: string };
  upcoming_reservations: { scheduled_checkin: string }[];
  next_available_until: string | null;
}

export interface AuditLog {
  id: string;
  audit_date: string;
  total_revenue: number;
  room_revenue: number;
  folio_revenue: number;
  total_expenses: number;
  occupancy_pct: number;
  rooms_sold: number;
  total_rooms: number;
  notes?: string;
  created_at: string;
}

export interface AnalyticsData {
  total_revenue: number;
  occupancy_rate: number;
  adr: number;
  revpar: number;
  daily_revenue: { date: string; revenue: number }[];
  revenue_by_source: { name: string; value: number }[];
  expenses_by_category: { category: string; amount: number }[];
  top_rooms: { room: string; revenue: number }[];
}

export type { CrashReportSummary } from "@/lib/crashReporting/types";

// ── Group Booking Types ──

export type GroupStatus = "active" | "partial_checkout" | "completed";

export interface BookingGroup {
  id: string;
  group_name: string;
  master_booking_id: string | null;
  organizer_name: string;
  organizer_phone: string | null;
  total_rooms: number;
  status: GroupStatus;
  notes: string | null;
  created_by: string | null;
  created_at: string;
}

export interface GroupService {
  id: string;
  group_id: string;
  booking_id: string | null;
  name: string;
  quantity: number;
  unit_price: number;
  total_price: number;
  note: string | null;
  created_by: string | null;
  created_at: string;
}

export interface GroupCheckinRequest {
  group_name: string;
  organizer_name: string;
  organizer_phone?: string;
  check_in_date?: string; // "YYYY-MM-DD", undefined = today
  room_ids: string[];
  master_room_id: string;
  guests_per_room: Record<string, CheckInGuestInput[]>;
  nights: number;
  source?: string;
  notes?: string;
  paid_amount?: number;
}

export interface GroupCheckoutRequest {
  group_id: string;
  booking_ids: string[];
  final_paid?: number;
}

export interface AddGroupServiceRequest {
  group_id: string;
  booking_id?: string;
  name: string;
  quantity: number;
  unit_price: number;
  note?: string;
}

export interface GroupDetailResponse {
  group: BookingGroup;
  bookings: BookingWithGuest[];
  services: GroupService[];
  total_room_cost: number;
  total_service_cost: number;
  grand_total: number;
  paid_amount: number;
}

export interface AutoAssignResult {
  assignments: RoomAssignment[];
}

export interface RoomAssignment {
  room: Room;
  floor: number;
}

export interface GroupInvoiceData {
  group: BookingGroup;
  rooms: GroupInvoiceRoomLine[];
  services: GroupService[];
  subtotal_rooms: number;
  subtotal_services: number;
  grand_total: number;
  paid_amount: number;
  balance_due: number;
  hotel_name: string;
  hotel_address: string;
  hotel_phone: string;
}

export interface GroupInvoiceRoomLine {
  room_name: string;
  room_type: string;
  nights: number;
  price_per_night: number;
  total: number;
  guest_name: string;
}
