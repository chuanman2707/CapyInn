import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AssistantSettings } from "@/types/assistant";

// Backend giả có trạng thái thật: lệnh ghi đổi `backend`, lệnh đọc sau đó thấy
// giá trị mới. Mock trả cứng một giá trị sẽ không phân biệt được "màn hình cài
// đặt có bơm lại store hay không" — đúng thứ file này tồn tại để chứng minh.
const { state } = vi.hoisted(() => ({
  state: {
    backend: null as AssistantSettings | null,
    hotel: {
      activeTab: "settings",
      roomDetail: null as { room?: { id?: string; name?: string } } | null,
      roomChangeBookingId: null as string | null,
    },
  },
}));

const invokeCommand = vi.fn();
const invokeWriteCommand = vi.fn();
vi.mock("@/lib/invokeCommand", () => ({
  invokeCommand: (...args: unknown[]) => invokeCommand(...args),
  invokeWriteCommand: (...args: unknown[]) => invokeWriteCommand(...args),
}));

// Panel đọc ba field của useHotelStore qua selector; store thật sẽ gọi Tauri
// lúc render nên mock lại, cùng cách AssistantPanel.test.tsx đang làm.
vi.mock("@/stores/useHotelStore", () => ({
  useHotelStore: (selector: (value: typeof state.hotel) => unknown) => selector(state.hotel),
}));

import { AssistantPanel } from "@/components/assistant/AssistantPanel";
import { useAssistantStore } from "@/stores/useAssistantStore";
import { AssistantSection } from "./AssistantSection";

const notReady: AssistantSettings = {
  config: {
    preset: "deep_seek",
    base_url: "https://api.deepseek.com/v1/chat/completions",
    model: "deepseek-chat",
  },
  has_api_key: true,
  cloud_data_opt_in: false,
  gate: { ready: false, missing: ["cloud_data_opt_in"] },
};

const ready: AssistantSettings = {
  ...notReady,
  cloud_data_opt_in: true,
  gate: { ready: true, missing: [] },
};

/// Nút gửi là dấu hiệu nhận biết panel: tiêu đề "Trợ lý quầy" xuất hiện ở cả
/// section cài đặt lẫn header panel nên không phân biệt được hai bên.
const panelMarker = { role: "button" as const, name: "Gửi tin nhắn" };

describe("Cài đặt trợ lý bơm lại store cho panel đang mở", () => {
  beforeEach(() => {
    invokeCommand.mockReset();
    invokeWriteCommand.mockReset();

    invokeCommand.mockImplementation(async (command: string) => {
      if (command === "get_assistant_settings") return state.backend;
      throw new Error(`Lệnh đọc ngoài dự kiến: ${command}`);
    });

    // Panel đã mở sẵn: đây là tình huống thật của chủ khách sạn — shell dựng
    // panel một lần lúc khởi động (MainShell gọi refreshSettings đúng một lần
    // lúc mount), rồi mới sang tab Cài đặt cấu hình trợ lý.
    useAssistantStore.setState({
      open: true,
      messages: [],
      history: [],
      pendingAction: null,
      busy: false,
      error: null,
      settings: null,
    });
  });

  it("bật công tắc dữ liệu cloud thì panel hiện ra ngay, không cần khởi động lại app", async () => {
    state.backend = notReady;
    useAssistantStore.setState({ settings: notReady });
    invokeWriteCommand.mockImplementation(async (command: string) => {
      if (command !== "set_assistant_cloud_opt_in") throw new Error(`Lệnh ghi ngoài dự kiến: ${command}`);
      state.backend = ready;
      return state.backend;
    });

    render(
      <>
        <AssistantSection />
        <AssistantPanel />
      </>,
    );

    const checkbox = await screen.findByRole("checkbox", { name: /đồng ý gửi dữ liệu/i });
    expect(screen.queryByRole(panelMarker.role, { name: panelMarker.name })).not.toBeInTheDocument();

    await userEvent.click(checkbox);

    // Không remount, không render lại bằng tay: cùng một cây React.
    expect(
      await screen.findByRole(panelMarker.role, { name: panelMarker.name }),
    ).toBeInTheDocument();
  });

  it("xoá khoá API thì panel biến mất ngay, không chờ khởi động lại app", async () => {
    // Chiều ngược lại quan trọng không kém: gỡ cấu hình mà panel vẫn còn đó thì
    // lễ tân gõ vào một trợ lý đã mất khoá và chỉ nhận về lỗi.
    const withoutKey: AssistantSettings = {
      ...ready,
      has_api_key: false,
      gate: { ready: false, missing: ["api_key"] },
    };
    state.backend = ready;
    useAssistantStore.setState({ settings: ready });
    invokeWriteCommand.mockImplementation(async (command: string) => {
      if (command !== "clear_assistant_api_key") throw new Error(`Lệnh ghi ngoài dự kiến: ${command}`);
      state.backend = withoutKey;
      return state.backend;
    });

    render(
      <>
        <AssistantSection />
        <AssistantPanel />
      </>,
    );

    expect(await screen.findByRole(panelMarker.role, { name: panelMarker.name })).toBeInTheDocument();

    await userEvent.click(await screen.findByRole("button", { name: /xoá khoá/i }));

    await waitFor(() =>
      expect(
        screen.queryByRole(panelMarker.role, { name: panelMarker.name }),
      ).not.toBeInTheDocument(),
    );
  });
});
