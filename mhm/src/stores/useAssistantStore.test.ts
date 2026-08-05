import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeCommand = vi.fn();
const invokeWriteCommand = vi.fn();

vi.mock("@/lib/invokeCommand", () => ({
  invokeCommand: (...args: unknown[]) => invokeCommand(...args),
  invokeWriteCommand: (...args: unknown[]) => invokeWriteCommand(...args),
  createIdempotencyKey: (command: string) => `${command}:test`,
}));

import { useAssistantStore } from "./useAssistantStore";
import { useAuthStore } from "./useAuthStore";
import type {
  AssistantConversationSummary,
  ChatMessage,
  ProposedAction,
  StoredMessage,
} from "@/types/assistant";
import { isActionExpired, CARD_TTL_MS, MESSAGE_WINDOW } from "@/types/assistant";

const sampleAction: ProposedAction = {
  kind: "check_in",
  payload: {
    room_id: "R1",
    guests: [{ full_name: "Nguyễn Văn Nam" }],
    nights: 2,
    source: "walk-in",
    notes: null,
    paid_amount: 500000,
    pricing_type: "nightly",
  },
  display: {
    room_id: "R1",
    guests: "Nguyễn Văn Nam",
    nights: "2 đêm",
    source: "walk-in",
    notes: "—",
    paid_amount: "500.000 ₫",
    pricing_type: "nightly",
    total: "700.000 ₫",
  },
  preview: { total: 700000 },
  warnings: [],
  built_at_ms: 1_000_000,
};

// Mốc "hiện tại" giả lập cho cả file: 1 giây sau khi sampleAction được dựng,
// nên sampleAction luôn còn hạn trừ khi một test tự dời built_at_ms đi chỗ khác.
const NOW_MS = sampleAction.built_at_ms + 1_000;

// Khoá phiên mà mỗi test bắt đầu. Đặt tay chứ không lấy giá trị store tự mint,
// để "khoá đã đổi" là một khẳng định đo được (`not.toBe(SESSION_KEY)`).
const SESSION_KEY = "key-A";

function storedMessage(id: string, kind: string, text: string): StoredMessage {
  return { id, kind, text, created_at: "2026-08-04T10:00:00+07:00" };
}

function summary(id: string, title: string): AssistantConversationSummary {
  return {
    id,
    user_id: "u1",
    user_name: "Lễ tân A",
    title,
    updated_at: "2026-08-04T10:00:00+07:00",
  };
}

