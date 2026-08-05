import { create } from "zustand";

import { invokeCommand, invokeWriteCommand } from "@/lib/invokeCommand";
import { isActionExpired, MESSAGE_WINDOW } from "@/types/assistant";
import type {
  AssistantConversationSummary,
  AssistantMessage,
  AssistantSettings,
  AssistantTurnResponse,
  ChatMessage,
  ProposedAction,
  ScreenContext,
  StoredMessage,
} from "@/types/assistant";

type AssistantState = {
  open: boolean;
  messages: AssistantMessage[];
  history: ChatMessage[];
  pendingAction: ProposedAction | null;
  busy: boolean;
  error: string | null;
  settings: AssistantSettings | null;

  /// Khoá phiên phía frontend, **LUÔN** có giá trị. Mint lần đầu lúc tạo store,
  /// mint lại mỗi khi bấm *hội thoại mới* hoặc mở một hội thoại từ lịch sử.
  ///
  /// Không dùng `conversationId` (id database) làm khoá so sánh: khi ghi DB
  /// hỏng nó là `null` ở cả hai vế, và `null === null` sẽ **khớp** — đúng cái
  /// phải chặn. Khoá phiên cũng độc lập với việc ghi DB thành hay bại.
  conversationKey: string;
  conversationId: string | null;
  /// Phiên mà thẻ đang treo được dựng ra. `null` ⟺ không có thẻ nào.
  pendingActionKey: string | null;
  conversations: AssistantConversationSummary[];
  /// Dòng nhắc khi mở lại một hội thoại cũ. `null` là không nhắc gì.
  historyNotice: string | null;

  togglePanel: () => void;
  refreshSettings: () => Promise<void>;
  send: (message: string, screenContext: ScreenContext) => Promise<void>;
  approve: () => Promise<void>;
  dismissAction: () => void;
  startNewChat: () => void;
  resetForLogout: () => void;
  openConversation: (conversationId: string) => Promise<void>;
  loadConversations: () => Promise<void>;
  deleteConversation: (conversationId: string) => Promise<void>;
  deleteAllConversations: () => Promise<void>;
};

function nextId(): string {
  return typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `${Date.now()}-${Math.random().toString(16).slice(2)}`;
}

function readErrorMessage(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return "Trợ lý gặp lỗi không xác định.";
}

/// Một hàng trong sổ thành một dòng trên panel.
///
/// `action` hiện như lời trợ lý: hàng ấy là **CHỮ** tóm tắt thẻ, không phải dữ
/// liệu dựng lại được (`commands/assistant.rs::summarize_action`). Nó cố ý
/// không mang `payload` hay `built_at_ms`, nên mở lại sổ cũ không bao giờ dựng
/// lại được một thẻ bấm duyệt được — và đó là lý do nó chỉ là chữ.
function toPanelMessage(message: StoredMessage): AssistantMessage {
  if (message.kind === "user") return { id: message.id, kind: "user", text: message.text };
  if (message.kind === "error") return { id: message.id, kind: "error", text: message.text };
  return { id: message.id, kind: "assistant", text: message.text };
}

/// Dựng lại `history` gửi cho nhà cung cấp từ các hàng vừa tải.
///
/// Chỉ `user` và `assistant`. `error` và `action` không phải lượt hội thoại hợp
/// lệ với nhà cung cấp: `error` là câu báo lỗi viết cho lễ tân đọc, `action` là
/// bản tóm tắt thẻ. Nhồi chúng vào là dạy nhà cung cấp một cuộc trò chuyện chưa
/// từng xảy ra.
function rebuildHistory(stored: StoredMessage[]): ChatMessage[] {
  return stored
    .filter((message) => message.kind === "user" || message.kind === "assistant")
    .map((message) => ({ role: message.kind, content: message.text }));
}

/// Hai dòng nhắc, **cả hai đều có điều kiện**. Vô điều kiện thì chúng nằm cả
/// trên một hội thoại hai tin nhắn chưa mất gì — thành nhiễu, và nhiễu thường
/// trực thì người ta thôi đọc.
function buildHistoryNotice(stored: StoredMessage[]): string | null {
  const notices: string[] = [];
  // Hàng `action` là tín hiệu **duy nhất đo được** từ dữ liệu đã lưu: vòng gọi
  // tool không được ghi lại nên không có cách nào biết hội thoại cũ từng tra
  // cứu gì.
  if (stored.some((message) => message.kind === "action")) {
    notices.push("Đây là bản ghi. Hỏi tiếp thì trợ lý không nhớ thẻ đã đề xuất trước đó.");
  }
  if (stored.length >= MESSAGE_WINDOW) {
    notices.push(`Chỉ ${MESSAGE_WINDOW} tin gần nhất được dùng làm ngữ cảnh.`);
  }
  return notices.length > 0 ? notices.join(" ") : null;
}

