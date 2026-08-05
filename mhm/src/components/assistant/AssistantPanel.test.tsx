import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// useHotelStore thật sẽ nạp dữ liệu qua Tauri lúc render. Panel chỉ cần ba
// trường, nên mock nguyên store cho test đứng độc lập. hotelState khai qua
// vi.hoisted vì factory của vi.mock bị hoist lên đầu file — tham chiếu một
// biến khai ở scope module thường sẽ vỡ ở đó. Để mutable (thay vì literal cố
// định) để test "chọn phòng" bên dưới tự đặt roomDetail trước khi render.
//
// Panel gọi hook này với một selector (mỗi field một lần — xem AssistantPanel.tsx),
// nên mock phải áp selector lên state giả lập chứ không trả nguyên object bất
// kể tham số: trả nguyên object sẽ khiến useHotelStore(s => s.activeTab) nhận
// cả object thay vì chuỗi "rooms".
const { hotelState } = vi.hoisted(() => ({
  hotelState: {
    activeTab: "rooms",
    roomDetail: null as { room?: { id?: string; name?: string } } | null,
    roomChangeBookingId: null as string | null,
  },
}));

vi.mock("@/stores/useHotelStore", () => ({
  useHotelStore: (selector: (state: typeof hotelState) => unknown) => selector(hotelState),
}));

import { useAssistantStore } from "@/stores/useAssistantStore";
import { useAuthStore } from "@/stores/useAuthStore";
import type { AssistantConversationSummary, ProposedAction } from "@/types/assistant";
import { SUGGESTIONS } from "./AssistantEmptyState";
import { AssistantPanel, buildScreenContext } from "./AssistantPanel";

const HISTORY_NOTICE = "Đây là bản ghi. Hỏi tiếp thì trợ lý không nhớ thẻ đã đề xuất trước đó.";

function makeAction(): ProposedAction {
  return {
    kind: "check_in",
    payload: { room_id: "R1", guests: [{ full_name: "Nguyễn Văn Nam" }], nights: 1 },
    display: { room_id: "Phòng 201" },
    preview: {},
    warnings: [],
    built_at_ms: Date.now(),
  };
}

function makeSummary(
  overrides: Partial<AssistantConversationSummary> = {},
): AssistantConversationSummary {
  return {
    id: "c1",
    user_id: "u1",
    user_name: "Lễ tân A",
    title: "Phòng 201 còn trống không?",
    updated_at: "2026-08-05T09:30:00+07:00",
    ...overrides,
  };
}

function signIn(role: "admin" | "receptionist") {
  useAuthStore.setState({
    user: { id: "u1", name: "Lễ tân A", role, active: true, created_at: "2026-08-01T00:00:00+07:00" },
    isAuthenticated: true,
  });
}

