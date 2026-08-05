import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { clearMockResponses, setMockResponses } from "@/__mocks__/tauri-core";
import { useAssistantStore } from "@/stores/useAssistantStore";
import { useAuthStore } from "@/stores/useAuthStore";
import { useHotelStore } from "@/stores/useHotelStore";

// Hai context này bọc MainShell trong app thật. Test bố cục không dựng lại cả
// cây provider — nó chỉ cần shell nhận đủ trường để render.
vi.mock("@/app/RuntimeStateProvider", () => ({
  useRuntimeState: () => ({
    backupUi: { visible: false, phase: "idle", message: "" },
    visibleBackupFailure: null,
    onDismissBackupFailure: vi.fn(),
    pendingCrashReport: null,
    crashPromptBusy: false,
    crashExportPath: null,
    onSendCrashReport: vi.fn(),
    onDismissCrashReport: vi.fn(),
    onExportCrashReport: vi.fn(),
    gatewayRunning: false,
    gatewayRuntimeEnabled: false,
    remoteCrashReportingEnabled: false,
  }),
}));

vi.mock("@/contexts/AppUpdateContext", () => ({
  useAppUpdate: () => ({
    phase: "idle",
    restartPromptOpen: false,
    availableVersion: null,
    currentVersion: "0.0.0",
    confirmInstall: vi.fn(),
    dismissRestartPrompt: vi.fn(),
    downloadUpdate: vi.fn(),
    openRestartPrompt: vi.fn(),
  }),
}));

vi.mock("@/components/AppLogo", () => ({ default: () => <div>Logo</div> }));
vi.mock("@/components/AppUpdateBadge", () => ({ default: () => null }));
vi.mock("@/components/AppUpdateRestartModal", () => ({ default: () => null }));
vi.mock("@/components/BackupStatusIndicator", () => ({ BackupStatusIndicator: () => null }));
vi.mock("@/components/CheckinSheet", () => ({ default: () => null }));
vi.mock("@/components/GroupCheckinSheet", () => ({ default: () => null }));
vi.mock("@/components/RoomChangeSheet", () => ({ RoomChangeSheet: () => null }));
vi.mock("@/app/AppToaster", () => ({ AppToaster: () => null }));
vi.mock("@/pages/Analytics", () => ({ default: () => <div>Analytics page</div> }));
vi.mock("@/pages/Dashboard", () => ({ default: () => <div>Dashboard page</div> }));
vi.mock("@/pages/Declaration", () => ({ default: () => <div>Declaration page</div> }));
vi.mock("@/pages/GroupManagement", () => ({ default: () => <div>Group page</div> }));
vi.mock("@/pages/Guests", () => ({ default: () => <div>Guests page</div> }));
vi.mock("@/pages/Housekeeping", () => ({ default: () => <div>Housekeeping page</div> }));
vi.mock("@/pages/NightAudit", () => ({ default: () => <div>Night Audit page</div> }));
vi.mock("@/pages/Reservations", () => ({ default: () => <div>Reservations page</div> }));
vi.mock("@/pages/Rooms", () => ({ default: () => <div>Rooms page</div> }));
vi.mock("@/pages/settings", () => ({ default: () => <div>Settings page</div> }));

import type { AssistantSettings } from "@/types/assistant";

import { MainShell } from "./MainShell";

const READY_SETTINGS: AssistantSettings = {
  config: { preset: "deep_seek", base_url: "https://x/y", model: "deepseek-chat" },
  has_api_key: true,
  cloud_data_opt_in: true,
  gate: { ready: true, missing: [] },
};

/// Ba mốc của bố cục, tìm bằng ruột chứ không bằng thứ tự — dùng thứ tự để tìm
/// rồi kiểm thứ tự thì test luôn xanh.
///
/// Panel tìm bằng `aria-label`, KHÔNG bằng `querySelector("form")`. Bám vào
/// `<form>` là bám vào ruột mà Task 8 được giao viết lại: khung soạn thôi là
/// `<form>` thì `panel` thành null, ba test bố cục đỏ **vì lý do sai**, và tệ
/// hơn — test cổng `gate.ready` bên dưới khẳng định `panel === null` nên nó
/// **xanh vĩnh viễn** rồi thôi canh gì cả. Một `<form>` lạ xuất hiện trước
/// panel (ô tìm kiếm trong nav) cũng làm nó trỏ nhầm sang `<aside>` nav.
/// `aria-label` vừa bền vừa thật: nó đặt tên cho landmark, không phải móc chỉ
/// để test bám vào.
function landmarks(container: HTMLElement) {
  const nav = container.querySelector("nav")?.closest("aside");
  const panel = container.querySelector('aside[aria-label="Trợ lý quầy"]');
  const content = container.querySelector("main");
  const inDocumentOrder = Array.from(container.querySelectorAll("aside, main"));
  return { nav, panel, content, inDocumentOrder };
}

