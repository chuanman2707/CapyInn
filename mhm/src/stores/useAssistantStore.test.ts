import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invokeCommand = vi.fn();
const invokeWriteCommand = vi.fn();

vi.mock("@/lib/invokeCommand", () => ({
  invokeCommand: (...args: unknown[]) => invokeCommand(...args),
  invokeWriteCommand: (...args: unknown[]) => invokeWriteCommand(...args),
  createIdempotencyKey: (command: string) => `${command}:test`,
}));

import { BUSY_REFUSAL, useAssistantStore } from "./useAssistantStore";
import { useAuthStore } from "./useAuthStore";
import { createAppErrorException } from "@/lib/appError";
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

/// Một promise mở, để test cầm được thời điểm lệnh "bay về".
///
/// Cả bộ test tranh chấp dưới đây sống nhờ nó: kịch bản duy nhất sinh ra lỗ
/// duyệt hai lần là **hai lệnh cùng bay** và lệnh cũ về sau, nên không giữ được
/// lệnh nào ở trạng thái lơ lửng thì không dựng lại được kịch bản.
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const turnResponse = (reply: string) => ({
  reply,
  proposed_action: null,
  history: [],
  conversation_id: null,
});

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
    /// Kịch bản đo được: thẻ đang duyệt, `check_in` đang bay, phiên đổi. Lệnh
    /// nhận phòng **vẫn phải chạy** — nó đã đi rồi — nhưng câu "Đã nhận phòng
    /// xong." mà rơi vào phiên mới là nó hiện ngay trên màn hình của một người
    /// khách khác.
    ///
    /// Đổi phiên bằng `resetForLogout()` chứ không bằng `startNewChat()`: từ
    /// bất biến sở hữu `busy`, đăng xuất là đường mint **duy nhất** chạy được
    /// khi đang bận. `startNewChat()` ở đây bị chính hàng rào của nó từ chối,
    /// nên viết như cũ là dựng một kịch bản không xảy ra được nữa.
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
      useAssistantStore.getState().resetForLogout();
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
      useAssistantStore.getState().resetForLogout();
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

  // ─── BẤT BIẾN SỞ HỮU `busy` — bốn nhánh stale, mỗi nhánh một test ───
  //
  // Bất biến: **ai mint khoá phiên mới thì người đó sở hữu `busy` của phiên
  // mới; một lượt cũ bay về muộn KHÔNG được đụng vào.**
  //
  // Trước bộ này, cả bốn nhánh "khoá phiên đã đổi" đều `set({ busy: false })`
  // vô điều kiện và **KHÔNG một test nào canh chúng theo bất kỳ chiều nào**: đo
  // được — đổi cả bốn sang `return` trần thì 784/784 vẫn xanh. Nên mỗi nhánh có
  // đúng một test riêng ở đây, và mỗi test đỏ khi và chỉ khi nhánh của nó dọn
  // hộ `busy`.
  //
  // Hình dạng chung của cả bốn: dựng một `busy` **thuộc về phiên mới**, thả cho
  // lượt cũ bay về, rồi khẳng định cờ ấy còn nguyên. Đổi phiên luôn bằng
  // `resetForLogout()` vì đó là đường mint duy nhất chạy được khi đang bận.

  describe("bất biến sở hữu busy — lượt cũ không dọn cờ của phiên mới", () => {
    it("nhánh stale của send() (đường THÀNH CÔNG) không dọn busy của phiên mới", async () => {
      const turnA = deferred<unknown>();
      const turnB = deferred<unknown>();
      invokeCommand
        .mockImplementationOnce(() => turnA.promise)
        .mockImplementationOnce(() => turnB.promise);

      // Lễ tân A hỏi, nhà cung cấp treo.
      const a = useAssistantStore.getState().send("A hỏi", { route: "rooms" });
      expect(useAssistantStore.getState().busy).toBe(true);

      // A giao ca. Đăng xuất là đường mint duy nhất chạy được khi đang bận, và
      // nó tự đặt lại cờ vì nó là chủ phiên mới.
      useAssistantStore.getState().resetForLogout();
      expect(useAssistantStore.getState().busy).toBe(false);

      // B hỏi: từ đây `busy` là CỦA B.
      const b = useAssistantStore.getState().send("B hỏi", { route: "rooms" });
      expect(useAssistantStore.getState().busy).toBe(true);

      turnA.resolve({
        reply: "của A",
        proposed_action: sampleAction,
        history: [{ role: "user", content: "Khách của A, CCCD 001" }],
        conversation_id: "c-a",
      });
      await a;

      const state = useAssistantStore.getState();
      // Cờ của B còn nguyên — đây là câu đỏ khi nhánh stale dọn hộ.
      expect(state.busy).toBe(true);
      // Và không một mẩu nào của A rơi sang (vế cũ, giữ nguyên).
      expect(state.messages).toEqual([
        expect.objectContaining({ kind: "user", text: "B hỏi" }),
      ]);
      expect(state.pendingAction).toBeNull();
      expect(state.history).toEqual([]);

      // Vế dương: lượt của CHÍNH B về thì cờ mới được hạ.
      turnB.resolve(turnResponse("của B"));
      await b;
      expect(useAssistantStore.getState().busy).toBe(false);
    });

    it("nhánh stale của send() (đường LỖI) không dọn busy của phiên mới", async () => {
      const turnA = deferred<unknown>();
      const turnB = deferred<unknown>();
      invokeCommand
        .mockImplementationOnce(() => turnA.promise)
        .mockImplementationOnce(() => turnB.promise);

      const a = useAssistantStore.getState().send("A hỏi", { route: "rooms" });
      useAssistantStore.getState().resetForLogout();
      const b = useAssistantStore.getState().send("B hỏi", { route: "rooms" });
      expect(useAssistantStore.getState().busy).toBe(true);

      turnA.reject(new Error("Nhà cung cấp AI không phản hồi"));
      await a;

      const state = useAssistantStore.getState();
      expect(state.busy).toBe(true);
      // Bong bóng lỗi và viên cảnh báo của A cũng không rơi sang (vế cũ).
      expect(state.error).toBeNull();
      expect(state.messages).toEqual([
        expect.objectContaining({ kind: "user", text: "B hỏi" }),
      ]);

      turnB.resolve(turnResponse("của B"));
      await b;
      expect(useAssistantStore.getState().busy).toBe(false);
    });

    /// Nhánh ĐẮT NHẤT trong bốn: nó nằm trên đường tiền. Dọn hộ `busy` ở đây là
    /// bật lại nút *Đồng ý* trên một thẻ vẫn còn trên màn hình.
    it("nhánh stale của approve() (đường THÀNH CÔNG) không dọn busy của phiên mới", async () => {
      const checkIn = deferred<unknown>();
      const turnB = deferred<unknown>();
      invokeWriteCommand.mockImplementation(() => checkIn.promise);
      invokeCommand.mockImplementation(() => turnB.promise);
      useAssistantStore.setState({ pendingAction: sampleAction, pendingActionKey: SESSION_KEY });

      const approval = useAssistantStore.getState().approve();
      expect(useAssistantStore.getState().busy).toBe(true);

      // Giao ca giữa lúc `check_in` đang bay.
      useAssistantStore.getState().resetForLogout();

      const b = useAssistantStore.getState().send("B hỏi", { route: "rooms" });
      expect(useAssistantStore.getState().busy).toBe(true);

      checkIn.resolve({ id: "B1" });
      await approval;

      const state = useAssistantStore.getState();
      expect(state.busy).toBe(true);
      // Câu "Đã nhận phòng xong." của A cũng không rơi sang (vế cũ).
      expect(state.messages).toEqual([
        expect.objectContaining({ kind: "user", text: "B hỏi" }),
      ]);

      turnB.resolve(turnResponse("của B"));
      await b;
      expect(useAssistantStore.getState().busy).toBe(false);
    });

    it("nhánh stale của approve() (đường LỖI) không dọn busy của phiên mới", async () => {
      const checkIn = deferred<unknown>();
      const turnB = deferred<unknown>();
      invokeWriteCommand.mockImplementation(() => checkIn.promise);
      invokeCommand.mockImplementation(() => turnB.promise);
      useAssistantStore.setState({ pendingAction: sampleAction, pendingActionKey: SESSION_KEY });

      const approval = useAssistantStore.getState().approve();
      useAssistantStore.getState().resetForLogout();
      const b = useAssistantStore.getState().send("B hỏi", { route: "rooms" });
      expect(useAssistantStore.getState().busy).toBe(true);

      checkIn.reject(new Error("Phòng đã có khách"));
      await approval;

      const state = useAssistantStore.getState();
      expect(state.busy).toBe(true);
      expect(state.error).toBeNull();

      turnB.resolve(turnResponse("của B"));
      await b;
      expect(useAssistantStore.getState().busy).toBe(false);
    });

    /// KỊCH BẢN TIỀN, đo đầu-cuối. Bốn test trên canh từng dòng; test này canh
    /// cái GIÁ của bốn dòng ấy cộng lại.
    ///
    /// Trước bản vá, đo được: `invokeWriteCommand` gọi **2 lần**, cả hai
    /// `"check_in"`, **cùng payload** — và `createIdempotencyKey` sinh UUID mới
    /// mỗi lượt gọi nên backend không dedupe được. Nhận phòng thật, hai lần.
    it("lượt cũ của người trước bay về không mở đường bắn check_in lần thứ hai", async () => {
      const turnA = deferred<unknown>();
      const checkIn = deferred<unknown>();
      invokeCommand
        .mockImplementationOnce(() => turnA.promise)
        .mockImplementationOnce(async () => ({
          reply: null,
          proposed_action: sampleAction,
          history: [],
          conversation_id: "c-b",
        }));
      invokeWriteCommand.mockImplementation(() => checkIn.promise);

      // A hỏi → nhà cung cấp treo.
      const a = useAssistantStore.getState().send("A hỏi", { route: "rooms" });
      // A đăng xuất, B đăng nhập.
      useAssistantStore.getState().resetForLogout();
      // B hỏi và nhận được thẻ nhận phòng.
      await useAssistantStore.getState().send("nhận phòng R1", { route: "rooms" });
      expect(useAssistantStore.getState().pendingAction).toEqual(sampleAction);

      // B bấm *Đồng ý*: `check_in` bay đi, nút xám, hiện "Đang gửi lệnh…".
      const approval = useAssistantStore.getState().approve();
      expect(useAssistantStore.getState().busy).toBe(true);
      expect(invokeWriteCommand).toHaveBeenCalledTimes(1);

      // Lượt CŨ của A bay về muộn.
      turnA.resolve({ reply: "của A", proposed_action: null, history: [], conversation_id: "c-a" });
      await a;

      const afterStale = useAssistantStore.getState();
      // Nút *Đồng ý* KHÔNG được sáng lại...
      expect(afterStale.busy).toBe(true);
      // ...trong khi thẻ thì vẫn còn nguyên trên màn hình. Chính cặp này —
      // thẻ còn + nút sáng — là cú bấm thứ hai.
      expect(afterStale.pendingAction).toEqual(sampleAction);

      // Và cú bấm thứ hai, nếu có, bị store chặn ở câu đầu của `approve()`.
      await useAssistantStore.getState().approve();
      const checkInCalls = invokeWriteCommand.mock.calls.filter(([command]) => command === "check_in");
      expect(checkInCalls).toHaveLength(1);

      checkIn.resolve({ id: "B1" });
      await approval;

      expect(
        invokeWriteCommand.mock.calls.filter(([command]) => command === "check_in"),
      ).toHaveLength(1);
      expect(useAssistantStore.getState().messages).toContainEqual(
        expect.objectContaining({ kind: "assistant", text: "Đã nhận phòng xong." }),
      );
    });
  });

  // ─── HÀNG RÀO `busy` Ở TẦNG STORE ───
  //
  // `disabled={busy}` trên nút là **lấy mẫu lúc render**, không phải hàng rào:
  // cú bấm đã đi rồi thì `busy` bật lên sau đó không thu lại được. Comment
  // trong `send()` viết đúng câu ấy: "khoá nút là kỷ luật của panel, không phải
  // hàng rào." Bộ này canh hàng rào thật, ở tầng store.

  describe("hàng rào busy — không mint khoá phiên khi lệnh đang bay", () => {
    it("startNewChat() bị từ chối khi đang bận và KHÔNG dọn gì", () => {
      useAssistantStore.setState({
        busy: true,
        conversationId: "c1",
        messages: [{ id: "m1", kind: "user", text: "đang dở" }],
        pendingAction: sampleAction,
        pendingActionKey: SESSION_KEY,
      });

      useAssistantStore.getState().startNewChat();

      const state = useAssistantStore.getState();
      expect(state.conversationKey).toBe(SESSION_KEY);
      expect(state.conversationId).toBe("c1");
      expect(state.messages).toEqual([{ id: "m1", kind: "user", text: "đang dở" }]);
      expect(state.pendingAction).toEqual(sampleAction);
      // Từ chối phải NÓI RA: `error` được vẽ ở cả panel lẫn Cài đặt, nên một cú
      // bấm không có hồi đáp là một cú bấm người ta sẽ bấm lại.
      expect(state.error).toBe(BUSY_REFUSAL);
      // Và không tự tiện hạ cờ của lệnh đang bay.
      expect(state.busy).toBe(true);
    });

    it("openConversation() bị từ chối ngay ở cửa vào, không đọc sổ", async () => {
      useAssistantStore.setState({ busy: true, conversationId: "c1" });

      await useAssistantStore.getState().openConversation("c2");

      expect(invokeCommand).not.toHaveBeenCalled();
      const state = useAssistantStore.getState();
      expect(state.conversationId).toBe("c1");
      expect(state.conversationKey).toBe(SESSION_KEY);
      expect(state.error).toBe(BUSY_REFUSAL);
    });

    /// Cửa vào cũng chỉ là **lấy mẫu**. Trong lúc đọc sổ, thẻ vẫn đang được vẽ
    /// (lớp 3 so `pendingActionKey === conversationKey`, mà khoá chưa đổi) và
    /// nút *Đồng ý* vẫn sáng — nên cú bấm rơi đúng vào khe giữa cửa vào và cú
    /// mint. Câu chặn thật là câu kiểm LẠI sau `await`.
    it("busy nổi lên giữa lúc đọc sổ thì openConversation() vẫn không mint khoá", async () => {
      const read = deferred<StoredMessage[]>();
      invokeCommand.mockImplementation(() => read.promise);
      invokeWriteCommand.mockImplementation(() => new Promise(() => {}));
      useAssistantStore.setState({ pendingAction: sampleAction, pendingActionKey: SESSION_KEY });

      const opening = useAssistantStore.getState().openConversation("c2");
      void useAssistantStore.getState().approve();
      expect(useAssistantStore.getState().busy).toBe(true);

      read.resolve([storedMessage("m1", "user", "Hỏi giá phòng đôi")]);
      await opening;

      const state = useAssistantStore.getState();
      expect(state.conversationKey).toBe(SESSION_KEY);
      expect(state.conversationId).toBeNull();
      expect(state.messages).toEqual([]);
      expect(state.pendingAction).toEqual(sampleAction);
      expect(state.error).toBe(BUSY_REFUSAL);
    });

    it("deleteConversation() bị từ chối khi đang bận, không xoá dòng nào", async () => {
      useAssistantStore.setState({
        busy: true,
        conversationId: "c1",
        conversations: [summary("c1", "Hỏi phòng")],
      });

      await useAssistantStore.getState().deleteConversation("c1");

      expect(invokeWriteCommand).not.toHaveBeenCalled();
      const state = useAssistantStore.getState();
      expect(state.conversations).toEqual([summary("c1", "Hỏi phòng")]);
      expect(state.conversationKey).toBe(SESSION_KEY);
      expect(state.error).toBe(BUSY_REFUSAL);
    });

    it("deleteAllConversations() bị từ chối khi đang bận, không xoá dòng nào", async () => {
      useAssistantStore.setState({
        busy: true,
        conversations: [summary("c1", "Hỏi phòng")],
      });

      await useAssistantStore.getState().deleteAllConversations();

      expect(invokeWriteCommand).not.toHaveBeenCalled();
      const state = useAssistantStore.getState();
      expect(state.conversations).toEqual([summary("c1", "Hỏi phòng")]);
      expect(state.conversationKey).toBe(SESSION_KEY);
      expect(state.error).toBe(BUSY_REFUSAL);
    });

    /// KỊCH BẢN CỬA THỨ NĂM, đo đầu-cuối — cửa xoá sạch ở Cài đặt.
    ///
    /// Trước bản vá: admin bấm *Xoá vĩnh viễn* lúc rảnh (cửa cho qua) → lễ tân
    /// bấm *Đồng ý* trên thẻ ở panel bên trái → `check_in` bay đi → lệnh xoá về
    /// → `startNewChat()` mint khoá mới giữa lúc lệnh ghi đang bay → lớp 4 vứt
    /// kết quả `check_in`, `pendingAction` mất, màn hình KHÔNG NÓI GÌ. Phòng đã
    /// nhận thật. Không mất tiền, mất tin.
    it("xoá sạch không mint khoá khi check_in đang bay, và kết quả check_in vẫn tới màn hình", async () => {
      const del = deferred<unknown>();
      const checkIn = deferred<unknown>();
      invokeWriteCommand.mockImplementation((command: string) =>
        command === "check_in" ? checkIn.promise : del.promise,
      );
      invokeCommand.mockResolvedValue([]);
      useAssistantStore.setState({
        pendingAction: sampleAction,
        pendingActionKey: SESSION_KEY,
        conversationId: "c1",
        conversations: [summary("c1", "Hỏi phòng")],
      });

      // Admin bấm *Xoá vĩnh viễn* lúc rảnh: cửa vào cho qua.
      const deleting = useAssistantStore.getState().deleteAllConversations();
      // Ngay sau đó lễ tân bấm *Đồng ý* trên thẻ ở panel bên trái.
      const approval = useAssistantStore.getState().approve();
      expect(useAssistantStore.getState().busy).toBe(true);

      del.resolve(2);
      await deleting;

      const afterDelete = useAssistantStore.getState();
      // Không mint khoá phiên giữa lúc một lệnh ghi đang bay...
      expect(afterDelete.conversationKey).toBe(SESSION_KEY);
      // ...và admin đọc được vì sao phiên trên panel vẫn còn nguyên. Câu này
      // phải sống sót qua `loadConversations()` — nó dọn `error` khi nạp được.
      expect(afterDelete.error).toBe(BUSY_REFUSAL);
      // Lệnh xoá thì vẫn chạy thật: sổ trên đĩa đã rỗng.
      expect(afterDelete.conversations).toEqual([]);

      checkIn.resolve({ id: "B1" });
      await approval;

      const afterCheckIn = useAssistantStore.getState();
      // Đây là câu quan trọng nhất: kết quả nhận phòng TỚI ĐƯỢC màn hình thay
      // vì rơi vào hư không.
      expect(afterCheckIn.messages).toContainEqual(
        expect.objectContaining({ kind: "assistant", text: "Đã nhận phòng xong." }),
      );
      expect(afterCheckIn.pendingAction).toBeNull();
      expect(afterCheckIn.busy).toBe(false);
    });

    /// VẾ SONG SINH, và là vế TRƯỚC ĐÂY KHÔNG AI CANH.
    ///
    /// `deleteAllConversations` có test ngay trên; `deleteConversation` thì
    /// không: đo được — gỡ hẳn `if (refused) set({ error: BUSY_REFUSAL })` ở
    /// `deleteConversation` mà cả bộ vẫn 799/799 xanh, trong khi gỡ đúng dòng ấy
    /// ở `deleteAllConversations` thì đỏ 1.
    ///
    /// Chế độ hỏng khi ai đó "dọn dẹp" mất nó: admin xoá một hội thoại **đang
    /// mở** giữa lúc `check_in` bay → dòng biến mất khỏi đĩa, panel vẫn ngồi
    /// trên nó, **màn hình không một chữ**.
    ///
    /// Câu này phải sống sót qua `loadConversations()` — nó dọn `error` khi nạp
    /// được — nên đo SAU khi cú xoá đã về đích, không đo ngay lúc bị từ chối.
    it("xoá hội thoại ĐANG MỞ lúc check_in đang bay thì admin đọc được vì sao phiên vẫn còn", async () => {
      const del = deferred<unknown>();
      const checkIn = deferred<unknown>();
      invokeWriteCommand.mockImplementation((command: string) =>
        command === "check_in" ? checkIn.promise : del.promise,
      );
      invokeCommand.mockResolvedValue([]);
      useAssistantStore.setState({
        pendingAction: sampleAction,
        pendingActionKey: SESSION_KEY,
        conversationId: "c1",
        conversations: [summary("c1", "Hỏi phòng")],
      });

      // Admin bấm *Xoá* trên đúng hội thoại đang mở, lúc còn rảnh: cửa cho qua.
      const deleting = useAssistantStore.getState().deleteConversation("c1");
      // Ngay sau đó lễ tân bấm *Đồng ý* trên thẻ.
      const approval = useAssistantStore.getState().approve();
      expect(useAssistantStore.getState().busy).toBe(true);

      del.resolve(1);
      await deleting;

      // Lệnh xoá CHẠY THẬT — vế dương, nếu không thì chẳng có gì để giải thích.
      expect(invokeWriteCommand).toHaveBeenCalledWith("delete_assistant_conversation", {
        conversationId: "c1",
      });
      expect(useAssistantStore.getState().error).toBe(BUSY_REFUSAL);

      checkIn.resolve({ id: "B1" });
      await approval;
    });

    /// I4(a) — trạng thái nửa vời làm CHẾT PHIÊN, đo đầu-cuối.
    ///
    /// Trước bản vá, sau cú dọn phiên bị từ chối ở trên: `conversations = []`
    /// (đĩa đã rỗng), nhưng `conversationId = "c1"` — hàng đã bị xoá. Lượt kế
    /// tiếp gửi `assistant_turn { conversation_id: "c1" }`, phía Rust
    /// `assert_can_read` thấy chủ hội thoại là `None` → `AUTH_FORBIDDEN`,
    /// `kind == User` nên `return Err`, hỏng cả lượt. `send()` rơi vào `catch`
    /// và không dọn id ⇒ **mọi câu hỏi sau đều hỏng y hệt, mãi mãi**.
    ///
    /// Cách chữa nhỏ nhất là đừng tạo ra trạng thái xấu: bỏ id đi ngay tại đây,
    /// **không mint khoá phiên**. Ba khẳng định dưới đo đúng ba vế đó — khoá
    /// không đổi, thẻ vẫn duyệt được, lượt sau mở sổ mới và chạy bình thường.
    it("cú dọn phiên bị từ chối thì bỏ id hội thoại đã chết mà KHÔNG mint khoá mới", async () => {
      const del = deferred<unknown>();
      const checkIn = deferred<unknown>();
      invokeWriteCommand.mockImplementation((command: string) =>
        command === "check_in" ? checkIn.promise : del.promise,
      );
      invokeCommand.mockResolvedValue([]);
      useAssistantStore.setState({
        pendingAction: sampleAction,
        pendingActionKey: SESSION_KEY,
        conversationId: "c1",
        conversations: [summary("c1", "Hỏi phòng")],
      });

      const deleting = useAssistantStore.getState().deleteConversation("c1");
      const approval = useAssistantStore.getState().approve();
      del.resolve(1);
      await deleting;

      const afterDelete = useAssistantStore.getState();
      // Id trỏ vào hàng đã chết bị bỏ đi...
      expect(afterDelete.conversationId).toBeNull();
      // ...mà khoá phiên KHÔNG đổi, nên lớp 3 và lớp 4 không vứt thẻ đang treo.
      expect(afterDelete.conversationKey).toBe(SESSION_KEY);
      expect(afterDelete.pendingAction).toBe(sampleAction);
      expect(afterDelete.pendingActionKey).toBe(SESSION_KEY);

      // Thẻ vẫn duyệt được thật: kết quả `check_in` tới được màn hình.
      checkIn.resolve({ id: "B1" });
      await approval;
      expect(useAssistantStore.getState().messages).toContainEqual(
        expect.objectContaining({ kind: "assistant", text: "Đã nhận phòng xong." }),
      );

      // Và đây là hậu quả thật đang được chữa: lượt kế tiếp mở sổ MỚI thay vì
      // gửi lên một id đã chết rồi ăn `AUTH_FORBIDDEN` mãi mãi.
      invokeCommand.mockResolvedValueOnce({
        ...turnResponse("Còn phòng 201."),
        conversation_id: "c9",
      });
      await useAssistantStore.getState().send("phòng nào trống", { route: "rooms" });

      expect(invokeCommand).toHaveBeenLastCalledWith("assistant_turn", {
        request: expect.objectContaining({ conversation_id: null }),
      });
      expect(useAssistantStore.getState().conversationId).toBe("c9");
    });

    /// Vế song sinh ở `deleteAllConversations`. Dòng riêng, ở hàm riêng, nên
    /// phải có đòn sabotage riêng — chỗ đặt một dòng thì không phải một lớp.
    it("xoá sạch bị từ chối cũng bỏ id hội thoại đã chết, khoá phiên vẫn nguyên", async () => {
      const del = deferred<unknown>();
      const checkIn = deferred<unknown>();
      invokeWriteCommand.mockImplementation((command: string) =>
        command === "check_in" ? checkIn.promise : del.promise,
      );
      invokeCommand.mockResolvedValue([]);
      useAssistantStore.setState({
        pendingAction: sampleAction,
        pendingActionKey: SESSION_KEY,
        conversationId: "c1",
        conversations: [summary("c1", "Hỏi phòng")],
      });

      const deleting = useAssistantStore.getState().deleteAllConversations();
      const approval = useAssistantStore.getState().approve();
      del.resolve(2);
      await deleting;

      const afterDelete = useAssistantStore.getState();
      expect(afterDelete.conversationId).toBeNull();
      expect(afterDelete.conversationKey).toBe(SESSION_KEY);
      expect(afterDelete.pendingActionKey).toBe(SESSION_KEY);

      checkIn.resolve({ id: "B1" });
      await approval;
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
      // `resetForLogout()` chứ không `startNewChat()`: đăng xuất là đường mint
      // duy nhất chạy được khi đang bận (bất biến sở hữu `busy`).
      useAssistantStore.getState().resetForLogout();
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
      useAssistantStore.getState().resetForLogout();
      await turn;

      const state = useAssistantStore.getState();
      expect(state.messages).toEqual([]);
      expect(state.error).toBeNull();
      expect(state.busy).toBe(false);
    });

    /// VẾ DƯƠNG của đường lỗi `send()` — đường hỏng phổ biến nhất của cả tính
    /// năng (nhà cung cấp AI hết hạn mức, mất mạng, khoá API sai), và trước test
    /// này không ai canh. Đo được: bỏ `error: text` khỏi khối `catch` mà giữ
    /// bong bóng thì cả bộ vẫn xanh, và bỏ bong bóng mà giữ `error` cũng vậy.
    /// Lý do là test duy nhất chạm khối `catch` — vế ÂM ngay bên trên — chỉ
    /// khẳng định "KHÔNG có gì rơi vào phiên mới", nên một bản chẳng đặt gì cả
    /// cũng qua được nó.
    ///
    /// Phải có CẢ HAI, và đây là quyết định thiết kế chứ không phải trùng lặp
    /// thừa: bong bóng `kind: "error"` là bản ghi ở lại trong dòng hội thoại
    /// (cuộn lên vẫn thấy lượt nào hỏng), còn `error` là viên `role="alert"`
    /// ghim tại chỗ và được trình đọc màn hình xướng lên. Hai vai trò khác nhau.
    it("send() hỏng trong CÙNG phiên thì đặt cả bong bóng lỗi lẫn viên cảnh báo", async () => {
      invokeCommand.mockRejectedValue(new Error("Nhà cung cấp AI không phản hồi"));

      await useAssistantStore.getState().send("phòng nào trống", { route: "rooms" });

      const state = useAssistantStore.getState();
      expect(state.error).toBe("Nhà cung cấp AI không phản hồi");
      expect(state.messages).toEqual([
        expect.objectContaining({ kind: "user", text: "phòng nào trống" }),
        expect.objectContaining({ kind: "error", text: "Nhà cung cấp AI không phản hồi" }),
      ]);
      expect(state.busy).toBe(false);
      // Không đổi phiên thì cũng không mint khoá mới: lượt hỏng vẫn thuộc về
      // hội thoại đang mở, hỏi lại được ngay.
      expect(state.conversationKey).toBe(SESSION_KEY);
    });

    /// I4(b) — TỰ LÀNH, lưới hứng cho mọi đường sinh ra id chết mà (a) không
    /// với tới.
    ///
    /// (a) dọn id ngay tại cú xoá bị từ chối, nhưng nó không với qua được một
    /// lượt CHƯA bay về: admin bấm *Xoá vĩnh viễn* ở Cài đặt (panel vẫn nằm bên
    /// trái) → lễ tân gõ một câu, `send()` bay đi với `conversation_id: "c1"` →
    /// lệnh xoá về, (a) đặt id về `null` → rồi `assistant_turn` mới về, và
    /// nhánh thành công gán lại `response.conversation_id ?? state.conversationId`
    /// = `"c1"`. Id chết sống lại. Chỉ có dòng ở `catch` mới bắt được ca này.
    ///
    /// Hai vế trong một test, và vế ÂM là vế dễ làm sai: dọn id ở MỌI lỗi thì
    /// mạng chập một cái là cuộc trò chuyện đang dở bị chẻ làm hai bản ghi rời.
    it("lượt bị AUTH_FORBIDDEN thì bỏ id hội thoại đã chết; lỗi thường thì KHÔNG", async () => {
      useAssistantStore.setState({ conversationId: "c1" });

      // Vế ÂM trước: mạng chập, nhà cung cấp câm — sổ vẫn còn nguyên trên đĩa,
      // hỏi lại phải là hỏi tiếp đúng sổ đó.
      invokeCommand.mockRejectedValueOnce(new Error("Nhà cung cấp AI không phản hồi"));
      await useAssistantStore.getState().send("phòng nào trống", { route: "rooms" });
      expect(useAssistantStore.getState().conversationId).toBe("c1");

      // Vế DƯƠNG. Ném qua đúng cái factory mà `invokeCommand` thật dùng
      // (`createAppErrorException`) chứ không tự bịa một object có `code`: cái
      // phải đo là tên trường khớp với thứ đường thật ném ra, không phải là
      // store biết đọc một hình dạng do chính test nghĩ ra.
      invokeCommand.mockRejectedValueOnce(
        createAppErrorException({
          code: "AUTH_FORBIDDEN",
          message: "Không mở được hội thoại này.",
          kind: "user",
          support_id: null,
        }),
      );
      await useAssistantStore.getState().send("còn phòng đôi không", { route: "rooms" });
      expect(useAssistantStore.getState().conversationId).toBeNull();

      // Hậu quả thật: lượt sau KHÔNG hỏng nữa. Không có dòng tự lành thì lượt
      // này lại gửi `"c1"` và lại ăn `AUTH_FORBIDDEN` — trợ lý chết cứng ở quầy
      // tới khi lễ tân tự đoán ra là phải bấm *Hội thoại mới*.
      invokeCommand.mockResolvedValueOnce({
        ...turnResponse("Còn phòng 201."),
        conversation_id: "c9",
      });
      await useAssistantStore.getState().send("thế phòng 201", { route: "rooms" });

      expect(invokeCommand).toHaveBeenLastCalledWith("assistant_turn", {
        request: expect.objectContaining({ conversation_id: null }),
      });
      const state = useAssistantStore.getState();
      expect(state.conversationId).toBe("c9");
      expect(state.messages).toContainEqual(
        expect.objectContaining({ kind: "assistant", text: "Còn phòng 201." }),
      );
      // Sổ chat là tiện ích: dọn id KHÔNG được kéo theo khoá phiên, kẻo thẻ
      // nhận phòng đang treo chết oan vì một lỗi ghi sổ.
      expect(state.conversationKey).toBe(SESSION_KEY);
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

    /// Viên lỗi phải chết khi thao tác MỚI thành công, không thì nó NÓI DỐI.
    ///
    /// Đường đo được: tải lịch sử hỏng → viên đỏ "Không đọc được sổ hội thoại"
    /// → bấm thử lại → danh sách hiện ra đầy đủ, mà viên đỏ cũ vẫn nằm nguyên
    /// trên đầu chính cái danh sách nó vừa tố là đọc không được. Không phải
    /// "viên đỏ nằm lì vài phút" mà là mâu thuẫn trực tiếp với thứ ngay bên dưới.
    it("tải lại được danh sách thì viên lỗi của lần hỏng trước tắt theo", async () => {
      invokeCommand.mockRejectedValueOnce(new Error("Không đọc được sổ hội thoại"));
      await useAssistantStore.getState().loadConversations();

      // Vế dương: lần hỏng CÓ đặt viên lỗi. Thiếu câu này thì một bản không bao
      // giờ đặt `error` cũng làm khẳng định bên dưới xanh.
      expect(useAssistantStore.getState().error).toContain("Không đọc được sổ hội thoại");

      invokeCommand.mockResolvedValueOnce([summary("c1", "Hỏi phòng")]);
      await useAssistantStore.getState().loadConversations();

      const state = useAssistantStore.getState();
      expect(state.conversations).toEqual([summary("c1", "Hỏi phòng")]);
      expect(state.error).toBeNull();
    });

    /// Ca duy nhất trong ba đường xoá/tải KHÔNG đi qua `startNewChat()` — tức
    /// không đi qua `error: null` của `emptySession()`. Nó dọn được viên lỗi cũ
    /// là nhờ `loadConversations()` chạy ở cuối, nên test này canh đúng sợi dây
    /// đó chứ không canh lại thứ `emptySession()` đã canh.
    it("xoá một hội thoại khác thành công cũng dọn viên lỗi cũ", async () => {
      useAssistantStore.setState({ conversationId: "c1", error: "Chỉ admin mới được thực hiện" });
      invokeWriteCommand.mockResolvedValue(1);
      invokeCommand.mockResolvedValue([summary("c1", "Hỏi phòng")]);

      await useAssistantStore.getState().deleteConversation("c2");

      const state = useAssistantStore.getState();
      // Phiên đang mở không bị đụng — đây đúng là ca không qua `startNewChat()`.
      expect(state.conversationId).toBe("c1");
      expect(state.conversationKey).toBe(SESSION_KEY);
      expect(state.error).toBeNull();
    });

    /// `openConversation` thành công dọn `error` qua `emptySession()`. Trước
    /// test này sợi dây đó chỉ được canh GIÁN TIẾP bởi test của `startNewChat()`
    /// — cùng dùng `emptySession()` — nên một bản đổi `openConversation` sang
    /// dựng state bằng tay sẽ để lại viên lỗi cũ mà cả bộ vẫn xanh.
    it("mở được hội thoại cũ thì viên lỗi của lần mở hỏng trước tắt theo", async () => {
      useAssistantStore.setState({ error: "Không đọc được hội thoại" });
      invokeCommand.mockResolvedValue([storedMessage("m1", "user", "Phòng 201 trống không?")]);

      await useAssistantStore.getState().openConversation("c1");

      const state = useAssistantStore.getState();
      expect(state.conversationId).toBe("c1");
      expect(state.error).toBeNull();
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

    /// Câu từ chối KHÔNG được đè lên lỗi của `loadConversations()`.
    ///
    /// Hai thứ hỏng cùng lúc, và chỉ một viên `role="alert"` để nói. Tin phải
    /// giữ là tin **danh sách đang hiện trên màn hình có thể đã cũ**: nó không
    /// có dấu hiệu nào khác, trong khi "phiên chưa dọn được" thì admin nhìn
    /// panel là thấy — hội thoại vẫn còn nguyên đó.
    it("nạp lại danh sách hỏng thì câu từ chối không đè mất tin danh sách đã cũ", async () => {
      const del = deferred<unknown>();
      const checkIn = deferred<unknown>();
      invokeWriteCommand.mockImplementation((command: string) =>
        command === "check_in" ? checkIn.promise : del.promise,
      );
      invokeCommand.mockRejectedValue(new Error("Không đọc được sổ hội thoại"));
      useAssistantStore.setState({
        pendingAction: sampleAction,
        pendingActionKey: SESSION_KEY,
        conversationId: "c1",
        conversations: [summary("c1", "Hỏi phòng")],
      });

      const deleting = useAssistantStore.getState().deleteConversation("c1");
      const approval = useAssistantStore.getState().approve();
      del.resolve(1);
      await deleting;

      const state = useAssistantStore.getState();
      expect(state.error).toContain("Không đọc được sổ hội thoại");
      // Và danh sách đúng là đã cũ: hàng "c1" đã bị xoá khỏi đĩa mà vẫn nằm đây.
      expect(state.conversations).toEqual([summary("c1", "Hỏi phòng")]);

      checkIn.resolve({ id: "B1" });
      await approval;
    });

    /// Vế song sinh ở `deleteAllConversations` — dòng riêng, sabotage riêng.
    it("xoá sạch rồi nạp lại hỏng thì cũng không đè mất tin danh sách đã cũ", async () => {
      const del = deferred<unknown>();
      const checkIn = deferred<unknown>();
      invokeWriteCommand.mockImplementation((command: string) =>
        command === "check_in" ? checkIn.promise : del.promise,
      );
      invokeCommand.mockRejectedValue(new Error("Không đọc được sổ hội thoại"));
      useAssistantStore.setState({
        pendingAction: sampleAction,
        pendingActionKey: SESSION_KEY,
        conversationId: "c1",
        conversations: [summary("c1", "Hỏi phòng")],
      });

      const deleting = useAssistantStore.getState().deleteAllConversations();
      const approval = useAssistantStore.getState().approve();
      del.resolve(2);
      await deleting;

      expect(useAssistantStore.getState().error).toContain("Không đọc được sổ hội thoại");

      checkIn.resolve({ id: "B1" });
      await approval;
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

    /// Ca lệnh KHÔNG BAO GIỜ bay về: mạng treo, nhà cung cấp AI câm.
    ///
    /// Bình thường `busy` tự lành mà không cần ai dọn — cả hai nhánh race guard
    /// trong `send()` đều `set({ busy: false })` trước khi `return`. Nhưng cả
    /// hai nhánh ấy chỉ chạy khi promise **giải quyết**. Không có `busy: false`
    /// trong `resetForLogout()` thì `busy` kẹt `true` sang phiên của người kế
    /// tiếp, và khung soạn của họ tắt cho tới khi khởi động lại app: `send()`
    /// tự chặn ở `if (!trimmed || get().busy) return;`, nên đây là hàng rào ở
    /// tầng store chứ không chỉ là một nút xám.
    ///
    /// Đăng xuất là đúng thời điểm dọn cứng: nó không được phụ thuộc vào một
    /// promise đang bay mà không ai biết bao giờ về.
    it("đăng xuất giữa lúc lệnh treo thì người kế tiếp vẫn gửi được tin", async () => {
      // Không `mockResolvedValue`, không `mockRejectedValue`: promise này
      // không bao giờ đổi trạng thái, đúng hình dạng của mạng treo.
      invokeCommand.mockReturnValue(new Promise(() => {}));

      void useAssistantStore.getState().send("phòng nào trống", { route: "rooms" });
      // Vế dương trước: thiếu câu này thì một bản `send()` không bao giờ bật
      // `busy` cũng làm khẳng định cuối cùng xanh, mà nó chẳng đo được gì.
      expect(useAssistantStore.getState().busy).toBe(true);

      await useAuthStore.getState().logout();

      expect(useAssistantStore.getState().busy).toBe(false);

      // Và đo tới tận hậu quả: người kế tiếp gõ được thật, không chỉ là một cờ
      // boolean đúng. `send()` chặn ngay ở đầu khi `busy`, nên nếu cờ còn kẹt
      // thì lệnh thứ hai này không bao giờ được bắn đi.
      invokeCommand.mockResolvedValue({
        reply: "ok",
        proposed_action: null,
        history: [],
        conversation_id: null,
      });
      await useAssistantStore.getState().send("của người mới", { route: "rooms" });

      expect(invokeCommand).toHaveBeenLastCalledWith(
        "assistant_turn",
        expect.objectContaining({
          request: expect.objectContaining({ message: "của người mới" }),
        }),
      );
    });
  });
});