/// Câu từ chối dùng chung cho mọi đường mint khoá phiên bị chặn vì đang bận.
///
/// Nói cả hai thứ có thể đang bay — lượt trả lời của trợ lý và lệnh nhận phòng
/// — vì `busy` là MỘT cờ dùng chung cho cả hai, và người đọc câu này cần biết
/// mình đang chờ cái gì. Được vẽ ở **cả hai** bề mặt: viên `role="alert"` của
/// panel (`AssistantPanel.tsx`) và dòng lỗi của Cài đặt → Trợ lý quầy.
export const BUSY_REFUSAL =
  "Trợ lý đang bận: còn một lượt trả lời hoặc một lệnh nhận phòng chưa xong. Xong rồi hãy thử lại.";

/// Trạng thái của một phiên chat trống. Dùng chung cho *hội thoại mới*, cho
/// đường xoá, và cho đăng xuất — ba chỗ phải dọn **đúng cùng một tập**, không
/// thì chỗ nào quên một field là chỗ đó rò sang người kế tiếp.
///
/// KHÔNG có `busy` ở đây, và đó là chủ ý — xem bất biến sở hữu `busy` ở
/// `startNewChat` bên dưới. Hai trong ba đường mint (`startNewChat`,
/// `openConversation`) chỉ chạy được khi `busy === false`, nên một dòng
/// `busy: false` ở đây là dòng không kịch bản nào làm sai được; đường thứ ba
/// (`resetForLogout`) đã đặt tay và **có** test canh. Thêm vào đây là thêm một
/// dòng không ai canh nổi.
function emptySession() {
  return {
    // Mint khoá mới: mọi thẻ đang treo lập tức mất quyền duyệt qua lớp 4.
    conversationKey: nextId(),
    conversationId: null,
    messages: [],
    history: [],
    pendingAction: null,
    pendingActionKey: null,
    historyNotice: null,
    error: null,
  };
}

