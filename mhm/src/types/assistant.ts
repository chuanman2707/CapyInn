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

/// Payload của thẻ **đặt phòng trước** → lệnh `create_reservation`.
///
/// Hình dạng lấy từ `CreateReservationRequest` (`models.rs`), đúng bộ trường mà
/// `ReservationSheet.tsx` — form làm tay — đang gửi. Frontend KHÔNG đọc trường
/// nào trong đây: nó chuyển thẳng payload sang lệnh, còn thứ lễ tân nhìn thấy là
/// `display`. Kiểu ở đây là để một người đọc code biết cái thẻ mang gì, và để
/// `tsc` bắt được nhánh nào trộn nhầm payload của loại thẻ khác.
export type ReservePayload = {
  room_id: string;
  /// MỘT khách, kiểu chuỗi — khác `check_in` (nhận cả danh sách).
  guest_name: string;
  guest_phone?: string | null;
  guest_doc_number?: string | null;
  /// `YYYY-MM-DD`, phải ở tương lai.
  check_in_date: string;
  /// `YYYY-MM-DD`, phải sau `check_in_date`.
  check_out_date: string;
  /// Dẫn xuất từ hai ngày trên phía Rust, **không** do model đưa.
  nights: number;
  deposit_amount?: number | null;
  source?: string | null;
  notes?: string | null;
  /// LUÔN `null` — quầy không thu phụ thu thêm người. Gửi số khách vào đây là
  /// thẻ báo cao hơn số thực thu. Cùng luật `check_in` và `CheckinSheet.tsx`.
  guests?: number | null;
};

/// Payload của thẻ **ghi bù** → lệnh `backfill_stay`.
///
/// Hình dạng lấy từ `BackfillStayRequest` (`models.rs`), đúng bộ trường mà
/// `BackfillSheet.tsx` đang gửi.
export type BackfillPayload = {
  room_id: string;
  /// Cùng `CreateGuestRequest` phía Rust như `check_in`.
  guests: CheckInGuestPayload[];
  /// `YYYY-MM-DD`, bắt buộc trong quá khứ.
  check_in_date: string;
  /// `null` ⇒ khách còn ở.
  check_out_date?: string | null;
  /// Bắt buộc khi khách còn ở, phải sau hôm nay.
  expected_checkout_date?: string | null;
  /// Số của **preview**, không phải số model đưa — xem spec mục 5.2. Đây là
  /// điểm khác `check_in` (lệnh ấy tự tính tiền), nên nó là chỗ duy nhất trong
  /// ba payload mà một con số tiền đi qua thẻ.
  total_price: number;
  paid_amount: number;
  source?: string | null;
  notes?: string | null;
};

/// Ba loại thẻ, ba đường ghi vào PMS.
///
/// Là union chứ không phải `string`: hai bảng tra theo `kind` (`ACTION_KIND_COPY`
/// dưới đây, và bảng ánh xạ lệnh trong `useAssistantStore.approve()`) đều khai
/// bằng `Record<ProposedActionKind, …>`, nên thêm một loại thứ tư mà quên một
/// bảng thì `tsc` đỏ ngay tại bảng ấy — chứ không đợi tới lúc một cái thẻ không
/// duyệt được ở quầy.
///
/// Cố ý ngược với `StoredMessage.kind` (để nguyên `string`): hàng trong sổ là dữ
/// liệu ĐÃ nằm trên đĩa và một `kind` lạ phải đi lọt qua đường đọc; thẻ thì tới
/// từ lượt chat vừa rồi và nó là đường GHI TIỀN, nên `kind` lạ phải bị chặn.
export type ProposedActionKind = "check_in" | "reserve" | "backfill";

type ProposedActionBase = {
  display: Record<string, string>;
  preview: Record<string, unknown>;
  warnings: string[];
  built_at_ms: number;
};

/// Union phân biệt bằng `kind`: mỗi loại thẻ mang đúng payload của lệnh nó gọi.
/// Trộn payload nhận phòng vào thẻ đặt phòng là lỗi `tsc`, không phải một lệnh
/// PMS hỏng lúc chạy.
export type ProposedAction =
  | (ProposedActionBase & { kind: "check_in"; payload: CheckInPayload })
  | (ProposedActionBase & { kind: "reserve"; payload: ReservePayload })
  | (ProposedActionBase & { kind: "backfill"; payload: BackfillPayload });

/// Chữ hiện cho lễ tân, theo loại thẻ.
export type ActionKindCopy = {
  /// Tiêu đề trên thẻ.
  title: string;
  /// Dòng trạng thái trong lúc lệnh đang bay.
  sending: string;
  /// Câu báo xong, in vào dòng hội thoại sau khi lệnh chạy xong.
  done: string;
  /// Câu gửi cho trợ lý khi bấm *Tính lại* trên một thẻ đã hết hạn.
  rebuildPrompt: string;
};

