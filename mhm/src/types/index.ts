import type { MoneyVnd } from "@/lib/money";

export type RoomStatus = "vacant" | "occupied" | "cleaning" | "booked";
export type BookingStatus =
  | "active"
  | "checked_out"
  | "booked"
  | "cancelled"
  | "no_show"
  | "voided";
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
  base_price: MoneyVnd;
  max_guests: number;
  extra_person_fee: MoneyVnd;
  status: RoomStatus;
}

/**
 * Giá niêm yết của một **loại phòng**, do `get_room_type_rates` trả về.
 *
 * Không phải báo giá: uplift cuối tuần, `special_dates` và phụ thu thêm người
 * đều phụ thuộc ngày và số khách. Con số gắn với một kỳ nghỉ cụ thể phải đi qua
 * lệnh xem trước (`usePricePreview` / `useRoomPrices`).
 */
export interface RoomTypeRate {
  room_type: string;
  nightly_rate: MoneyVnd;
  /** `false` = chưa có bảng giá cho loại này, số trên là suy ra chứ không ai đặt. */
  configured: boolean;
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
  total_price: MoneyVnd;
  paid_amount: MoneyVnd;
  status: BookingStatus;
  source?: BookingSource | null;
  notes?: string;
  created_at: string;
  /** Thời điểm gần nhất giá/đêm bị lễ tân đổi tay; null nếu chưa từng đổi. */
  rate_overridden_at?: string | null;
}

export interface RoomChangeOption {
  roomId: string;
  name: string;
  roomType: string;
  floor: number;
  maxGuests: number;
  priceDifference: MoneyVnd;
}

export interface RoomChangeOptions {
  bookingId: string;
  currentRoomId: string;
  currentRoomName: string;
  fromDate: string;
  toDate: string;
  nightsRemaining: number;
  nightsStayed: number;
  guestCount: number;
  rooms: RoomChangeOption[];
}

export type CheckoutSettlementMode = "actual_nights" | "hourly" | "booked_nights";

export interface CheckoutSettlementPreview {
  settlement_mode: CheckoutSettlementMode;
  settled_nights: number;
  recommended_total: MoneyVnd;
  explanation: string;
}

export interface CheckoutSettlementPayload {
  settlementMode: CheckoutSettlementMode;
  finalTotal: MoneyVnd;
}

export interface RoomWithBooking {
  room: Room;
  booking: Booking | null;
  guests: Guest[];
  /** group_id của lượt đang ở (nếu có); null khi không thuộc đoàn hoặc không có lượt nào. */
  group_id?: string | null;
}

export interface DashboardStats {
  total_rooms: number;
  occupied: number;
  vacant: number;
  cleaning: number;
  revenue_today: MoneyVnd;
}

export interface Expense {
  id: string;
  category: string;
  amount: MoneyVnd;
  note?: string;
  expense_date: string;
  created_at: string;
}

export interface RevenueStats {
  total_revenue: MoneyVnd;
  rooms_sold: number;
  occupancy_rate: number;
  daily_revenue: { date: string; revenue: MoneyVnd }[];
}

export type HotelTab =
  | "dashboard"
  | "rooms"
  | "reservations"
  | "guests"
  | "groups"
  | "analytics"
  | "settings"
  | "declaration"
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
  total_spent: MoneyVnd;
  last_visit: string | null;
}

export type GuestSuggestion = GuestSummary;

export interface AvailabilityResult {
  available: boolean;
  conflicts: { date: string; status: string; guest_name: string; booking_id: string }[];
  max_nights: number | null;
}

export interface PricingLine {
  label: string;
  amount: MoneyVnd;
}

export interface PricingResult {
  pricing_type: string;
  base_amount: MoneyVnd;
  surcharge_amount: MoneyVnd;
  weekend_amount: MoneyVnd;
  total: MoneyVnd;
  breakdown: PricingLine[];
  capped: boolean;
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
  guests: number | null;
  total_price: MoneyVnd;
  deposit_amount: MoneyVnd | null;
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
  extra_person_fee: MoneyVnd;
}