export const useAssistantStore = create<AssistantState>((set, get) => ({
  open: false,
  messages: [],
  history: [],
  pendingAction: null,
  busy: false,
  error: null,
  settings: null,
  conversationKey: nextId(),
  conversationId: null,
  pendingActionKey: null,
  conversations: [],
  historyNotice: null,

  togglePanel: () => set((state) => ({ open: !state.open })),

  refreshSettings: async () => {
    try {
      const settings = await invokeCommand<AssistantSettings>("get_assistant_settings");
      set({ settings });
    } catch {
      set({ settings: null });
    }
  },

  send: async (message, screenContext) => {
    const trimmed = message.trim();
    if (!trimmed || get().busy) return;

    // Phiên mà lượt này thuộc về, chốt TRƯỚC khi bắn lệnh đi.
    const turnKey = get().conversationKey;

    set((state) => ({
      busy: true,
      error: null,
      messages: [...state.messages, { id: nextId(), kind: "user", text: trimmed }],
    }));

    try {
      const response = await invokeCommand<AssistantTurnResponse>("assistant_turn", {
        request: {
          message: trimmed,
          screen_context: screenContext,
          history: get().history,
          conversation_id: get().conversationId,
        },
      });

      // Đổi hội thoại giữa lúc câu trả lời đang bay về. `response.history` là
      // transcript của hội thoại CŨ — tên khách và CCCD của người không liên
      // quan — nên đổ nó vào phiên mới là rò dữ liệu khách, và luật lọc
      // `user_id` phía Rust không nhìn thấy đường này.
      //
      // `return` TRẦN, không `set({ busy: false })` — xem bất biến sở hữu
      // `busy` ở `startNewChat`. Lượt này thuộc về phiên đã chết; `busy` mà nó
      // thấy bây giờ là `busy` CỦA PHIÊN KHÁC, và dọn hộ là xoá mất trạng thái
      // "đang gửi lệnh nhận phòng" của người đang đứng ở quầy.
      if (get().conversationKey !== turnKey) {
        return;
      }

      set((state) => ({
        busy: false,
        history: response.history,
        pendingAction: response.proposed_action,
        // Gắn thẻ vào đúng phiên nó được dựng ra, ngay tại thời điểm gán.
        pendingActionKey: response.proposed_action ? turnKey : null,
        // `null` từ backend nghĩa là **không tạo được** hội thoại, không phải
        // "chưa có". Ghi đè bằng `null` khi đang có id là bảo lượt sau mở hội
        // thoại mới, và cuộc trò chuyện đang dở bị chẻ làm hai bản ghi rời.
        conversationId: response.conversation_id ?? state.conversationId,
        messages: response.reply
          ? [...state.messages, { id: nextId(), kind: "assistant", text: response.reply }]
          : state.messages,
      }));
    } catch (error) {
      const text = readErrorMessage(error);
      // `return` trần, cùng lý do như nhánh thành công ngay trên.
      if (get().conversationKey !== turnKey) {
        return;
      }
      set((state) => ({
        busy: false,
        error: text,
        messages: [...state.messages, { id: nextId(), kind: "error", text }],
      }));
    }
  },

  approve: async () => {
    const { pendingAction, pendingActionKey, conversationKey, busy } = get();
    if (!pendingAction || busy) return;

    // Lớp 4 — lớp duy nhất nằm trên đường tiền.
    //
    // Lớp 3 (panel không vẽ thẻ ở sai hội thoại) canh việc VẼ; lớp này canh
    // việc GHI. `approve()` là chỗ duy nhất đường trợ lý chạm `check_in` (chỗ
    // thứ hai là `useHotelStore.ts:157`, màn hình nhận phòng tay, cố ý nằm
    // ngoài rào). Không có câu này thì một thẻ dựng ở hội thoại A vẫn duyệt
    // được trong lúc đang mở hội thoại B — lệnh nhận phòng thật, tiền thật,
    // gắn nhầm chỗ.
    //
    // So bằng `conversationKey` chứ KHÔNG bằng `conversationId`: id database là
    // `null` khi ghi hỏng, và `null === null` sẽ khớp.
    //
    // Kiểm TRƯỚC hạn 5 phút: thẻ của hội thoại khác thì hết hạn hay chưa cũng
    // không đổi được câu trả lời, mà báo "hết hạn" cho nó là mời lễ tân bấm
    // *tính lại* trên một thẻ đáng lẽ không được nhìn thấy.
    if (pendingActionKey !== conversationKey) {
      set({ pendingAction: null, pendingActionKey: null });
      return;
    }

    if (isActionExpired(pendingAction, Date.now())) {
      // Giữ nguyên thẻ: giá trên thẻ được tính cho một mốc thời gian cụ thể; nếu
      // duyệt thẻ đã hết hạn, số tiền thu có thể lệch với giá hiện hành (ví dụ
      // đổi ngày, đổi cuối tuần). Không tự duyệt thẳng — bắt trợ lý tính lại.
      set({ error: "Thẻ xác nhận đã hết hạn, vui lòng yêu cầu trợ lý tính lại." });
      return;
    }

    // Phiên mà lượt duyệt này thuộc về, chốt TRƯỚC khi bắn lệnh ghi đi — cùng
    // một chốt `send()` đang có. Đây là câu đọc đồng bộ, KHÔNG phải `await`,
    // nên nó không mở khe nào giữa lớp 4 ở trên và `invokeWriteCommand` dưới.
    const approveKey = get().conversationKey;

    set({ busy: true, error: null });
    try {
      await invokeWriteCommand("check_in", { req: pendingAction.payload });
      // Đổi hội thoại giữa lúc `check_in` đang bay về. Lệnh đã đi rồi và vẫn
      // phải chạy — huỷ một lượt nhận phòng đã bắn là chuyện khác — nhưng KẾT
      // QUẢ thì không được đổ vào phiên đang mở: lễ tân mở chat mới để phục vụ
      // khách kế tiếp mà thấy "Đã nhận phòng xong." ngay trên màn hình của
      // khách đó là đúng kiểu nhầm lẫn trên đường tiền mà lớp 4 sinh ra để chặn.
      //
      // `return` trần: xem bất biến sở hữu `busy` ở `startNewChat`. Đây là
      // nhánh ĐẮT NHẤT trong bốn nhánh — dọn hộ `busy` ở đây là bật lại nút
      // *Đồng ý* trên một thẻ vẫn đang nằm trên màn hình, và cú bấm thứ hai
      // bắn `check_in` LẦN NỮA với một `idempotencyKey` mới toanh
      // (`createIdempotencyKey` sinh UUID mới mỗi lượt gọi) nên backend không
      // dedupe được: nhận phòng thật, hai lần.
      if (get().conversationKey !== approveKey) {
        return;
      }
      set((state) => ({
        busy: false,
        pendingAction: null,
        pendingActionKey: null,
        messages: [
          ...state.messages,
          { id: nextId(), kind: "assistant", text: "Đã nhận phòng xong." },
        ],
      }));
    } catch (error) {
      // Cùng lý do như đường thành công, nặng hơn một bậc: chuỗi lỗi của PMS
      // ("Phòng đã có khách") hiện trong một hội thoại không liên quan, còn cái
      // thẻ để sửa thì đã mất theo phiên cũ nên chẳng ai làm gì được với nó.
      // `return` trần, cùng lý do như nhánh thành công ngay trên.
      if (get().conversationKey !== approveKey) {
        return;
      }
      // Giữ nguyên thẻ: người dùng còn sửa hoặc mở form làm tay được.
      set({ busy: false, error: readErrorMessage(error) });
    }
  },

  dismissAction: () => set({ pendingAction: null, pendingActionKey: null }),

  // ── BẤT BIẾN SỞ HỮU `busy` ──────────────────────────────────────────────
  //
  // **Ai mint khoá phiên mới thì người đó sở hữu `busy` của phiên mới; một lượt
  // cũ bay về muộn KHÔNG được đụng vào.**
  //
  // Hai vế, và phải có cả hai:
  //
  // 1. Bốn nhánh "khoá phiên đã đổi" (hai trong `send()`, hai trong `approve()`)
  //    `return` TRẦN. Trước đây cả bốn `set({ busy: false })` vô điều kiện, và
  //    đó là con đường duyệt `check_in` HAI LẦN, đo được đầu-cuối:
  //      A hỏi → nhà cung cấp treo (busy=true) → A đăng xuất (resetForLogout đặt
  //      busy=false, mint khoá) → B đăng nhập, hỏi, có thẻ → B bấm *Đồng ý*
  //      (check_in bay, busy=true, nút xám) → lượt CŨ của A về → nhánh stale
  //      set busy=false → nút *Đồng ý* sáng lại TRÊN CÁI THẺ VẪN CÒN ĐÓ → B bấm
  //      lần hai → `check_in` bắn lần thứ hai, cùng payload, `idempotencyKey`
  //      MỚI (`createIdempotencyKey` sinh UUID mỗi lượt gọi) nên backend không
  //      dedupe. Nhận phòng thật, hai lần, tiền thật.
  //
  // 2. Ba đường mint còn lại **từ chối khi đang bận** (`startNewChat` ở đây,
  //    `openConversation`, và hai đường xoá đi qua `startNewChat`). Đây là hàng
  //    rào ở TẦNG STORE: `disabled={busy}` trên nút chỉ là **lấy mẫu lúc
  //    render** — cú bấm đã đi rồi thì `busy` bật lên sau đó không thu lại
  //    được. Đo được ở cửa xoá sạch của Cài đặt (`AssistantSection.tsx`): admin
  //    bấm *Xoá vĩnh viễn* lúc rảnh → lễ tân bấm *Đồng ý* trên thẻ ở panel bên
  //    trái → `check_in` bay đi → lệnh xoá về → `startNewChat()` mint khoá mới
  //    giữa lúc `check_in` đang bay → lớp 4 vứt kết quả, `pendingAction` mất,
  //    màn hình KHÔNG NÓI GÌ, mà phòng thì đã nhận thật.
  //
  // `resetForLogout` là **ngoại lệ duy nhất**: không thể từ chối một cú đăng
  // xuất (giao ca là thao tác bình thường, nút không có `disabled` nào ở
  // `MainShell.tsx:264`/`:274`). Nó mint được cả khi đang bận, và vì thế nó
  // phải TỰ đặt `busy: false` — nó là chủ phiên mới.
  //
  // Từ chối chứ không im lặng: `error` đã được vẽ ở **cả hai** bề mặt (viên
  // `role="alert"` của panel và dòng lỗi của Cài đặt), nên câu từ chối tới được
  // mắt người dùng thay vì biến thành một cú bấm không có hồi đáp.
  startNewChat: () => {
    if (get().busy) {
      set({ error: BUSY_REFUSAL });
      return;
    }
    set(emptySession());
  },

  /// Đổi người dùng. Gọi từ `useAuthStore.logout()`, không phải từ panel.
  ///
  /// Store zustand là singleton của module và trong `src/` không có chỗ nào
  /// `location.reload`, nên mọi thứ ở đây sống nguyên vẹn qua màn hình PIN nếu
  /// không dọn tay: lễ tân B đăng nhập là đọc được hội thoại của lễ tân A, và —
  /// nặng hơn — bấm duyệt được luôn thẻ nhận phòng còn treo của A, vì
  /// `pendingActionKey === conversationKey` vẫn khớp. `emptySession()` mint
  /// khoá phiên mới, và chính cú mint đó làm lớp 4 vứt thẻ của người trước.
  ///
  /// Dọn thêm hai thứ ngoài `emptySession()`:
  /// - `conversations`: danh sách lịch sử của người vừa ra, tiêu đề mang tên
  ///   khách. Nó không nằm trong `emptySession()` vì *hội thoại mới* cố ý giữ
  ///   lại danh sách — cùng một người, cùng quyền đọc.
  /// - `open`: panel không được mở sẵn trên tay người kế tiếp.
  /// - `busy`: cờ "đang chờ trả lời". Đây là đường mint **duy nhất** chạy được
  ///   khi đang bận, nên nó là đường duy nhất phải tự đặt lại cờ — bất biến sở
  ///   hữu `busy` ở `startNewChat` nói đủ vì sao. Hai lý do cộng lại:
  ///   - Lệnh không bao giờ bay về (mạng treo, nhà cung cấp AI câm) thì không
  ///     nhánh nào trong `send()` chạy, cờ kẹt `true` sang phiên của người kế
  ///     tiếp, và `send()` tự chặn ở câu đầu tiên: khung soạn của họ chết tới
  ///     khi khởi động lại app. Đăng xuất là đúng lúc dọn cứng, không phụ thuộc
  ///     vào một promise đang bay.
  ///   - Và kể cả khi lệnh CÓ bay về, bốn nhánh stale nay `return` trần —
  ///     chúng không còn dọn hộ cờ của phiên mới nữa, nên phiên mới phải bắt
  ///     đầu từ một cờ sạch do chính nó đặt.
  resetForLogout: () =>
    set({ ...emptySession(), conversations: [], open: false, busy: false }),

  openConversation: async (conversationId) => {
    // Cửa vào — xem bất biến sở hữu `busy` ở `startNewChat`.
    if (get().busy) {
      set({ error: BUSY_REFUSAL });
      return;
    }

    let stored: StoredMessage[];
    try {
      stored = await invokeCommand<StoredMessage[]>("get_assistant_conversation", {
        conversationId,
      });
    } catch (error) {
      // Không đổi phiên khi chưa đọc được gì: nửa chuyển là trạng thái tệ nhất
      // — panel nói đang ở hội thoại kia mà `history` vẫn của hội thoại này.
      set({ error: readErrorMessage(error) });
      return;
    }

    // Kiểm LẠI sau `await`, và đây mới là câu chặn thật.
    //
    // Cửa vào ở trên cũng chỉ là **lấy mẫu**, y như `disabled={busy}` trên nút:
    // giữa lúc đọc sổ, thẻ nhận phòng của phiên hiện tại vẫn đang được VẼ (lớp
    // 3 so `pendingActionKey === conversationKey`, mà khoá chưa đổi) và nút
    // *Đồng ý* vẫn sáng (`busy` vẫn false). Lễ tân bấm → `check_in` bay →
    // `busy=true` → sổ đọc xong về tới đây → mint khoá giữa lúc lệnh ghi đang
    // bay. Đúng con đường của cửa xoá sạch, chỉ khác cửa.
    if (get().busy) {
      set({ error: BUSY_REFUSAL });
      return;
    }

    set({
      ...emptySession(),
      conversationId,
      messages: stored.map(toPanelMessage),
      // `history` là bản ghi gửi cho nhà cung cấp AI. Không dựng lại từ đầu thì
      // lượt sau gửi kèm transcript của khách trước — tên và CCCD của người
      // không liên quan. Luật lọc `user_id` không nhìn thấy đường này.
      history: rebuildHistory(stored),
      historyNotice: buildHistoryNotice(stored),
    });
  },

  loadConversations: async () => {
    try {
      // Không tham số: danh tính lấy từ phiên phía Rust, frontend không khai
      // mình là ai. Trần 50 hội thoại chốt cứng ở tầng query, không phân trang.
      const conversations = await invokeCommand<AssistantConversationSummary[]>(
        "list_assistant_conversations",
      );
      // Dọn viên lỗi cũ, không chỉ nạp danh sách. Không có `error: null` thì
      // đường đo được là: tải hỏng → viên đỏ "Không đọc được sổ hội thoại" →
      // bấm thử lại → danh sách hiện ra ĐẦY ĐỦ, mà viên đỏ cũ vẫn nằm nguyên
      // trên đầu chính cái danh sách nó vừa tố là đọc không được. Không phải
      // "viên đỏ nằm lì vài phút" mà là mâu thuẫn trực tiếp với thứ ngay bên
      // dưới nó.
      //
      // Đây cũng là chỗ dọn cho hai đường xoá: cả hai kết thúc bằng
      // `loadConversations()`, kể cả ca xoá một hội thoại KHÁC hội thoại đang
      // mở — ca duy nhất không đi qua `startNewChat()` (và `error: null` của
      // `emptySession()`).
      set({ conversations, error: null });
    } catch (error) {
      set({ error: readErrorMessage(error) });
    }
  },

  deleteConversation: async (conversationId) => {
    // Cửa vào — xem bất biến sở hữu `busy` ở `startNewChat`. Từ chối cả LỆNH
    // XOÁ chứ không chỉ cú mint: xoá được nhưng không dọn nổi phiên thì panel
    // ngồi trên một hội thoại đã biến mất khỏi đĩa, và lượt sau còn gửi kèm
    // `conversation_id` của một hàng không còn tồn tại.
    if (get().busy) {
      set({ error: BUSY_REFUSAL });
      return;
    }

    try {
      await invokeWriteCommand("delete_assistant_conversation", { conversationId });
    } catch (error) {
      // Lệnh này là **admin only**; lễ tân gọi nhận `AUTH_FORBIDDEN`. Giấu nút
      // xoá là chuyện của panel, không phải hàng rào — nên bị từ chối thì store
      // tuyệt đối không dọn gì. Dọn trước rồi mới biết bị từ chối là thổi bay
      // hội thoại đang dở trên màn hình trong khi backend chẳng xoá dòng nào.
      set({ error: readErrorMessage(error) });
      return;
    }

    // `busy` có thể đã bật lên TRONG lúc lệnh xoá bay — cửa vào ở trên chỉ lấy
    // mẫu. Không tự dọn phiên ở đây mà đi qua `startNewChat()`, vì hàng rào nằm
    // trong đó: một chỗ chặn cho cả bốn cửa.
    const keyBeforeReset = get().conversationKey;
    const mustReset = get().conversationId === conversationId;
    if (mustReset) get().startNewChat();
    // Bị từ chối ⟺ khoá phiên KHÔNG đổi. Đọc bằng khoá chứ không bằng `error`:
    // `error` lúc này có thể là viên cũ từ trước, mà viên cũ thì
    // `loadConversations()` được phép dọn (và có test canh đúng việc đó).
    const refused = mustReset && get().conversationKey === keyBeforeReset;

    await get().loadConversations();
    // Đặt LẠI sau `loadConversations()`: nạp được danh sách thì nó dọn `error`,
    // và câu từ chối vừa rồi sẽ chết theo — admin bấm xoá, sổ vẫn còn nguyên
    // trên màn hình, không một chữ giải thích.
    if (refused) set({ error: BUSY_REFUSAL });
  },

  deleteAllConversations: async () => {
    // Cửa vào — xem bất biến sở hữu `busy` ở `startNewChat`, và cùng lý do từ
    // chối cả lệnh xoá như `deleteConversation` ngay trên.
    if (get().busy) {
      set({ error: BUSY_REFUSAL });
      return;
    }

    try {
      await invokeWriteCommand("delete_all_assistant_conversations");
    } catch (error) {
      set({ error: readErrorMessage(error) });
      return;
    }

    const keyBeforeReset = get().conversationKey;
    get().startNewChat();
    const refused = get().conversationKey === keyBeforeReset;

    await get().loadConversations();
    if (refused) set({ error: BUSY_REFUSAL });
  },
}));