describe("AssistantPanel", () => {
  beforeEach(() => {
    // Trả hotelState về mặc định trước mỗi test — test "chọn phòng" bên dưới
    // sửa nó và không được rò rỉ sang các test khác chạy sau.
    hotelState.activeTab = "rooms";
    hotelState.roomDetail = null;
    hotelState.roomChangeBookingId = null;

    // Mặc định là lễ tân: quyền admin phải được từng test bật lên rõ ràng.
    useAuthStore.setState({ user: null, isAuthenticated: false });

    // Trả store về đúng bản gốc TRƯỚC khi đặt trạng thái của test.
    //
    // `vi.spyOn` trên state zustand rò sang test kế tiếp: `setState` sao chép
    // mọi thuộc tính sang một object MỚI nên con mock đi theo, trong khi
    // `vi.restoreAllMocks()` chỉ khôi phục trên object CŨ đã bị vứt. Lần
    // `vi.spyOn` sau nhận lại **chính con mock cũ kèm nguyên sổ gọi**, vì
    // `vitest.config.ts` không bật `clearMocks`. Đây là bẫy vòng duyệt Task 7
    // ghi lại, và nó đã cắn thật ở đây: ba test lớp 1 đỏ vì đếm cú gọi của test
    // TRƯỚC, không phải của chính nó. Còn nguy hơn chiều ngược lại — phần cài
    // đặt giả (`mockImplementation(() => {})`) cũng ở lại, nên một test sau
    // tưởng đang gọi hàm thật của store thì thật ra gọi một hàm rỗng.
    useAssistantStore.setState(useAssistantStore.getInitialState(), true);

    useAssistantStore.setState({
      open: true,
      messages: [],
      history: [],
      pendingAction: null,
      pendingActionKey: null,
      conversationKey: "phien-dang-mo",
      conversationId: null,
      conversations: [],
      historyNotice: null,
      busy: false,
      error: null,
      settings: {
        config: { preset: "deep_seek", base_url: "https://x/y", model: "deepseek-chat" },
        has_api_key: true,
        cloud_data_opt_in: true,
        gate: { ready: true, missing: [] },
      },
    });
  });

  afterEach(() => {
    // Test "bấm gửi..." spy trên chính store thật (xem dưới); trả action gốc
    // lại để không rò rỉ sang test hoặc file khác.
    vi.restoreAllMocks();
  });

  it("không hiện gì khi trợ lý chưa cấu hình", () => {
    useAssistantStore.setState({
      settings: {
        config: { preset: "deep_seek", base_url: "", model: "" },
        has_api_key: false,
        cloud_data_opt_in: false,
        gate: { ready: false, missing: ["api_key", "cloud_data_opt_in"] },
      },
    });

    const { container } = render(<AssistantPanel />);

    expect(container).toBeEmptyDOMElement();
  });

  it("không hiện gì khi panel đang đóng, dù trợ lý đã cấu hình xong", () => {
    // Điều kiện ẩn có hai vế (!gate.ready || !open); test trên chỉ phủ vế
    // đầu. Vế "open" tắt độc lập với cổng để không bỏ sót nhánh còn lại.
    useAssistantStore.setState({ open: false });

    const { container } = render(<AssistantPanel />);

    expect(container).toBeEmptyDOMElement();
  });

  it("hiện chip ngữ cảnh của màn hình đang mở", () => {
    render(<AssistantPanel />);

    expect(screen.getByText(/đang xem/i)).toBeInTheDocument();
  });

  it("ngữ cảnh lấy phòng đang chọn từ hotel store", () => {
    const context = buildScreenContext({
      activeTab: "rooms",
      roomDetail: { room: { id: "R1", name: "Phòng 201" } },
      roomChangeBookingId: null,
    } as never);

    expect(context.route).toBe("rooms");
    expect(context.selectedRoomId).toBe("R1");
    expect(context.selectedRoomNumber).toBe("Phòng 201");
  });

  it("nút gửi có tên truy cập cho công nghệ hỗ trợ", () => {
    // Nút gửi chỉ có icon (Send), không có chữ. Đây là cách duy nhất để gửi
    // tin nhắn; thiếu aria-label thì trình đọc màn hình không biết nút này
    // làm gì. Một khi thẻ xác nhận xuất hiện, nó cũng không còn là nút duy
    // nhất trong panel nên getByRole("button") trần sẽ mơ hồ.
    render(<AssistantPanel />);

    expect(screen.getByRole("button", { name: "Gửi tin nhắn" })).toBeInTheDocument();
  });

  it("bấm gửi thì gọi send với đúng nội dung gõ và ngữ cảnh màn hình hiện tại", async () => {
    // Spy trên store thật (không mock nguyên module) để khẳng định hành vi
    // thật của panel: form submit phải gọi đúng send(message, context).
    const sendSpy = vi.spyOn(useAssistantStore.getState(), "send").mockResolvedValue(undefined);

    render(<AssistantPanel />);

    await userEvent.type(screen.getByRole("textbox"), "Phòng 201 còn trống không?");
    await userEvent.click(screen.getByRole("button", { name: "Gửi tin nhắn" }));

    expect(sendSpy).toHaveBeenCalledWith("Phòng 201 còn trống không?", {
      route: "rooms",
      selectedRoomId: undefined,
      selectedRoomNumber: undefined,
      selectedBookingId: undefined,
    });
  });

  it("bấm gửi khi đang chọn một phòng thì gọi send kèm đúng phòng đó trong ngữ cảnh", async () => {
    // Test "bấm gửi..." ở trên luôn có roomDetail: null nên mọi field của
    // context đều undefined; phần ánh xạ khi CÓ phòng đang chọn trước giờ chỉ
    // được phủ cách ly qua buildScreenContext, chưa từng qua một lần submit
    // thật. Đây là điểm tích hợp thật sự quan trọng: AI có được cho biết
    // đúng phòng lễ tân đang xem hay không.
    hotelState.roomDetail = { room: { id: "R1", name: "Phòng 201" } };
    const sendSpy = vi.spyOn(useAssistantStore.getState(), "send").mockResolvedValue(undefined);

    render(<AssistantPanel />);

    await userEvent.type(screen.getByRole("textbox"), "Phòng này còn trống không?");
    await userEvent.click(screen.getByRole("button", { name: "Gửi tin nhắn" }));

    expect(sendSpy).toHaveBeenCalledWith("Phòng này còn trống không?", {
      route: "rooms",
      selectedRoomId: "R1",
      selectedRoomNumber: "Phòng 201",
      selectedBookingId: undefined,
    });
  });

  it("gửi bằng đường gõ-rồi-Enter thì dọn sạch ô nhập", async () => {
    // Đường gợi ý (bấm chip ở màn hình trống) có canh chuyện dọn ô; đường
    // gõ-rồi-gửi — đường lễ tân dùng 99% thời gian — thì không: bỏ hẳn
    // `setDraft("")` khỏi `onSubmitDraft` mà cả 31 test cũ vẫn xanh.
    //
    // Chữ ở lại trong ô sau khi gửi thì cú Enter kế tiếp bắn lại y nguyên câu
    // vừa gửi: một lượt `assistant_turn` thừa, và với câu dựng thẻ thì là một
    // thẻ nhận phòng THỨ HAI.
    const sendSpy = vi.spyOn(useAssistantStore.getState(), "send").mockResolvedValue(undefined);

    render(<AssistantPanel />);
    const box = screen.getByRole("textbox");

    await userEvent.type(box, "Nhận phòng giúp tôi");
    fireEvent.keyDown(box, { key: "Enter", isComposing: false });

    // Gửi đúng câu đã gõ — chặn luôn bản "dọn ô trước rồi mới đọc `draft`",
    // vốn cũng làm hai khẳng định dưới xanh nhưng gửi lên một chuỗi rỗng.
    expect(sendSpy).toHaveBeenCalledWith("Nhận phòng giúp tôi", {
      route: "rooms",
      selectedRoomId: undefined,
      selectedRoomNumber: undefined,
      selectedBookingId: undefined,
    });
    expect(box).toHaveValue("");
    expect(screen.getByRole("button", { name: "Gửi tin nhắn" })).toBeDisabled();

    // Và cú Enter kế tiếp không được bắn lại câu vừa gửi.
    fireEvent.keyDown(box, { key: "Enter", isComposing: false });
    expect(sendSpy).toHaveBeenCalledTimes(1);
  });

  it("bấm gợi ý ở màn hình trống thì GỬI luôn câu đó, không chỉ điền vào ô nhập", async () => {
    // Đây là điểm nối thật của màn hình trống: `AssistantEmptyState` chỉ biết
    // gọi `onPick`, còn việc cú bấm ấy có thành một lượt chat hay chỉ nhét chữ
    // vào ô nhập rồi đứng im thì chỉ đo được ở đây. Khẳng định thứ hai (ô nhập
    // vẫn rỗng) chặn đúng bản cài đặt "điền hộ rồi bắt bấm gửi lần nữa".
    const sendSpy = vi.spyOn(useAssistantStore.getState(), "send").mockResolvedValue(undefined);

    render(<AssistantPanel />);

    await userEvent.click(screen.getByRole("button", { name: SUGGESTIONS[0] }));

    expect(sendSpy).toHaveBeenCalledWith(SUGGESTIONS[0], {
      route: "rooms",
      selectedRoomId: undefined,
      selectedRoomNumber: undefined,
      selectedBookingId: undefined,
    });
    expect(screen.getByRole("textbox")).toHaveValue("");
  });

  it("vẽ thẻ khi thẻ được dựng ở chính hội thoại đang mở", () => {
    // Vế dương của lớp 3. Thiếu nó thì một bản cài đặt không vẽ thẻ bao giờ
    // cũng làm test âm bên dưới xanh.
    useAssistantStore.setState({
      pendingAction: makeAction(),
      pendingActionKey: "phien-dang-mo",
      conversationKey: "phien-dang-mo",
    });

    render(<AssistantPanel />);

    expect(screen.getByText("Xác nhận nhận phòng")).toBeInTheDocument();
  });

  it("KHÔNG vẽ thẻ dựng ở hội thoại khác (lớp 3)", () => {
    // Lớp 4 trong `approve()` đã chặn được TIỀN, nhưng nó chỉ chạy sau khi lễ
    // tân bấm *Đồng ý* trên một cái thẻ đáng lẽ không có ở đó — thẻ biến mất,
    // không một lời giải thích. Lớp 3 tồn tại để nó đừng xuất hiện từ đầu.
    useAssistantStore.setState({
      pendingAction: makeAction(),
      pendingActionKey: "phien-khac",
      conversationKey: "phien-dang-mo",
    });

    render(<AssistantPanel />);

    expect(screen.queryByText("Xác nhận nhận phòng")).not.toBeInTheDocument();
  });

  it("hiện dòng nhắc khi mở lại một hội thoại cũ", () => {
    useAssistantStore.setState({
      messages: [{ id: "m1", kind: "user", text: "Phòng 201 còn trống không?" }],
      historyNotice: HISTORY_NOTICE,
    });

    render(<AssistantPanel />);

    expect(screen.getByRole("status")).toHaveTextContent(HISTORY_NOTICE);
  });

  it("không nhắc gì khi historyNotice rỗng", () => {
    // Cặp với test trên: nhắc thường trực thì thành nhiễu, mà nhiễu thường
    // trực thì người ta thôi đọc. Một bản cài đặt in cứng câu nhắc sẽ đỏ ở đây.
    //
    // Khẳng định trên PHẦN TỬ chứ không trên chữ. Bản bọc-vô-điều-kiện —
    // `<p className="…bg-amber-50 px-3 py-2">{historyNotice}</p>`, chỉ nội dung
    // mới có điều kiện — vẽ ra một viên nền vàng RỖNG thường trực. jsdom không
    // thấy nền nên `queryByText` vẫn xanh với nó; `queryByRole` thì đỏ.
    useAssistantStore.setState({
      messages: [{ id: "m1", kind: "user", text: "Phòng 201 còn trống không?" }],
      historyNotice: null,
    });

    render(<AssistantPanel />);

    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("header nói Hội thoại mới khi chưa mở hội thoại nào", () => {
    render(<AssistantPanel />);

    expect(screen.getByRole("heading", { name: "Hội thoại mới" })).toBeInTheDocument();
  });

  it("header hiện tên của hội thoại đang mở", () => {
    // Cặp với test trên: một bản in cứng "Hội thoại mới" làm test kia xanh mãi.
    useAssistantStore.setState({
      conversations: [makeSummary({ id: "c1", title: "Phòng 201 còn trống không?" })],
      conversationId: "c1",
    });

    render(<AssistantPanel />);

    expect(
      screen.getByRole("heading", { name: "Phòng 201 còn trống không?" }),
    ).toBeInTheDocument();
  });

  it("nút thu panel gọi togglePanel", async () => {
    const toggleSpy = vi
      .spyOn(useAssistantStore.getState(), "togglePanel")
      .mockImplementation(() => {});

    render(<AssistantPanel />);
    await userEvent.click(screen.getByRole("button", { name: "Thu trợ lý" }));

    expect(toggleSpy).toHaveBeenCalledTimes(1);
  });

  it("bấm lịch sử thì đổi nội dung panel sang danh sách và tải danh sách về", async () => {
    const loadSpy = vi
      .spyOn(useAssistantStore.getState(), "loadConversations")
      .mockResolvedValue(undefined);
    useAssistantStore.setState({ conversations: [makeSummary()] });

    render(<AssistantPanel />);

    // Trước khi bấm KHÔNG được có danh sách nào: thiếu câu này thì một bản vẽ
    // danh sách thường trực — tức không có máy chuyển view nào cả — vẫn xanh.
    expect(screen.queryByRole("listitem")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Lịch sử" }));

    expect(loadSpy).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("listitem")).toHaveTextContent("Phòng 201 còn trống không?");
    // Danh sách thay chỗ khung chat, không chen bên cạnh nó.
    expect(screen.queryByPlaceholderText("Hỏi hoặc ra việc…")).not.toBeInTheDocument();
  });

  it("nút quay lại đưa panel về khung chat", async () => {
    vi.spyOn(useAssistantStore.getState(), "loadConversations").mockResolvedValue(undefined);
    useAssistantStore.setState({ conversations: [makeSummary()] });

    render(<AssistantPanel />);
    await userEvent.click(screen.getByRole("button", { name: "Lịch sử" }));
    await userEvent.click(screen.getByRole("button", { name: "Quay lại hội thoại" }));

    expect(screen.queryByRole("listitem")).not.toBeInTheDocument();
    expect(screen.getByPlaceholderText("Hỏi hoặc ra việc…")).toBeInTheDocument();
  });

  it("đang chờ trả lời thì khoá nút hội thoại mới và nút lịch sử", () => {
    // Spec: chuyển hội thoại giữa lúc câu trả lời đang bay về đẻ ra tranh chấp
    // mà không đổi lại được gì.
    useAssistantStore.setState({ busy: true });

    render(<AssistantPanel />);

    expect(screen.getByRole("button", { name: "Hội thoại mới" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Lịch sử" })).toBeDisabled();
  });

  // ── Lớp 1 của bốn lớp bảo vệ `pendingAction` ────────────────────────────
  //
  // Lớp 1 chặn Ở NGUỒN: đang treo thẻ nhận phòng mà bấm *hội thoại mới* hoặc mở
  // một hội thoại từ lịch sử thì phải hỏi trước. Cả HAI đường, vì cả hai đều
  // gọi `emptySession()` (lớp 2) và dọn sạch thẻ không một lời nào.
  //
  // Lớp 1 là lớp *lịch sự*, lớp 4 (`approve()`) là lớp *giữ tiền* — có lớp 1
  // rồi vẫn giữ nguyên lớp 4.

  it("không treo thẻ thì bấm hội thoại mới đi thẳng, không hỏi han gì", async () => {
    // Vế dương: thiếu nó thì một bản HỎI MỌI LÚC — kể cả khi chẳng có gì để
    // mất — cũng làm các test lớp 1 bên dưới xanh.
    const newChatSpy = vi
      .spyOn(useAssistantStore.getState(), "startNewChat")
      .mockImplementation(() => {});

    render(<AssistantPanel />);
    await userEvent.click(screen.getByRole("button", { name: "Hội thoại mới" }));

    expect(newChatSpy).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });

  it("đang treo thẻ mà bấm hội thoại mới thì HỎI trước, chưa dọn gì (lớp 1)", async () => {
    const newChatSpy = vi
      .spyOn(useAssistantStore.getState(), "startNewChat")
      .mockImplementation(() => {});
    useAssistantStore.setState({
      pendingAction: makeAction(),
      pendingActionKey: "phien-dang-mo",
      conversationKey: "phien-dang-mo",
    });

    render(<AssistantPanel />);
    await userEvent.click(screen.getByRole("button", { name: "Hội thoại mới" }));

    expect(newChatSpy).not.toHaveBeenCalled();
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();
    // Và thẻ vẫn còn nguyên trên màn hình trong lúc hộp hỏi đang mở: hỏi xong
    // mới dọn, không phải dọn xong mới hỏi.
    expect(screen.getByText("Xác nhận nhận phòng")).toBeInTheDocument();
  });

  it("đồng ý bỏ thẻ rồi thì mới mở hội thoại mới", async () => {
    const newChatSpy = vi
      .spyOn(useAssistantStore.getState(), "startNewChat")
      .mockImplementation(() => {});
    useAssistantStore.setState({
      pendingAction: makeAction(),
      pendingActionKey: "phien-dang-mo",
      conversationKey: "phien-dang-mo",
    });

    render(<AssistantPanel />);
    await userEvent.click(screen.getByRole("button", { name: "Hội thoại mới" }));
    await userEvent.click(screen.getByRole("button", { name: "Bỏ thẻ và đi tiếp" }));

    expect(newChatSpy).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  });

  it("bấm Ở lại thì giữ nguyên thẻ, không mở hội thoại mới", async () => {
    const newChatSpy = vi
      .spyOn(useAssistantStore.getState(), "startNewChat")
      .mockImplementation(() => {});
    useAssistantStore.setState({
      pendingAction: makeAction(),
      pendingActionKey: "phien-dang-mo",
      conversationKey: "phien-dang-mo",
    });

    render(<AssistantPanel />);
    await userEvent.click(screen.getByRole("button", { name: "Hội thoại mới" }));
    await userEvent.click(screen.getByRole("button", { name: "Ở lại" }));

    expect(newChatSpy).not.toHaveBeenCalled();
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    expect(screen.getByText("Xác nhận nhận phòng")).toBeInTheDocument();
  });

  it("không treo thẻ thì mở hội thoại từ lịch sử đi thẳng", async () => {
    vi.spyOn(useAssistantStore.getState(), "loadConversations").mockResolvedValue(undefined);
    const openSpy = vi
      .spyOn(useAssistantStore.getState(), "openConversation")
      .mockResolvedValue(undefined);
    useAssistantStore.setState({ conversations: [makeSummary({ id: "c1" })] });

    render(<AssistantPanel />);
    await userEvent.click(screen.getByRole("button", { name: "Lịch sử" }));
    await userEvent.click(screen.getByRole("button", { name: /^Phòng 201 còn trống không/ }));

    expect(openSpy).toHaveBeenCalledWith("c1");
    expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
    // Mở xong thì về khung chat, không đứng lại ở danh sách.
    expect(screen.queryByRole("listitem")).not.toBeInTheDocument();
  });

  it("đang treo thẻ mà mở hội thoại từ lịch sử thì HỎI trước (lớp 1, đường thứ hai)", async () => {
    // Đường thứ hai của lớp 1. Một bản chỉ canh nút *hội thoại mới* sẽ xanh ở
    // ba test trên và đỏ ở đây — và ngoài đời thì lễ tân mất thẻ y như cũ, chỉ
    // là mất bằng cửa khác.
    vi.spyOn(useAssistantStore.getState(), "loadConversations").mockResolvedValue(undefined);
    const openSpy = vi
      .spyOn(useAssistantStore.getState(), "openConversation")
      .mockResolvedValue(undefined);
    useAssistantStore.setState({
      conversations: [makeSummary({ id: "c1" })],
      pendingAction: makeAction(),
      pendingActionKey: "phien-dang-mo",
      conversationKey: "phien-dang-mo",
    });

    render(<AssistantPanel />);
    await userEvent.click(screen.getByRole("button", { name: "Lịch sử" }));
    await userEvent.click(screen.getByRole("button", { name: /^Phòng 201 còn trống không/ }));

    expect(openSpy).not.toHaveBeenCalled();
    expect(screen.getByRole("alertdialog")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Bỏ thẻ và đi tiếp" }));

    expect(openSpy).toHaveBeenCalledWith("c1");
  });

  it("bấm Ở lại thì không mở hội thoại từ lịch sử", async () => {
    vi.spyOn(useAssistantStore.getState(), "loadConversations").mockResolvedValue(undefined);
    const openSpy = vi
      .spyOn(useAssistantStore.getState(), "openConversation")
      .mockResolvedValue(undefined);
    useAssistantStore.setState({
      conversations: [makeSummary({ id: "c1" })],
      pendingAction: makeAction(),
      pendingActionKey: "phien-dang-mo",
      conversationKey: "phien-dang-mo",
    });

    render(<AssistantPanel />);
    await userEvent.click(screen.getByRole("button", { name: "Lịch sử" }));
    await userEvent.click(screen.getByRole("button", { name: /^Phòng 201 còn trống không/ }));
    await userEvent.click(screen.getByRole("button", { name: "Ở lại" }));

    expect(openSpy).not.toHaveBeenCalled();
    // Ở lại thì ở lại đúng chỗ đang đứng — danh sách vẫn còn đó.
    expect(screen.getByRole("listitem")).toBeInTheDocument();
  });

  it("lễ tân không thấy nút xoá nào trong danh sách lịch sử", async () => {
    signIn("receptionist");
    vi.spyOn(useAssistantStore.getState(), "loadConversations").mockResolvedValue(undefined);
    useAssistantStore.setState({ conversations: [makeSummary()] });

    render(<AssistantPanel />);
    await userEvent.click(screen.getByRole("button", { name: "Lịch sử" }));

    expect(screen.getByRole("listitem")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /xoá/i })).not.toBeInTheDocument();
  });

  it("admin xoá một hội thoại thì gọi đúng lệnh xoá của store", async () => {
    // Đo cả sợi dây `isAdmin` chạy từ store xác thực xuống danh sách: cặp với
    // test trên, một bản in cứng `isAdmin` bằng hằng số sẽ đỏ ở một trong hai.
    signIn("admin");
    vi.spyOn(useAssistantStore.getState(), "loadConversations").mockResolvedValue(undefined);
    const deleteSpy = vi
      .spyOn(useAssistantStore.getState(), "deleteConversation")
      .mockResolvedValue(undefined);
    useAssistantStore.setState({ conversations: [makeSummary({ id: "c1" })] });

    render(<AssistantPanel />);
    await userEvent.click(screen.getByRole("button", { name: "Lịch sử" }));
    await userEvent.click(
      screen.getByRole("button", { name: "Xoá hội thoại Phòng 201 còn trống không?" }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Xoá hội thoại này" }));

    expect(deleteSpy).toHaveBeenCalledWith("c1");
  });

  it("admin gõ XOÁ HẾT rồi mới gọi được lệnh xoá sạch", async () => {
    signIn("admin");
    vi.spyOn(useAssistantStore.getState(), "loadConversations").mockResolvedValue(undefined);
    const deleteAllSpy = vi
      .spyOn(useAssistantStore.getState(), "deleteAllConversations")
      .mockResolvedValue(undefined);
    useAssistantStore.setState({ conversations: [makeSummary()] });

    render(<AssistantPanel />);
    await userEvent.click(screen.getByRole("button", { name: "Lịch sử" }));
    await userEvent.click(screen.getByRole("button", { name: "Xoá tất cả hội thoại" }));
    await userEvent.type(
      screen.getByRole("textbox", { name: "Gõ XOÁ HẾT để xác nhận" }),
      "XOÁ HẾT",
    );
    await userEvent.click(screen.getByRole("button", { name: "Xoá vĩnh viễn" }));

    expect(deleteAllSpy).toHaveBeenCalledTimes(1);
  });
});