export interface PricingRuleData {
  room_type: string;
  hourly_rate: MoneyVnd;
  overnight_rate: MoneyVnd;
  daily_rate: MoneyVnd;
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
  supported: boolean;
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
  | "manual"
  | "scheduled";

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
  total_price: MoneyVnd;
  paid_amount: MoneyVnd;
  status: BookingStatus;
  source: BookingSource | null;
  booking_type: string | null;
  deposit_amount: MoneyVnd | null;
  scheduled_checkin: string | null;
  scheduled_checkout: string | null;
  guest_phone: string | null;
  guests: number | null;
  group_id: string | null;
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
  amount: MoneyVnd;
}

export interface ChartDataPoint {
  name: string;
  revenue: MoneyVnd;
}

export interface RoomAvailability {
  room: { id: string };
  upcoming_reservations: { scheduled_checkin: string }[];
  next_available_until: string | null;
}

export interface AuditLog {
  id: string;
  audit_date: string;
  total_revenue: MoneyVnd;
  room_revenue: MoneyVnd;
  folio_revenue: MoneyVnd;
  total_expenses: MoneyVnd;
  occupancy_pct: number;
  rooms_sold: number;
  total_rooms: number;
  notes?: string;
  created_at: string;
}

export interface AnalyticsData {
  total_revenue: MoneyVnd;
  occupancy_rate: number;
  adr: number;
  revpar: number;
  daily_revenue: { date: string; revenue: MoneyVnd }[];
  revenue_by_source: { name: string; value: MoneyVnd }[];
  expenses_by_category: { category: string; amount: MoneyVnd }[];
  top_rooms: { room: string; revenue: MoneyVnd }[];
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
  unit_price: MoneyVnd;
  total_price: MoneyVnd;
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
  paid_amount?: MoneyVnd;
  /**
   * Giá mỗi đêm gõ tay theo TỪNG phòng. Khoá là room_id. Phòng không có
   * trong map ⇒ engine tính. `group_checkin_tx` từ chối cả giao dịch nếu map
   * chứa một khoá không nằm trong `room_ids` — luôn gửi map, kể cả rỗng,
   * không gửi `undefined`.
   */
  rate_override_per_room: Record<string, MoneyVnd>;
}

export interface GroupCheckoutRequest {
  group_id: string;
  booking_ids: string[];
  final_paid?: MoneyVnd;
}

export interface AddGroupServiceRequest {
  group_id: string;
  booking_id?: string;
  name: string;
  quantity: number;
  unit_price: MoneyVnd;
  note?: string;
}

export interface GroupDetailResponse {
  group: BookingGroup;
  bookings: BookingWithGuest[];
  services: GroupService[];
  total_room_cost: MoneyVnd;
  total_service_cost: MoneyVnd;
  grand_total: MoneyVnd;
  paid_amount: MoneyVnd;
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
  subtotal_rooms: MoneyVnd;
  subtotal_services: MoneyVnd;
  grand_total: MoneyVnd;
  paid_amount: MoneyVnd;
  balance_due: MoneyVnd;
  hotel_name: string;
  hotel_address: string;
  hotel_phone: string;
}

export interface GroupInvoiceRoomLine {
  room_name: string;
  room_type: string;
  nights: number;
  price_per_night: MoneyVnd;
  total: MoneyVnd;
  guest_name: string;
}

// ── Khai báo tạm trú ──
// Shapes mirror `declaration_identity` / `declaration_link` / `declaration_batch`
// in the design spec §5. Field names stay snake_case because they come straight
// off the Rust structs.

export type DeclarationSource = "qr_cccd" | "mrz_td3" | "manual";
export type DeclarationConfidence = "verified" | "needs_review";
export type DeclarationBatchKind = "NNN" | "VN";
export type DeclarationSeverity = "blocking" | "warning";
export type DeclarationBatchStatus = "exported" | "uploaded" | "verified" | "failed";
export type DeclarationDocTypeSource = "heuristic" | "human";

