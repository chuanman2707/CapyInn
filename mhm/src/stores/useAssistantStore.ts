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

/// Trạng thái của một phiên chat trống. Dùng chung cho *hội thoại mới*, cho
/// đường xoá, và (Task 10) cho đăng xuất — ba chỗ phải dọn **đúng cùng một
/// tập**, không thì chỗ nào quên một field là chỗ đó rò sang người kế tiếp.
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
      // `user_id` phía Rust không nhìn thấy đường này. Panel khoá nút khi
      // `busy` (spec), nhưng khoá nút là kỷ luật của panel, không phải hàng rào.
      if (get().conversationKey !== turnKey) {
        set({ busy: false });
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
      if (get().conversationKey !== turnKey) {
        set({ busy: false });
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

    set({ busy: true, error: null });
    try {
      await invokeWriteCommand("check_in", { req: pendingAction.payload });
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
      // Giữ nguyên thẻ: người dùng còn sửa hoặc mở form làm tay được.
      set({ busy: false, error: readErrorMessage(error) });
    }
  },

  dismissAction: () => set({ pendingAction: null, pendingActionKey: null }),

  startNewChat: () => set(emptySession()),

  openConversation: async (conversationId) => {
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
      set({ conversations });
    } catch (error) {
      set({ error: readErrorMessage(error) });
    }
  },

  deleteConversation: async (conversationId) => {
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

    if (get().conversationId === conversationId) {
      get().startNewChat();
    }
    await get().loadConversations();
  },

  deleteAllConversations: async () => {
    try {
      await invokeWriteCommand("delete_all_assistant_conversations");
    } catch (error) {
      set({ error: readErrorMessage(error) });
      return;
    }

    get().startNewChat();
    await get().loadConversations();
  },
}));
