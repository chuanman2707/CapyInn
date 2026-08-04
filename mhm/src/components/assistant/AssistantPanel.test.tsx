import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// useHotelStore thật sẽ nạp dữ liệu qua Tauri lúc render. Panel chỉ cần ba
// trường, nên mock nguyên store cho test đứng độc lập.
vi.mock("@/stores/useHotelStore", () => ({
  useHotelStore: () => ({
    activeTab: "rooms",
    roomDetail: null,
    roomChangeBookingId: null,
  }),
}));

import { useAssistantStore } from "@/stores/useAssistantStore";
import { AssistantPanel, buildScreenContext } from "./AssistantPanel";

describe("AssistantPanel", () => {
  beforeEach(() => {
    useAssistantStore.setState({
      open: true,
      messages: [],
      history: [],
      pendingAction: null,
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

  it("bấm gửi thì gọi send với đúng nội dung gõ và ngữ cảnh màn hình hiện tại", async () => {
    // Spy trên store thật (không mock nguyên module) để khẳng định hành vi
    // thật của panel: form submit phải gọi đúng send(message, context).
    const sendSpy = vi.spyOn(useAssistantStore.getState(), "send").mockResolvedValue(undefined);

    render(<AssistantPanel />);

    // Đúng một ô nhập và một nút (chưa có thẻ xác nhận nào đang chờ) nên
    // không cần tên phân biệt.
    await userEvent.type(screen.getByRole("textbox"), "Phòng 201 còn trống không?");
    await userEvent.click(screen.getByRole("button"));

    expect(sendSpy).toHaveBeenCalledWith("Phòng 201 còn trống không?", {
      route: "rooms",
      selectedRoomId: undefined,
      selectedRoomNumber: undefined,
      selectedBookingId: undefined,
    });
  });
});