describe("useAssistantStore", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW_MS);
    invokeCommand.mockReset();
    invokeWriteCommand.mockReset();
    useAssistantStore.setState({
      open: false,
      messages: [],
      pendingAction: null,
      busy: false,
      error: null,
      history: [],
      conversationKey: SESSION_KEY,
      conversationId: null,
      pendingActionKey: null,
      conversations: [],
      historyNotice: null,
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("gửi kèm ngữ cảnh màn hình trong mỗi lượt", async () => {
    invokeCommand.mockResolvedValue({ reply: "ok", proposed_action: null, history: [] });

    await useAssistantStore
      .getState()
      .send("phòng nào trống", { route: "rooms", selectedRoomNumber: "201" });

    expect(invokeCommand).toHaveBeenCalledWith("assistant_turn", {
      request: expect.objectContaining({
        message: "phòng nào trống",
        screen_context: { route: "rooms", selectedRoomNumber: "201" },
      }),
    });
  });

  it("giữ thẻ xác nhận khi backend trả về", async () => {
    invokeCommand.mockResolvedValue({
      reply: null,
      proposed_action: sampleAction,
      history: [],
    });

    await useAssistantStore.getState().send("check-in phòng R1", { route: "rooms" });

    expect(useAssistantStore.getState().pendingAction).toEqual(sampleAction);
  });

  it("duyệt thẻ thì gọi invokeWriteCommand với đúng payload", async () => {
    useAssistantStore.setState({ pendingAction: sampleAction, pendingActionKey: SESSION_KEY });
    invokeWriteCommand.mockResolvedValue({ id: "B1" });

    await useAssistantStore.getState().approve();

    expect(invokeWriteCommand).toHaveBeenCalledWith("check_in", { req: sampleAction.payload });
    expect(useAssistantStore.getState().pendingAction).toBeNull();
  });

  it("lỗi PMS lúc duyệt thì giữ nguyên thẻ để sửa", async () => {
    useAssistantStore.setState({ pendingAction: sampleAction, pendingActionKey: SESSION_KEY });
    invokeWriteCommand.mockRejectedValue(new Error("Phòng đã có khách"));

    await useAssistantStore.getState().approve();

    expect(useAssistantStore.getState().pendingAction).toEqual(sampleAction);
    expect(useAssistantStore.getState().error).toContain("Phòng đã có khách");
  });

  it("thẻ quá 5 phút bị coi là hết hạn", () => {
    expect(isActionExpired(sampleAction, sampleAction.built_at_ms + CARD_TTL_MS - 1)).toBe(false);
    expect(isActionExpired(sampleAction, sampleAction.built_at_ms + CARD_TTL_MS + 1)).toBe(true);
  });

  it("thẻ đã hết hạn thì approve() từ chối, không gọi PMS và giữ nguyên thẻ", async () => {
    const expiredAction: ProposedAction = {
      ...sampleAction,
      built_at_ms: NOW_MS - CARD_TTL_MS - 1,
    };
    useAssistantStore.setState({ pendingAction: expiredAction, pendingActionKey: SESSION_KEY });

    await useAssistantStore.getState().approve();

    expect(invokeWriteCommand).not.toHaveBeenCalled();
    expect(useAssistantStore.getState().pendingAction).toEqual(expiredAction);
    expect(useAssistantStore.getState().error).toMatch(/hết hạn/);
  });

  it("phát lại lịch sử y nguyên hai chiều: gửi đúng lịch sử đã có, nhận đúng lịch sử backend trả về", async () => {
    const seededHistory: ChatMessage[] = [
      { role: "user", content: "phòng nào trống" },
      {
        role: "assistant",
        content: null,
        tool_calls: [
          { id: "call_1", type: "function", function: { name: "list_rooms", arguments: "{}" } },
        ],
      },
      { role: "tool", content: "[]", tool_call_id: "call_1" },
    ];
    const returnedHistory: ChatMessage[] = [
      ...seededHistory,
      { role: "user", content: "check-in phòng R1" },
      { role: "assistant", content: "Đã tạo thẻ xác nhận." },
    ];

    useAssistantStore.setState({ history: seededHistory });
    invokeCommand.mockResolvedValue({
      reply: "Đã tạo thẻ xác nhận.",
      proposed_action: null,
      history: returnedHistory,
    });

    await useAssistantStore.getState().send("check-in phòng R1", { route: "rooms" });

    // Chiều đi: lịch sử gửi lên đúng bằng lịch sử đã có trong store, không rỗng,
    // không bị lọc bớt vai "tool".
    expect(invokeCommand).toHaveBeenCalledWith("assistant_turn", {
      request: expect.objectContaining({ history: seededHistory }),
    });
    // Chiều về: store phải thay bằng đúng lịch sử backend trả, không giữ lại
    // lịch sử cũ và không tự chế thêm.
    expect(useAssistantStore.getState().history).toEqual(returnedHistory);
  });

  // ─── Lớp 4: chốt duyệt nằm BÊN TRONG approve(), trên đường tiền ───
  //
  // Mọi test dưới đây gọi thẳng `approve()`, KHÔNG đi qua panel. Lớp 3 (panel
  // không vẽ thẻ ở sai hội thoại) canh việc VẼ; lớp này canh việc GHI, và nó
  // tồn tại đúng cho trường hợp thẻ vẫn còn trong store trong khi panel đã
  // thôi vẽ nó ra.

  describe("lớp 4 — approve() tự kiểm phiên trước khi chạm check_in", () => {
    it("approve() không gọi check_in khi thẻ thuộc hội thoại khác", async () => {
      useAssistantStore.setState({
        pendingAction: sampleAction,
        pendingActionKey: "key-A",
        conversationKey: "key-B",
      });

      await useAssistantStore.getState().approve();

      expect(invokeWriteCommand).not.toHaveBeenCalled();
    });

    /// `invokeWriteCommand` cố ý cho hỏng, và đó là cả sức mạnh của test này.
    /// Với mock thành công thì "thẻ biến mất" đúng cho CẢ HAI đường — bị lớp 4
    /// vứt, hoặc đã thật sự nhận phòng xong — nên khẳng định không phân biệt
    /// được gì (đo được: bỏ lớp 4 thì test vẫn xanh). Đường lỗi thì giữ nguyên
    /// thẻ để lễ tân sửa, nên "thẻ biến mất" chỉ còn một cách xảy ra.
    it("thẻ của hội thoại khác bị vứt luôn, không nằm chờ dịp khác", async () => {
      invokeWriteCommand.mockRejectedValue(new Error("Phòng đã có khách"));
      useAssistantStore.setState({
        pendingAction: sampleAction,
        pendingActionKey: "key-A",
        conversationKey: "key-B",
      });

      await useAssistantStore.getState().approve();

      const state = useAssistantStore.getState();
      expect(state.pendingAction).toBeNull();
      expect(state.pendingActionKey).toBeNull();
    });

    it("approve() vẫn gọi check_in khi thẻ thuộc đúng hội thoại đang mở", async () => {
      invokeWriteCommand.mockResolvedValue({ id: "B1" });
      useAssistantStore.setState({
        pendingAction: sampleAction,
        pendingActionKey: "key-A",
        conversationKey: "key-A",
      });

      await useAssistantStore.getState().approve();

      expect(invokeWriteCommand).toHaveBeenCalledWith("check_in", { req: sampleAction.payload });
    });

    /// Ghi DB hỏng (conversationId = null) nhưng thẻ vẫn phải duyệt được: mất
    /// sổ không được lấy mất cái thẻ lễ tân đang cần bấm.
    it("thẻ vẫn duyệt được khi không lưu được hội thoại", async () => {
      invokeWriteCommand.mockResolvedValue({ id: "B1" });
      useAssistantStore.setState({
        pendingAction: sampleAction,
        pendingActionKey: "key-A",
        conversationKey: "key-A",
        conversationId: null,
      });

      await useAssistantStore.getState().approve();

      expect(invokeWriteCommand).toHaveBeenCalledWith("check_in", { req: sampleAction.payload });
    });

    /// Lỗ `null === null`: nếu khoá so sánh là `conversationId` thì hai bên đều
    /// null và khớp nhau — thẻ cũ sống lại ngay sau khi bấm *hội thoại mới*.
    /// Khoá phiên luôn có giá trị nên trạng thái đó không tồn tại.
    it("bấm hội thoại mới khi ghi DB hỏng thì thẻ cũ không sống lại", () => {
      useAssistantStore.setState({
        pendingAction: sampleAction,
        pendingActionKey: "key-A",
        conversationKey: "key-A",
        conversationId: null,
      });

      useAssistantStore.getState().startNewChat();

      const state = useAssistantStore.getState();
      expect(state.pendingAction).toBeNull();
      expect(state.pendingActionKey).toBeNull();
      expect(state.conversationKey).not.toBe("key-A");
    });

    /// Vế còn lại của cùng lỗ, và là test **duy nhất** phân biệt được hai thiết
    /// kế khoá — vì nó KHÔNG tự đặt `pendingActionKey`, nó đọc đúng cái khoá mà
    /// `send()` đã gắn.
    ///
    /// Kịch bản: lượt dựng thẻ chạy trên một DB đang hỏng nên backend không trả
    /// được `conversation_id`; rồi lễ tân bấm *hội thoại mới* trong khi thẻ vẫn
    /// còn trong store (lớp 2 bị bỏ sót — đúng ca lớp 4 sinh ra để chặn).
    ///
    /// Khoá phiên: thẻ mang khoá cũ, phiên đã mint khoá mới → chặn.
    /// Khoá bằng `conversationId`: thẻ mang `null`, phiên cũng `null`, và
    /// `null === null` **KHỚP** → nhận phòng thật cho nhầm khách. Đo được: đổi
    /// khoá so sánh sang `conversationId` thì đúng test này đỏ vì `check_in`
    /// được gọi, còn mọi test khác đỏ vì lý do ngược lại (chặn cả thẻ hợp lệ).
    it("thẻ dựng lúc ghi DB hỏng không duyệt được sau khi bấm hội thoại mới", async () => {
      invokeCommand.mockResolvedValue({
        reply: null,
        proposed_action: sampleAction,
        history: [],
        conversation_id: null,
      });
      await useAssistantStore.getState().send("check-in phòng R1", { route: "rooms" });

      const card = useAssistantStore.getState().pendingAction;
      const cardKey = useAssistantStore.getState().pendingActionKey;
      useAssistantStore.getState().startNewChat();
      // Lớp 2 bị bỏ sót: thẻ cũ còn nguyên trong store, mang đúng khoá mà
      // `send()` đã gắn cho nó — không phải một khoá do test bịa ra.
      useAssistantStore.setState({ pendingAction: card, pendingActionKey: cardKey });

      await useAssistantStore.getState().approve();

      expect(useAssistantStore.getState().conversationId).toBeNull();
      expect(invokeWriteCommand).not.toHaveBeenCalled();
    });

    /// Chốt tranh chấp của `approve()`, đường THÀNH CÔNG.
    ///
    /// Kịch bản đo được: thẻ đang duyệt, `check_in` đang bay, lễ tân bấm *hội
    /// thoại mới* để phục vụ khách kế tiếp. Lệnh nhận phòng **vẫn phải chạy** —
    /// nó đã đi rồi — nhưng câu "Đã nhận phòng xong." mà rơi vào phiên mới là
    /// nó hiện ngay trên màn hình của một người khách khác.
    it("đổi hội thoại giữa lúc check_in đang bay về thì lời báo xong không rơi vào phiên mới", async () => {
      let releaseCheckIn: (value: unknown) => void = () => {};
      invokeWriteCommand.mockImplementation(
        () =>
          new Promise((resolve) => {
            releaseCheckIn = resolve;
          }),
      );
      useAssistantStore.setState({ pendingAction: sampleAction, pendingActionKey: SESSION_KEY });

      const approval = useAssistantStore.getState().approve();
      useAssistantStore.getState().startNewChat();
      releaseCheckIn({ id: "B1" });
      await approval;

      // Lệnh đã bắn thì vẫn chạy: chốt này quyết định KẾT QUẢ đổ đi đâu, không
      // phải huỷ một lượt nhận phòng đang dở.
      expect(invokeWriteCommand).toHaveBeenCalledWith("check_in", { req: sampleAction.payload });
      const state = useAssistantStore.getState();
      expect(state.messages).toEqual([]);
      expect(state.busy).toBe(false);
    });

    /// Vế đối xứng, đường LỖI của `approve()` — đường tệ hơn: chuỗi lỗi của PMS
    /// hiện trong một hội thoại không liên quan, còn cái thẻ để sửa thì đã mất
    /// theo phiên cũ nên lễ tân chẳng làm gì được với nó.
    it("đổi hội thoại giữa lúc check_in đang lỗi thì chuỗi lỗi không rơi vào phiên mới", async () => {
      invokeWriteCommand.mockRejectedValue(new Error("Phòng đã có khách"));
      useAssistantStore.setState({ pendingAction: sampleAction, pendingActionKey: SESSION_KEY });

      const approval = useAssistantStore.getState().approve();
      useAssistantStore.getState().startNewChat();
      await approval;

      expect(invokeWriteCommand).toHaveBeenCalledWith("check_in", { req: sampleAction.payload });
      const state = useAssistantStore.getState();
      expect(state.error).toBeNull();
      expect(state.busy).toBe(false);
    });

    it("dismissAction() gỡ cả khoá của thẻ, không để lại khoá mồ côi", () => {
      useAssistantStore.setState({ pendingAction: sampleAction, pendingActionKey: SESSION_KEY });

      useAssistantStore.getState().dismissAction();

      expect(useAssistantStore.getState().pendingAction).toBeNull();
      expect(useAssistantStore.getState().pendingActionKey).toBeNull();
    });
  });

  // ─── Nối dây conversation_id và gắn thẻ vào đúng phiên ───

  describe("khoá phiên và conversation_id", () => {
    it("send() gắn thẻ vào đúng phiên nó được dựng ra", async () => {
      invokeCommand.mockResolvedValue({
        reply: null,
        proposed_action: sampleAction,
        history: [],
        conversation_id: "c1",
      });

      await useAssistantStore.getState().send("check-in phòng R1", { route: "rooms" });

      const state = useAssistantStore.getState();
      expect(state.pendingActionKey).toBe(SESSION_KEY);
      expect(state.pendingActionKey).toBe(state.conversationKey);
    });

    it("lượt đầu gửi conversation_id rỗng rồi giữ lấy id backend trả về", async () => {
      invokeCommand.mockResolvedValue({
        reply: "ok",
        proposed_action: null,
        history: [],
        conversation_id: "c-moi",
      });

      await useAssistantStore.getState().send("phòng nào trống", { route: "rooms" });

      expect(invokeCommand).toHaveBeenCalledWith("assistant_turn", {
        request: expect.objectContaining({ conversation_id: null }),
      });
      expect(useAssistantStore.getState().conversationId).toBe("c-moi");
    });

    it("lượt sau gửi kèm đúng conversation_id đang mở", async () => {
      useAssistantStore.setState({ conversationId: "c-cu" });
      invokeCommand.mockResolvedValue({
        reply: "ok",
        proposed_action: null,
        history: [],
        conversation_id: "c-cu",
      });

      await useAssistantStore.getState().send("hỏi tiếp", { route: "rooms" });

      expect(invokeCommand).toHaveBeenCalledWith("assistant_turn", {
        request: expect.objectContaining({ conversation_id: "c-cu" }),
      });
    });

    /// Ca 3b của spec: backend không trả được id thì KHÔNG được rơi về null —
    /// rơi về null là lượt kế tiếp mở hội thoại mới và cuộc trò chuyện đang dở
    /// bị chẻ làm hai bản ghi rời.
    it("backend không trả được id thì giữ nguyên id đang mở, không chẻ sổ", async () => {
      useAssistantStore.setState({ conversationId: "c-cu" });
      invokeCommand.mockResolvedValue({
        reply: "ok",
        proposed_action: null,
        history: [],
        conversation_id: null,
      });

      await useAssistantStore.getState().send("hỏi tiếp", { route: "rooms" });

      expect(useAssistantStore.getState().conversationId).toBe("c-cu");
    });

    /// `conversationKey` là khoá phiên, không dính gì tới việc ghi DB thành hay
    /// bại (spec dòng 448-450).
    it("ghi DB hỏng không làm đổi khoá phiên", async () => {
      invokeCommand.mockResolvedValue({
        reply: "ok",
        proposed_action: null,
        history: [],
        conversation_id: null,
      });

      await useAssistantStore.getState().send("phòng nào trống", { route: "rooms" });

      expect(useAssistantStore.getState().conversationKey).toBe(SESSION_KEY);
    });

    /// Câu trả lời của hội thoại cũ bay về sau khi đã đổi phiên: `history` của
    /// nó là transcript của khách trước — tên và CCCD của người không liên
    /// quan. Đổ nó vào phiên mới là rò dữ liệu khách, không phải lỗi tiện dụng.
    it("đổi hội thoại giữa lúc câu trả lời đang bay về thì lượt cũ không đổ vào phiên mới", async () => {
      let releaseTurn: (response: unknown) => void = () => {};
      invokeCommand.mockImplementation(
        () =>
          new Promise((resolve) => {
            releaseTurn = resolve;
          }),
      );

      const turn = useAssistantStore.getState().send("phòng nào trống", { route: "rooms" });
      useAssistantStore.getState().startNewChat();
      releaseTurn({
        reply: "của hội thoại cũ",
        proposed_action: sampleAction,
        history: [{ role: "user", content: "Khách Nguyễn Văn A, CCCD 001" }],
        conversation_id: "c-cu",
      });
      await turn;

      const state = useAssistantStore.getState();
      expect(state.history).toEqual([]);
      expect(state.messages).toEqual([]);
      expect(state.conversationId).toBeNull();
      expect(state.pendingAction).toBeNull();
      expect(state.busy).toBe(false);
    });

    /// Vế đối xứng của test ngay trên, cho đường LỖI của `send()`. Không có nó
    /// thì bốn dòng chốt trong khối `catch` xoá đi vẫn xanh cả bộ (đo được), và
    /// một đợt dọn dẹp sau này sẽ dọn mất chúng: bong bóng lỗi + `error` của
    /// hội thoại cũ đổ vào phiên mới, ngay dưới mắt khách kế tiếp.
    ///
    /// `mockRejectedValue` là đủ để dựng ca này: `send()` chạy đồng bộ tới chỗ
    /// `await`, test bấm *hội thoại mới* ngay sau đó, khối `catch` chỉ chạy ở
    /// microtask sau — đúng thứ tự cần đo.
    it("đổi hội thoại giữa lúc lượt cũ đang lỗi thì bong bóng lỗi không rơi vào phiên mới", async () => {
      invokeCommand.mockRejectedValue(new Error("Nhà cung cấp AI không phản hồi"));

      const turn = useAssistantStore.getState().send("phòng nào trống", { route: "rooms" });
      useAssistantStore.getState().startNewChat();
      await turn;

      const state = useAssistantStore.getState();
      expect(state.messages).toEqual([]);
      expect(state.error).toBeNull();
      expect(state.busy).toBe(false);
    });
  });

  // ─── history khi đổi hội thoại — đường rò CCCD xuyên hội thoại ───

  describe("startNewChat và openConversation", () => {
    it("startNewChat() dọn sạch phiên và mint khoá mới", () => {
      useAssistantStore.setState({
        messages: [{ id: "m1", kind: "user", text: "của hội thoại cũ" }],
        history: [{ role: "user", content: "Khách Nguyễn Văn A, CCCD 001" }],
        conversationId: "c1",
        pendingAction: sampleAction,
        pendingActionKey: SESSION_KEY,
        error: "lỗi cũ",
        historyNotice: "nhắc cũ",
      });

      useAssistantStore.getState().startNewChat();

      const state = useAssistantStore.getState();
      expect(state.messages).toEqual([]);
      expect(state.history).toEqual([]);
      expect(state.conversationId).toBeNull();
      expect(state.pendingAction).toBeNull();
      expect(state.pendingActionKey).toBeNull();
      expect(state.error).toBeNull();
      expect(state.historyNotice).toBeNull();
      expect(state.conversationKey).not.toBe(SESSION_KEY);
    });

    it("bấm hội thoại mới thì lượt kế tiếp gửi đi history rỗng", async () => {
      useAssistantStore.setState({
        history: [{ role: "user", content: "Khách Nguyễn Văn A, CCCD 001" }],
      });
      invokeCommand.mockResolvedValue({
        reply: "ok",
        proposed_action: null,
        history: [],
        conversation_id: "c-moi",
      });

      useAssistantStore.getState().startNewChat();
      await useAssistantStore.getState().send("phòng nào trống", { route: "rooms" });

      expect(invokeCommand).toHaveBeenCalledWith("assistant_turn", {
        request: expect.objectContaining({ history: [], conversation_id: null }),
      });
    });

    it("mở hội thoại cũ thì history chỉ dựng lại từ user và assistant", async () => {
      invokeCommand.mockResolvedValue([
        storedMessage("m1", "user", "Khách Nguyễn Văn A, CCCD 001"),
        storedMessage("m2", "assistant", "Còn phòng 101."),
        storedMessage("m3", "action", "Đề xuất nhận phòng:\n- room_id: R201"),
        storedMessage("m4", "error", "Trợ lý gặp lỗi."),
      ]);

      await useAssistantStore.getState().openConversation("c1");

      expect(invokeCommand).toHaveBeenCalledWith("get_assistant_conversation", {
        conversationId: "c1",
      });
      expect(useAssistantStore.getState().history).toEqual([
        { role: "user", content: "Khách Nguyễn Văn A, CCCD 001" },
        { role: "assistant", content: "Còn phòng 101." },
      ]);
    });

    it("mở hội thoại cũ vẫn hiện đủ bốn hàng trên panel, kể cả error và action", async () => {
      invokeCommand.mockResolvedValue([
        storedMessage("m1", "user", "Khách Nguyễn Văn A"),
        storedMessage("m2", "assistant", "Còn phòng 101."),
        storedMessage("m3", "action", "Đề xuất nhận phòng:\n- room_id: R201"),
        storedMessage("m4", "error", "Trợ lý gặp lỗi."),
      ]);

      await useAssistantStore.getState().openConversation("c1");

      expect(useAssistantStore.getState().messages).toEqual([
        { id: "m1", kind: "user", text: "Khách Nguyễn Văn A" },
        { id: "m2", kind: "assistant", text: "Còn phòng 101." },
        { id: "m3", kind: "assistant", text: "Đề xuất nhận phòng:\n- room_id: R201" },
        { id: "m4", kind: "error", text: "Trợ lý gặp lỗi." },
      ]);
    });

    /// Test chống rò dữ liệu khách, không phải test tiện ích: `history` là bản
    /// ghi gửi cho nhà cung cấp AI, và luật lọc `user_id` phía Rust không nhìn
    /// thấy đường này.
    it("mở hội thoại khác thì không còn một dòng history nào của hội thoại trước", async () => {
      useAssistantStore.setState({
        history: [{ role: "user", content: "Khách hội thoại trước, CCCD 001" }],
        messages: [{ id: "cu", kind: "user", text: "Khách hội thoại trước, CCCD 001" }],
      });
      invokeCommand.mockResolvedValue([storedMessage("m1", "user", "Hỏi giá phòng đôi")]);

      await useAssistantStore.getState().openConversation("c2");

      const state = useAssistantStore.getState();
      expect(state.history).toEqual([{ role: "user", content: "Hỏi giá phòng đôi" }]);
      expect(JSON.stringify(state.history)).not.toContain("CCCD");
      expect(JSON.stringify(state.messages)).not.toContain("CCCD");
    });

    it("mở hội thoại từ lịch sử thì thẻ đang treo mất quyền duyệt", async () => {
      useAssistantStore.setState({ pendingAction: sampleAction, pendingActionKey: SESSION_KEY });
      invokeCommand.mockResolvedValue([storedMessage("m1", "user", "Hỏi giá")]);

      await useAssistantStore.getState().openConversation("c2");

      const state = useAssistantStore.getState();
      expect(state.pendingAction).toBeNull();
      expect(state.pendingActionKey).toBeNull();
      expect(state.conversationKey).not.toBe(SESSION_KEY);
      expect(state.conversationId).toBe("c2");
    });

    it("mở hội thoại hỏng thì không đổi phiên, chỉ báo lỗi", async () => {
      useAssistantStore.setState({
        history: [{ role: "user", content: "Khách đang dở, CCCD 001" }],
        conversationId: "c1",
      });
      invokeCommand.mockRejectedValue(new Error("Không tìm thấy hội thoại"));

      await useAssistantStore.getState().openConversation("c2");

      const state = useAssistantStore.getState();
      expect(state.conversationId).toBe("c1");
      expect(state.conversationKey).toBe(SESSION_KEY);
      expect(state.history).toEqual([{ role: "user", content: "Khách đang dở, CCCD 001" }]);
      expect(state.error).toContain("Không tìm thấy hội thoại");
    });

    it("hội thoại có hàng action thì nhắc trợ lý không nhớ thẻ đã đề xuất", async () => {
      invokeCommand.mockResolvedValue([
        storedMessage("m1", "user", "check-in phòng R1"),
        storedMessage("m2", "action", "Đề xuất nhận phòng:\n- room_id: R201"),
      ]);

      await useAssistantStore.getState().openConversation("c1");

      expect(useAssistantStore.getState().historyNotice).toMatch(/không nhớ thẻ/);
    });

    it("hội thoại chạm trần 100 tin thì nhắc chỉ 100 tin gần nhất", async () => {
      invokeCommand.mockResolvedValue(
        Array.from({ length: MESSAGE_WINDOW }, (_, index) =>
          storedMessage(`m${index}`, index % 2 === 0 ? "user" : "assistant", `tin ${index}`),
        ),
      );

      await useAssistantStore.getState().openConversation("c1");

      expect(useAssistantStore.getState().historyNotice).toMatch(/100 tin gần nhất/);
    });

    /// Hai dòng nhắc đều CÓ ĐIỀU KIỆN. Vô điều kiện thì chúng nằm cả trên hội
    /// thoại hai tin nhắn chưa mất gì — thành nhiễu, và nhiễu thường trực thì
    /// người ta thôi đọc.
    it("hội thoại ngắn không có hàng action thì không nhắc gì", async () => {
      invokeCommand.mockResolvedValue([
        storedMessage("m1", "user", "phòng nào trống"),
        storedMessage("m2", "assistant", "Còn phòng 101."),
      ]);

      await useAssistantStore.getState().openConversation("c1");

      expect(useAssistantStore.getState().historyNotice).toBeNull();
    });
  });

  // ─── Danh sách lịch sử và hai lệnh xoá ───

  describe("danh sách lịch sử và xoá", () => {
    it("loadConversations() nạp danh sách, không gửi kèm danh tính nào", async () => {
      invokeCommand.mockResolvedValue([summary("c1", "Hỏi phòng")]);

      await useAssistantStore.getState().loadConversations();

      expect(invokeCommand).toHaveBeenCalledWith("list_assistant_conversations");
      expect(useAssistantStore.getState().conversations).toEqual([summary("c1", "Hỏi phòng")]);
    });

    it("xoá đúng hội thoại đang mở thì về hội thoại mới và nạp lại danh sách", async () => {
      useAssistantStore.setState({
        conversationId: "c1",
        messages: [{ id: "m1", kind: "user", text: "của hội thoại vừa xoá" }],
        history: [{ role: "user", content: "Khách Nguyễn Văn A, CCCD 001" }],
        conversations: [summary("c1", "Hỏi phòng")],
      });
      invokeWriteCommand.mockResolvedValue(1);
      invokeCommand.mockResolvedValue([]);

      await useAssistantStore.getState().deleteConversation("c1");

      expect(invokeWriteCommand).toHaveBeenCalledWith("delete_assistant_conversation", {
        conversationId: "c1",
      });
      const state = useAssistantStore.getState();
      expect(state.conversationId).toBeNull();
      expect(state.messages).toEqual([]);
      expect(state.history).toEqual([]);
      expect(state.conversationKey).not.toBe(SESSION_KEY);
      expect(state.conversations).toEqual([]);
    });

    it("xoá một hội thoại khác thì không đụng hội thoại đang mở", async () => {
      useAssistantStore.setState({
        conversationId: "c1",
        messages: [{ id: "m1", kind: "user", text: "đang dở" }],
      });
      invokeWriteCommand.mockResolvedValue(1);
      invokeCommand.mockResolvedValue([summary("c1", "Hỏi phòng")]);

      await useAssistantStore.getState().deleteConversation("c2");

      const state = useAssistantStore.getState();
      expect(state.conversationId).toBe("c1");
      expect(state.conversationKey).toBe(SESSION_KEY);
      expect(state.messages).toEqual([{ id: "m1", kind: "user", text: "đang dở" }]);
      expect(state.conversations).toEqual([summary("c1", "Hỏi phòng")]);
    });

    it("deleteAllConversations() dọn sạch phiên và nạp lại danh sách rỗng", async () => {
      useAssistantStore.setState({
        conversationId: "c1",
        messages: [{ id: "m1", kind: "user", text: "đang dở" }],
        conversations: [summary("c1", "Hỏi phòng"), summary("c2", "Hỏi giá")],
      });
      invokeWriteCommand.mockResolvedValue(2);
      invokeCommand.mockResolvedValue([]);

      await useAssistantStore.getState().deleteAllConversations();

      expect(invokeWriteCommand).toHaveBeenCalledWith("delete_all_assistant_conversations");
      const state = useAssistantStore.getState();
      expect(state.conversationId).toBeNull();
      expect(state.messages).toEqual([]);
      expect(state.conversationKey).not.toBe(SESSION_KEY);
      expect(state.conversations).toEqual([]);
    });

    /// Hai lệnh xoá là admin only; lễ tân gọi nhận `AUTH_FORBIDDEN`. Giấu nút
    /// xoá là chuyện của panel, KHÔNG phải hàng rào — store bị từ chối thì
    /// không được dọn gì cả, không thì một cú bấm lọt lưới vẫn thổi bay hội
    /// thoại đang dở trên màn hình dù backend chẳng xoá dòng nào.
    it("lễ tân bị từ chối thì store không dọn gì, chỉ báo lỗi", async () => {
      useAssistantStore.setState({
        conversationId: "c1",
        messages: [{ id: "m1", kind: "user", text: "đang dở" }],
        conversations: [summary("c1", "Hỏi phòng")],
      });
      invokeWriteCommand.mockRejectedValue(new Error("Chỉ admin mới được thực hiện"));

      await useAssistantStore.getState().deleteConversation("c1");
      await useAssistantStore.getState().deleteAllConversations();

      const state = useAssistantStore.getState();
      expect(state.conversationId).toBe("c1");
      expect(state.conversationKey).toBe(SESSION_KEY);
      expect(state.messages).toEqual([{ id: "m1", kind: "user", text: "đang dở" }]);
      expect(state.conversations).toEqual([summary("c1", "Hỏi phòng")]);
      expect(state.error).toContain("Chỉ admin mới được thực hiện");
    });
  });

  // ─── Đổi người dùng — lớp 4 phải sống sót qua màn hình PIN ───
  //
  // Store zustand là singleton của module và trong `src/` không có chỗ nào
  // `location.reload`, nên không dọn tay thì mọi thứ ở đây sống nguyên qua
  // `logout()`. Test đi qua `useAuthStore.getState().logout()` chứ không gọi
  // thẳng `resetForLogout()`: cái phải đo là SỢI DÂY đã nối, không phải cái
  // hàm dọn tồn tại.

  describe("đăng xuất", () => {
    it("đăng xuất xoá sạch phiên của người trước và mint khoá mới", async () => {
      useAssistantStore.setState({
        open: true,
        conversationId: "c1",
        messages: [{ id: "m1", kind: "user", text: "Khách Nguyễn Văn A, CCCD 001" }],
        history: [{ role: "user", content: "Khách Nguyễn Văn A, CCCD 001" }],
        conversations: [summary("c1", "Hỏi phòng")],
        pendingAction: sampleAction,
        pendingActionKey: SESSION_KEY,
        historyNotice: "nhắc cũ",
        error: "lỗi cũ",
      });

      await useAuthStore.getState().logout();

      const state = useAssistantStore.getState();
      expect(state.pendingAction).toBeNull();
      expect(state.pendingActionKey).toBeNull();
      expect(state.history).toEqual([]);
      expect(state.messages).toEqual([]);
      expect(state.conversations).toEqual([]);
      expect(state.conversationId).toBeNull();
      expect(state.historyNotice).toBeNull();
      expect(state.error).toBeNull();
      expect(state.open).toBe(false);
      // Chính cú mint này là thứ làm lớp 4 vứt thẻ của người trước.
      expect(state.conversationKey).not.toBe(SESSION_KEY);
    });

    /// Ca tiền: lễ tân A để lại một thẻ nhận phòng đang treo, A đăng xuất, B
    /// đăng nhập và bấm duyệt. Không dọn store thì `pendingActionKey ===
    /// conversationKey` vẫn khớp và `check_in` chạy thật — nhận phòng thật,
    /// tiền thật, dưới danh nghĩa người khác.
    it("thẻ còn treo của người trước không duyệt được sau khi đăng xuất", async () => {
      invokeWriteCommand.mockResolvedValue({ id: "B1" });
      useAssistantStore.setState({
        pendingAction: sampleAction,
        pendingActionKey: SESSION_KEY,
        conversationKey: SESSION_KEY,
      });

      await useAuthStore.getState().logout();
      await useAssistantStore.getState().approve();

      expect(invokeWriteCommand).not.toHaveBeenCalled();
    });
  });
});
