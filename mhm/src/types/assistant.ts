export type ScreenContext = {
  route: string;
  selectedRoomId?: string;
  selectedRoomNumber?: string;
  selectedBookingId?: string;
  dateInView?: string;
};

export type CheckInGuestPayload = {
  full_name: string;
  doc_number?: string;
  phone?: string | null;
};

export type CheckInPayload = {
  room_id: string;
  guests: CheckInGuestPayload[];
  nights: number;
  source?: string | null;
  notes?: string | null;
  paid_amount?: number | null;
  pricing_type?: string | null;
};

export type ProposedAction = {
  kind: "check_in";
  payload: CheckInPayload;
  display: Record<string, string>;
  preview: Record<string, unknown>;
  warnings: string[];
  built_at_ms: number;
};

export type ChatMessage = {
  role: string;
  content?: string | null;
  tool_calls?: unknown[] | null;
  tool_call_id?: string | null;
};

export type AssistantTurnResponse = {
  reply: string | null;
  proposed_action: ProposedAction | null;
  history: ChatMessage[];
  /// Id để dùng cho lượt sau. `null` nghĩa là **không tạo được** hội thoại,
  /// không phải "chưa có" — nên nhận `null` mà đang có id thì giữ id cũ, đừng
  /// ghi đè. Xem `AssistantTurnResponse` phía Rust.
  conversation_id: string | null;
};

/// Một dòng trong danh sách lịch sử. Cùng hình dạng với
/// `queries::assistant::conversation_queries::ConversationSummary`.
export type AssistantConversationSummary = {
  id: string;
  user_id: string;
  user_name: string;
  title: string;
  updated_at: string;
};

/// Một hàng đã ghi trong sổ hội thoại.
///
/// `kind` để nguyên `string` chứ không thu về union: nó là dữ liệu đã nằm trên
/// đĩa, và một `kind` lạ (bản cũ, bản sau) phải đi lọt qua đường đọc chứ không
/// được làm hỏng cả hội thoại. Chỗ nào cần phân nhánh thì tự so chuỗi.
export type StoredMessage = {
  id: string;
  kind: string;
  text: string;
  created_at: string;
};

/// Trần tải một hội thoại, chốt cứng ở `conversation_queries::MESSAGE_WINDOW`
/// và **không có phân trang**. Chạm trần nghĩa là phần cũ hơn không được gửi
/// cho nhà cung cấp — đó là thứ phải nói ra chứ không để lễ tân tự đoán.
export const MESSAGE_WINDOW = 100;

export type AssistantGateMissing = "api_key" | "cloud_data_opt_in" | "model" | "base_url";

export type AssistantSettings = {
  config: {
    preset: "deep_seek" | "open_router" | "custom";
    base_url: string;
    model: string;
  };
  has_api_key: boolean;
  cloud_data_opt_in: boolean;
  gate: { ready: boolean; missing: AssistantGateMissing[] };
};

export type AssistantMessage =
  | { id: string; kind: "user"; text: string }
  | { id: string; kind: "assistant"; text: string }
  | { id: string; kind: "error"; text: string };

export const CARD_TTL_MS = 5 * 60 * 1000;

export function isActionExpired(action: ProposedAction, nowMs: number): boolean {
  return nowMs - action.built_at_ms > CARD_TTL_MS;
}

/// Chuỗi phải gõ đúng thì nút xoá sạch mới bật.
///
/// Ở đây chứ không ở trong một component nào, vì spec dòng 359 đòi nút *Xoá tất
/// cả* ở **HAI** chỗ — cuối danh sách lịch sử và Cài đặt → Trợ lý quầy. Hai bản
/// chép tay là hai chỗ trôi độc lập, và cái trôi được thì đúng là hàng rào.
export const DELETE_ALL_PHRASE = "XOÁ HẾT";

/// So bằng `===` trên chuỗi **thô**: không `trim()`, không `toUpperCase()`,
/// không bỏ dấu.
///
/// Hàng rào này tồn tại để gây khó — lệnh xoá sạch xoá bản duy nhất, không hoàn
/// tác, và chủ nhà đã chọn hệ thống không tự xoá nên đây là lối ra duy nhất của
/// dữ liệu khách. Mỗi cách nới lỏng biến nó thành một ô nhập trang trí: chuẩn
/// hoá bỏ dấu cho "xoa het" đi lọt, mà "xoa het" là thứ gõ nhầm ra được; còn
/// `trim()` cho một chuỗi dán từ chỗ khác đi lọt, mà dán chuỗi không phải hành
/// vi của người vừa đọc xong câu cảnh báo.
///
/// Là một hàm dùng chung chứ không phải hai câu so sánh chép tay: nới lỏng ở
/// đây làm đỏ test của **cả hai** cửa cùng lúc.
export function isDeleteAllPhrase(typed: string): boolean {
  return typed === DELETE_ALL_PHRASE;
}
