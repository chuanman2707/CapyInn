import { render, screen } from "@testing-library/react";
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
import type { ProposedAction } from "@/types/assistant";
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

describe("AssistantPanel", () => {
  beforeEach(() => {
    // Trả hotelState về mặc định trước mỗi test — test "chọn phòng" bên dưới
    // sửa nó và không được rò rỉ sang các test khác chạy sau.
    hotelState.activeTab = "rooms";
    hotelState.roomDetail = null;
    hotelState.roomChangeBookingId = null;

    useAssistantStore.setState({
      open: true,
      messages: [],
      history: [],
      pendingAction: null,
      pendingActionKey: null,
      conversationKey: "phien-dang-mo",
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

    expect(screen.getByText(HISTORY_NOTICE)).toBeInTheDocument();
  });

  it("không nhắc gì khi historyNotice rỗng", () => {
    // Cặp với test trên: nhắc thường trực thì thành nhiễu, mà nhiễu thường
    // trực thì người ta thôi đọc. Một bản cài đặt in cứng câu nhắc sẽ đỏ ở đây.
    useAssistantStore.setState({
      messages: [{ id: "m1", kind: "user", text: "Phòng 201 còn trống không?" }],
      historyNotice: null,
    });

    render(<AssistantPanel />);

    expect(screen.queryByText(HISTORY_NOTICE)).not.toBeInTheDocument();
  });
});