export interface DeclarationIdentity {
  id: string;
  full_name: string;
  dob: string;
  gender: string;
  nationality_iso3: string;
  // khách Việt Nam
  doc_type_code?: string | null;
  doc_type_source?: DeclarationDocTypeSource | null;
  doc_type_name?: string | null;
  doc_no?: string | null;
  phone?: string | null;
  residence_status?: string | null;
  address_detail?: string | null;
  // khách nước ngoài
  passport_no?: string | null;
  passport_expiry?: string | null;
  /** Nhập tay. KHÁC `passport_expiry` — xem §8.1 E10. */
  visa_valid_until?: string | null;
  // kiểm soát
  name_confirmed_by_human: boolean;
  single_token_name_ok?: boolean;
}

export interface ExtractedIdentity {
  source: DeclarationSource;
  confidence: DeclarationConfidence;
  identity: DeclarationIdentity;
  review_hints: string[];
  /** data:image/png;base64,… — chỉ đi qua IPC, không bao giờ ghi đĩa (§12.4). */
  crop_data_url?: string | null;
}

export interface StayInfo {
  stay_id: string;
  room_no: string | null;
  check_in: string;
  expected_out: string;
  /** Tên khách đã có trong CapyInn — dùng để xếp hạng gợi ý ghép (§7). */
  guest_name?: string | null;
}

export interface DeclarationRow {
  link_id: string;
  identity_id: string;
  full_name: string;
  dob: string;
  gender: string;
  nationality_iso3: string;
  doc_type_code: string | null;
  doc_type_name: string | null;
  doc_no: string | null;
  phone: string | null;
  residence_status: string | null;
  address_detail: string | null;
  passport_no: string | null;
  passport_expiry: string | null;
  visa_valid_until: string | null;
  room_no: string | null;
  check_in_date: string;
  expected_check_out: string;
  stay_reason: string;
  stay_reason_note: string | null;
  name_confirmed_by_human: boolean;
  single_token_name_ok: boolean;
}

export interface DeclarationFinding {
  code: string;
  severity: DeclarationSeverity;
  link_id: string;
  field?: string | null;
  message: string;
}

export interface DeclarationExportResult {
  batch_id: string;
  file_path: string;
  row_count: number;
  kind: DeclarationBatchKind;
}

export interface DeclarationBatch {
  id: string;
  kind: DeclarationBatchKind;
  file_path: string;
  row_count: number;
  status: DeclarationBatchStatus;
  verified_count: number | null;
  verified_at: string | null;
  created_at: string;
}

/** Khớp `VoidBookingPreview` (`src-tauri/src/models.rs`) — không thêm bớt field. */
export interface VoidBookingPreview {
  booking_id: string;
  guest_name: string;
  room_id: string;
  previous_status: string;
  /** Tiền rời khỏi báo cáo: tiền phòng đã ghi nhận + folio + phí huỷ. */
  revenue_impact: MoneyVnd;
  revenue_date: string;
  /** Tiền cọc. KHÔNG nằm trong revenue_impact — cọc là khoản thanh toán,
   *  chưa bao giờ là doanh thu, và `transactions` chỉ ghi thêm nên xoá lượt
   *  không gỡ nó đi. Hiển thị thành dòng riêng, chữ khác. */
  deposit_amount: MoneyVnd;
  nights_recognized: number;
  nights_total: number;
  is_audited: boolean;
  /** True = xoá lượt này sẽ KHÔNG đổi trạng thái phòng (chỉ tính cho
   *  previous_status "checked_out", luôn false ở nơi khác). KHÔNG suy ra có
   *  khách khác đang ở — true cả khi phòng đã Trống. */
  room_status_unchanged: boolean;
  is_group_booking: boolean;
}