describe("MainShell — bố cục ba cột", () => {
  beforeEach(() => {
    clearMockResponses();
    setMockResponses({
      kbtt_undeclared_count: () => 0,
      get_assistant_settings: () => READY_SETTINGS,
    });

    // jsdom mặc định 1024px, nằm dưới mốc hẹp 1200 nên thanh điều hướng sẽ thu
    // sẵn vì lý do "cửa sổ hẹp" và mọi khẳng định về trợ lý mất ý nghĩa.
    Object.defineProperty(window, "innerWidth", { value: 1456, configurable: true });
    localStorage.clear();

    useAuthStore.setState({ user: null, isAuthenticated: true, loading: false, error: null });
    useHotelStore.setState({ activeTab: "dashboard", roomDetail: null });
    useAssistantStore.setState({
      open: false,
      messages: [],
      pendingAction: null,
      busy: false,
      settings: READY_SETTINGS,
    });
  });

  it("xếp thanh điều hướng, panel trợ lý rồi mới tới nội dung", async () => {
    useAssistantStore.setState({ open: true });
    const { container } = render(<MainShell />);

    await waitFor(() => expect(landmarks(container).panel).not.toBeNull());
    const { nav, panel, content, inDocumentOrder } = landmarks(container);

    expect(inDocumentOrder.indexOf(nav!)).toBeLessThan(inDocumentOrder.indexOf(panel!));
    expect(inDocumentOrder.indexOf(panel!)).toBeLessThan(inDocumentOrder.indexOf(content!));
  });

  it("panel kẻ viền về phía nội dung, không phải phía thanh điều hướng", async () => {
    useAssistantStore.setState({ open: true });
    const { container } = render(<MainShell />);

    await waitFor(() => expect(landmarks(container).panel).not.toBeNull());

    expect(landmarks(container).panel).toHaveClass("border-r");
    expect(landmarks(container).panel).not.toHaveClass("border-l");
  });

  it("mở trợ lý thì thanh điều hướng thu về icon", async () => {
    const { container } = render(<MainShell />);
    expect(screen.getByText("Main")).toBeInTheDocument();

    useAssistantStore.setState({ open: true });

    await waitFor(() => expect(landmarks(container).panel).not.toBeNull());
    expect(screen.queryByText("Main")).not.toBeInTheDocument();
  });

  it("lễ tân bấm mở lại được thanh trong lúc trợ lý vẫn mở", async () => {
    const user = userEvent.setup();
    useAssistantStore.setState({ open: true });
    const { container } = render(<MainShell />);
    await waitFor(() => expect(landmarks(container).panel).not.toBeNull());
    expect(screen.queryByText("Main")).not.toBeInTheDocument();

    await user.click(screen.getByTitle("Mở rộng"));

    expect(screen.getByText("Main")).toBeInTheDocument();
    expect(landmarks(container).panel).not.toBeNull();
  });

  it("chưa bật trợ lý trong Cài đặt thì không dựng cột giữa", async () => {
    const notReady: AssistantSettings = {
      ...READY_SETTINGS,
      gate: { ready: false, missing: ["api_key"] },
    };
    setMockResponses({ get_assistant_settings: () => notReady });
    useAssistantStore.setState({ open: true, settings: notReady });

    const { container } = render(<MainShell />);

    await waitFor(() => expect(screen.getByText("Dashboard page")).toBeInTheDocument());
    expect(landmarks(container).panel).toBeNull();
    // Quét cả cây, không quét trong `<main>`: panel là anh em của `<main>` chứ
    // không phải con, nên tìm trong `<main>` là một khẳng định luôn đúng.
    expect(within(container).queryByPlaceholderText("Hỏi hoặc ra việc…")).toBeNull();
  });
});