/// BA BỘ CHỮ KHÁC NHAU **BẰNG CHỮ**, không chỉ bằng màu.
///
/// Lễ tân đọc dòng đầu để biết mình sắp làm gì; màu là tín hiệu phụ và người mù
/// màu không có nó. Cùng lý do cho ba dòng còn lại: một thẻ đặt phòng báo "Đã
/// nhận phòng xong." là **đúng câu nói dối đã mở ra cả spec này** (spec mục 1 —
/// trợ lý trả lời "Đã nhận phòng xong." trong khi vừa ghi một bản ghi sai), và
/// một cú *Tính lại* trên thẻ đặt phòng mà gửi đi "Tính lại thẻ nhận phòng vừa
/// rồi." là tự tay đẩy model về đúng cái búa nó đã đóng nhầm.
///
/// Là `Record<ProposedActionKind, …>` chứ không object trần: thiếu một loại thì
/// `tsc` đỏ tại đây.
const ACTION_KIND_COPY: Record<ProposedActionKind, ActionKindCopy> = {
  check_in: {
    title: "Xác nhận nhận phòng",
    sending: "Đang gửi lệnh nhận phòng…",
    done: "Đã nhận phòng xong.",
    rebuildPrompt: "Tính lại thẻ nhận phòng vừa rồi.",
  },
  reserve: {
    title: "Xác nhận đặt phòng",
    sending: "Đang gửi lệnh đặt phòng…",
    done: "Đã đặt phòng xong.",
    rebuildPrompt: "Tính lại thẻ đặt phòng vừa rồi.",
  },
  backfill: {
    title: "Xác nhận ghi bù",
    sending: "Đang gửi lệnh ghi bù…",
    done: "Đã ghi bù xong.",
    rebuildPrompt: "Tính lại thẻ ghi bù vừa rồi.",
  },
};

/// Bộ chữ cho một thẻ mang `kind` **không thuộc ba loại đã biết**.
///
/// Không gọi tên nó theo loại nào cả. Gán bừa "nhận phòng" cho một cái thẻ mà
/// `approve()` sắp từ chối là dán nhãn sai — đúng lớp lỗi cả spec này sinh ra để
/// sửa — và tệ hơn cả việc nói thẳng là không biết.
const UNKNOWN_KIND_COPY: ActionKindCopy = {
  title: "Thẻ không rõ loại, không duyệt được",
  sending: "Đang gửi lệnh…",
  done: "Đã xong.",
  rebuildPrompt: "Tính lại thẻ vừa rồi.",
};

/// Tra qua `Map` chứ không `ACTION_KIND_COPY[kind]`: bảng trên là object literal
/// nên một `kind` rác trùng tên prototype (`"toString"`, `"constructor"`) sẽ trả
/// về đồ của `Object.prototype` thay vì `undefined`. `Object.hasOwn` không dùng
/// được — `tsconfig.json` khoá `lib: ES2020`.
const ACTION_KIND_COPY_BY_KIND = new Map<string, ActionKindCopy>(Object.entries(ACTION_KIND_COPY));

/// Chữ cho một `kind` **đọc từ backend** — tức một chuỗi bất kỳ, không phải một
/// thành viên union mà `tsc` đã bảo đảm.
///
/// Rơi về `UNKNOWN_KIND_COPY` ở đây **không** phải cái fallback bị cấm trong
/// `approve()`: luật cấm nói về việc đoán một LỆNH để bắn đi, còn đây là chữ trên
/// màn hình cho một cái thẻ mà `approve()` đằng nào cũng từ chối. Không có nó thì
/// một `kind` lạ làm cả panel trắng màn hình (`undefined.title`), và mất trắng
/// panel thì tệ hơn hẳn một dòng tiêu đề nói "không rõ loại".
export function actionKindCopy(kind: string): ActionKindCopy {
  return ACTION_KIND_COPY_BY_KIND.get(kind) ?? UNKNOWN_KIND_COPY;
}

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
  /// Lượt vừa rồi có vào sổ hội thoại **trọn vẹn** hay không.
  ///
  /// **Chỉ backend biết, frontend tuyệt đối không đoán.** Ca 3b (hội thoại đã
  /// có, lệnh chèn message hỏng) trả về **đúng id cũ** — không một trường nào
  /// khác trên hợp đồng này phân biệt được nó với một lượt thành công. Đoán ở
  /// frontend là dựng nguồn sự thật thứ hai cho một câu hỏi chỉ có SQLite trả
  /// lời được. Xem `close_turn_record` (`commands/assistant.rs`).
  turn_saved: boolean;
};

/// Một dòng trong danh sách lịch sử. Cùng hình dạng với
/// `queries::assistant::conversation_queries::ConversationSummary`.
///
/// Cố ý KHÔNG có `user_id`. Bản trước có và **không chỗ nào đọc** — cột hiện tên
/// người tạo dùng `user_name` (`AssistantHistoryList.tsx`) — nhưng nó vẫn được
/// ship xuống webview cho cả 50 dòng mỗi lần mở lịch sử. Đã bỏ ở cả hai đầu
/// cùng lúc; xem doc của `ConversationSummary` phía Rust.
export type AssistantConversationSummary = {
  id: string;
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
