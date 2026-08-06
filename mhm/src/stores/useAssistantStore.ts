import { create } from "zustand";

import { invokeCommand, invokeWriteCommand } from "@/lib/invokeCommand";
import { actionKindCopy, isActionExpired, MESSAGE_WINDOW } from "@/types/assistant";
import type {
  AssistantConversationSummary,
  AssistantMessage,
  AssistantSettings,
  AssistantTurnResponse,
  ChatMessage,
  ProposedAction,
  ProposedActionKind,
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
  /// Lượt chat **gần nhất** không vào được sổ. Đọc thẳng từ `turn_saved` của
  /// backend, không suy ra từ bất cứ thứ gì khác — xem `AssistantTurnResponse`.
  ///
  /// Là một `boolean` của LƯỢT GẦN NHẤT, cố ý không phải một danh sách và cũng
  /// không phải một bộ đếm: spec dòng 446-447 đòi "một lần cho mỗi lượt chat
  /// hỏng, **không cộng dồn**", và cách rẻ nhất để bảo đảm điều đó là làm cho
  /// việc cộng dồn **không diễn đạt được**. Mỗi lượt ghi đè giá trị này, nên
  /// lượt sau lưu được là dòng thông báo tự biến mất; không có đường nào để hai
  /// lượt hỏng thành hai dòng.
  ///
  /// Khác `historyNotice` (dòng nhắc 100 tin / bản-ghi-cũ) cả về nguồn lẫn về
  /// vòng đời: `historyNotice` đọc từ dữ liệu đã tải khi MỞ một hội thoại, còn
  /// cờ này đọc từ kết quả GỬI một lượt. Spec đòi riêng từng dòng.
  saveFailed: boolean;

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

/// Mã lỗi backend trả khi `conversation_id` gửi kèm lượt chat không mở được.
///
/// Viết thẳng chuỗi thay vì tra `APP_ERROR_CODES`: bảng mã là
/// `Record<string, string>` dựng lúc chạy từ `shared/error-codes.json`, nên mã
/// bị đổi tên thì `APP_ERROR_CODES.AUTH_FORBIDDEN` thành `undefined`, và câu so
/// sánh dưới đây sẽ khớp **mọi** lỗi không mang `code` — nhận nhầm im lặng còn
/// tệ hơn bỏ sót im lặng.
const CONVERSATION_FORBIDDEN_CODE = "AUTH_FORBIDDEN";

/// Lượt chat vừa hỏng vì chính cái `conversation_id` frontend gửi lên.
///
/// Đường Rust, đọc chứ không đoán: `commands/assistant.rs::open_turn_record` →
/// `services/assistant/conversation_service.rs::assert_can_read` → chủ hội
/// thoại là `None` (hàng đã bị xoá) hoặc là người khác → `forbidden()` =
/// `CommandError::user(AUTH_FORBIDDEN, "Không mở được hội thoại này.")` → `kind
/// == User` nên nổ ra ngoài bằng `Err`, hỏng cả lượt. Đây là chỗ **duy nhất**
/// `assistant_turn` sinh ra mã này; hai ca xác thực còn lại mang mã khác
/// (`AUTH_NOT_AUTHENTICATED`).
///
/// Cả hai nguyên nhân đều dẫn tới cùng một kết luận ở frontend: id đang giữ
/// KHÔNG dùng được nữa, và giữ nó lại là bảo đảm mọi lượt sau hỏng y hệt.
function isConversationForbidden(error: unknown): boolean {
  return (
    typeof error === "object" &&
    error !== null &&
    (error as { code?: unknown }).code === CONVERSATION_FORBIDDEN_CODE
  );
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

/// ── ÁNH XẠ `kind` → LỆNH PMS ────────────────────────────────────────────────
///
/// Ba dòng, TƯỜNG MINH, và **không có `default`**. Ba lệnh này ghi ba loại bản
/// ghi khác hẳn nhau vào cùng một cuốn sổ tiền: `check_in` đóng dấu `Local::now()`
/// và bắt đầu tính tiền ngay, `create_reservation` giữ chỗ cho một ngày tương
/// lai, `backfill_stay` ghi lại một kỳ ở đã qua. Đoán nhầm một dòng ở bảng này là
/// đúng con bug đã mở ra cả spec — một cái búa cho mọi cái đinh.
///
/// Là `Record<ProposedActionKind, string>` chứ không object trần: thêm loại thẻ
/// thứ tư mà quên bảng này thì `tsc` đỏ **tại đây**, chứ không đợi tới lúc lễ tân
/// bấm *Đồng ý* và không có gì xảy ra.
const WRITE_COMMAND_BY_KIND: Record<ProposedActionKind, string> = {
  check_in: "check_in",
  reserve: "create_reservation",
  backfill: "backfill_stay",
};

/// Tra qua `Map`, không `WRITE_COMMAND_BY_KIND[kind]` trần.
///
/// `kind` tới từ backend nên nó là một CHUỖI BẤT KỲ, không phải một thành viên
/// union mà `tsc` đã bảo đảm. Object literal trả về đồ của `Object.prototype` cho
/// `"toString"`/`"constructor"` thay vì `undefined`, và câu kiểm "kind lạ" bên
/// dưới sẽ lọt — rồi `invokeWriteCommand` bắn đi một thứ không phải tên lệnh.
/// `Object.hasOwn` không dùng được: `tsconfig.json` khoá `lib: ES2020`.
const WRITE_COMMANDS = new Map<string, string>(Object.entries(WRITE_COMMAND_BY_KIND));

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
    // Dòng "Không lưu được hội thoại này" nói về một lượt của phiên VỪA ĐÓNG.
    // Mang nó sang phiên mới là để nó tố một hội thoại nó không nói về, và ở
    // `openConversation` thì còn tệ hơn: nhắc mất dữ liệu ngay trên một sổ vừa
    // đọc lên nguyên vẹn từ đĩa.
    saveFailed: false,
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
  saveFailed: false,

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
      // Vế thứ nhất của "không cộng dồn": dòng thông báo của lượt TRƯỚC chết
      // ngay lúc lượt mới bắt đầu. Không có dòng này thì một lượt hỏng để lại
      // một dòng nằm lì suốt phần còn lại của phiên, kể cả khi mọi lượt sau đó
      // đều lưu được — và một dòng cảnh báo thường trực thì người ta thôi đọc,
      // đúng lúc nó nói thật thì không ai nhìn.
      saveFailed: false,
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
        // Lượt này XONG rồi thì viên đỏ nói "chưa xong" là nói dối.
        //
        // `send()` đã đặt `error: null` ở đầu, nên dòng này chỉ ăn thua với
        // viên đặt **giữa lúc lệnh đang bay** — và đúng một loại viên rơi vào
        // khe đó: `BUSY_REFUSAL`, từ cú `openConversation()` kiểm lại sau
        // `await` (đường duy nhất từ panel còn tới được câu từ chối; mọi nút
        // khác đều `disabled={busy}`). Đo được trên DOM: màn hình hiện câu trả
        // lời VÀ viên đỏ "Trợ lý đang bận… Xong rồi hãy thử lại." cùng lúc.
        //
        // Cùng lớp lỗi `loadConversations()` đã bịt bằng `error: null`; câu từ
        // chối vì bận mở lại lớp ấy ở đây và ở `approve()`.
        error: null,
        // Vế thứ hai của "không cộng dồn": mỗi lượt GHI ĐÈ, không cộng thêm.
        //
        // Đọc thẳng bit của backend, không suy diễn — `conversation_id` của ca
        // 3b giống hệt của một lượt thành công, nên mọi cách đoán ở đây đều là
        // đoán sai. Đảo dấu đúng một lần, ở đúng chỗ này: `turn_saved` mang
        // chiều "an toàn thì true" để một backend quên đặt sẽ kêu chứ không im.
        saveFailed: !response.turn_saved,
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
        // `saveFailed` cố ý KHÔNG đặt ở đây, và nó đang là `false` do cú dọn ở
        // đầu `send()`. Lượt hỏng thì backend không trả `AssistantTurnResponse`
        // nào cả, nên **không có bit nào để đọc** — dựng một giá trị ở đây là
        // đúng cái "frontend tự đoán" mà trường `turn_saved` sinh ra để cấm. Và
        // lượt này đã có câu lỗi thật (viên đỏ + một bong bóng trong dòng hội
        // thoại); chồng thêm "Không lưu được hội thoại này" là đoán mò về một
        // hàng `kind='error'` mà `close_turn_record` rất có thể đã ghi xong.
        // TỰ LÀNH — đường (b) của I4. Backend vừa từ chối chính cái
        // `conversation_id` này, nên giữ nó lại là bảo đảm **mọi lượt sau hỏng
        // y hệt, mãi mãi**, tới khi lễ tân tự đoán ra là phải bấm *Hội thoại
        // mới*: trợ lý chết cứng ở quầy.
        //
        // Bỏ id đi là lượt kế tiếp mở sổ mới và chạy bình thường. KHÔNG mint
        // khoá phiên ở đây: khoá đổi thì lớp 3 và lớp 4 vứt mất thẻ nhận phòng
        // đang treo, mà thẻ ấy chẳng liên quan gì tới việc ghi sổ chat hỏng.
        //
        // Có điều kiện chứ không dọn mọi lỗi: mạng chập hay nhà cung cấp AI câm
        // thì hội thoại vẫn còn nguyên trên đĩa, và hỏi lại phải là hỏi tiếp
        // đúng sổ đó chứ không phải chẻ ra một sổ mới mỗi lần lỗi.
        conversationId: isConversationForbidden(error) ? null : state.conversationId,
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

    // Lệnh PMS của đúng loại thẻ này — xem `WRITE_COMMAND_BY_KIND` ở đầu file.
    //
    // `kind` LẠ THÌ TỪ CHỐI, không đoán. Không có nhánh nào rơi về `check_in`:
    // một thẻ đặt phòng trước bị bắn qua đường nhận phòng là đóng dấu ngày hôm
    // nay lên một kỳ ở của ngày mai, khoá phòng khỏi mọi phép tra phòng trống, và
    // night audit tính tiền một đêm chưa xảy ra. Đó là bản ghi có thật đã sinh ra
    // spec này, và nó đến từ đúng một chỗ: đoán bừa khi không biết.
    //
    // Ngoài đời `kind` lạ chỉ xảy ra khi frontend và backend lệch hợp đồng, mà
    // hai bên đóng gói chung một app — nên đây là hàng rào cho một trạng thái
    // đáng lẽ không tồn tại. Giữ nguyên thẻ (không vứt): lễ tân còn đọc được nó
    // để làm tay trong PMS, y như nhánh thẻ hết hạn và nhánh PMS trả lỗi.
    const command = WRITE_COMMANDS.get(pendingAction.kind);
    if (command === undefined) {
      set({
        error: `Trợ lý đề xuất một loại thẻ không duyệt được ("${pendingAction.kind}"). Vui lòng làm bằng tay trong PMS.`,
      });
      return;
    }

    // Phiên mà lượt duyệt này thuộc về, chốt TRƯỚC khi bắn lệnh ghi đi — cùng
    // một chốt `send()` đang có. Đây là câu đọc đồng bộ, KHÔNG phải `await`,
    // nên nó không mở khe nào giữa lớp 4 ở trên và `invokeWriteCommand` dưới.
    const approveKey = get().conversationKey;

    set({ busy: true, error: null });
    try {
      await invokeWriteCommand(command, { req: pendingAction.payload });
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
        // Cùng lý do như nhánh thành công của `send()`, và đây là chỗ đọc kỳ
        // quặc nhất trong hai chỗ: viên đỏ "Trợ lý đang bận: … chưa xong."
        // nằm ngay cạnh dòng "Đã nhận phòng xong." vừa in ra. Đo được trên DOM.
        error: null,
        pendingAction: null,
        pendingActionKey: null,
        // Câu báo xong nói ĐÚNG việc vừa làm. Một thẻ đặt phòng trước duyệt xong
        // mà báo "Đã nhận phòng xong." là in lại nguyên văn câu nói dối đã mở ra
        // spec này (mục 1) — chỉ khác là lần này lệnh chạy đúng còn lời kể thì
        // sai, và lễ tân tin lời kể.
        messages: [
          ...state.messages,
          { id: nextId(), kind: "assistant", text: actionKindCopy(pendingAction.kind).done },
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

    // ── I4(a): đừng để lại trạng thái nửa vời ────────────────────────────────
    //
    // Lệnh xoá ĐÃ chạy (đĩa rỗng) nhưng cú dọn phiên bị từ chối, nên
    // `conversationId` còn trỏ vào một hàng không còn tồn tại. Đo được: lượt kế
    // tiếp gửi `assistant_turn { conversation_id: "c1" }` → phía Rust
    // `assert_can_read` thấy chủ hội thoại là `None` → `AUTH_FORBIDDEN`, `kind
    // == User` nên nổ ra ngoài bằng `Err` → cả lượt hỏng → `send()` rơi vào
    // `catch` mà **không** dọn id ⇒ mọi câu hỏi sau hỏng y hệt, mãi mãi.
    //
    // Bỏ id đi mà **KHÔNG mint khoá phiên mới** — đó là toàn bộ điểm của cách
    // này. Khoá không đổi ⇒ lớp 3 vẫn vẽ thẻ, lớp 4 vẫn duyệt được nó, kết quả
    // `check_in` đang bay vẫn tới được màn hình. Không lượt nào hỏng cả, thay vì
    // để lượt sau hỏng rồi mới tự lành.
    if (refused) set({ conversationId: null });

    await get().loadConversations();
    // Đặt LẠI sau `loadConversations()`: nạp được danh sách thì nó dọn `error`,
    // và câu từ chối vừa rồi sẽ chết theo — admin bấm xoá, sổ vẫn còn nguyên
    // trên màn hình, không một chữ giải thích.
    //
    // Nhưng chỉ đặt khi `loadConversations()` KHÔNG để lại lỗi nào. Nạp lại
    // hỏng thì `error` đang mang câu "không đọc được sổ hội thoại", và đè lên nó
    // là lấy mất của admin tin quan trọng hơn: **danh sách đang hiện có thể đã
    // cũ**. Câu từ chối thì còn đo được ở chỗ khác (phiên trên panel vẫn nguyên
    // vẹn); danh sách sai thì không có dấu hiệu nào cả.
    if (refused && get().error === null) set({ error: BUSY_REFUSAL });
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

    // Vế song sinh của `deleteConversation` ngay trên — cùng hai lý do, cùng hai
    // dòng. Ở đây sổ rỗng SẠCH, nên `conversationId` còn sót lại chắc chắn trỏ
    // vào một hàng đã chết.
    if (refused) set({ conversationId: null });

    await get().loadConversations();
    if (refused && get().error === null) set({ error: BUSY_REFUSAL });
  },
}));
