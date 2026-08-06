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
  // Thanh điều hướng nay cũng tìm bằng `aria-label`, cùng lý do đã viết ở trên
  // cho panel. Bản cũ đi vòng `querySelector("nav")?.closest("aside")` vì lúc
  // đó `<aside>` này chưa có tên; nó gãy nếu thẻ `<nav>` bị bọc thêm một lớp,
  // và im lặng trỏ nhầm nếu một `<nav>` khác xuất hiện trước.
  const nav = container.querySelector('aside[aria-label="Điều hướng"]');
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

  // ── Nút Trợ lý ở thanh điều hướng ────────────────────────────────────────
  //
  // Nút này TỪNG nằm ở cụm phải của header, cạnh `SCANNER READY`. Nó mở/đóng
  // cột giữa, nên chỗ của nó là cột trái — ngay dưới logo, trên cả nhóm điều
  // hướng, cùng lối Airtable đặt nút Omni.
  //
  // Test đầu tiên là test SEAM: nó hỏi nút nằm Ở ĐÂU. Hai test cũ trong
  // `App.experimentalRuntime.test.tsx` chỉ hỏi "có một nút tên Trợ lý không" —
  // câu đó đúng cả trước lẫn sau khi ai đó trả nút về header, tức chúng không
  // canh vị trí. Đây là lớp lỗi nhánh trợ lý đã dính ba lần.
  describe("nút Trợ lý", () => {
    it("nằm trong thanh điều hướng, KHÔNG nằm ở header", async () => {
      const { container } = render(<MainShell />);
      await waitFor(() => expect(screen.getByText("Dashboard page")).toBeInTheDocument());

      const { nav } = landmarks(container);
      const header = container.querySelector("header")!;

      expect(within(nav as HTMLElement).getByRole("button", { name: /trợ lý/i })).toBeInTheDocument();
      // Vế âm mới là vế có răng: thiếu nó thì một bản vẽ nút ở CẢ HAI chỗ vẫn
      // xanh, mà hai nút cùng việc là đúng thứ thay đổi này đi dọn.
      expect(within(header).queryByRole("button", { name: /trợ lý/i })).toBeNull();
    });

    it("đứng trên cả nhóm điều hướng, không lẫn vào giữa các mục", async () => {
      const { container } = render(<MainShell />);
      await waitFor(() => expect(screen.getByText("Dashboard page")).toBeInTheDocument());

      const nav = landmarks(container).nav as HTMLElement;
      const assistant = within(nav).getByRole("button", { name: /trợ lý/i });
      const dashboard = within(nav).getByRole("button", { name: /dashboard/i });
      // `Element[]`, không phải `HTMLButtonElement[]`: `getByRole` trả về
      // `HTMLElement`, nên để nguyên kiểu hẹp của `querySelectorAll` thì
      // `indexOf` không chịu — và `npm test` vẫn xanh vì vitest KHÔNG typecheck.
      const order: Element[] = Array.from(nav.querySelectorAll("button"));

      expect(order.indexOf(assistant)).toBeLessThan(order.indexOf(dashboard));
    });

    it("sáng lên khi panel mở, tắt khi panel đóng", async () => {
      const user = userEvent.setup();
      const { container } = render(<MainShell />);
      await waitFor(() => expect(screen.getByText("Dashboard page")).toBeInTheDocument());

      const find = () =>
        within(landmarks(container).nav as HTMLElement).getByRole("button", { name: /trợ lý/i });

      // `aria-pressed`, không phải class CSS. Đây là chỗ khai báo nút này là
      // BẬT/TẮT chứ không phải một mục chuyển trang — và là thứ trình đọc màn
      // hình đọc được. So class thì chỉ đo được màu, không đo được nghĩa.
      expect(find()).toHaveAttribute("aria-pressed", "false");

      await user.click(find());

      await waitFor(() => expect(landmarks(container).panel).not.toBeNull());
      expect(find()).toHaveAttribute("aria-pressed", "true");

      await user.click(find());

      await waitFor(() => expect(landmarks(container).panel).toBeNull());
      expect(find()).toHaveAttribute("aria-pressed", "false");
    });

    it("không dùng chung icon với mục Housekeeping ngay bên dưới", async () => {
      const { container } = render(<MainShell />);
      await waitFor(() => expect(screen.getByText("Dashboard page")).toBeInTheDocument());

      const nav = landmarks(container).nav as HTMLElement;
      const iconOf = (name: RegExp) =>
        within(nav).getByRole("button", { name }).querySelector("svg")?.getAttribute("class") ?? "";

      const assistant = iconOf(/trợ lý/i);
      const housekeeping = iconOf(/housekeeping/i);

      // Chặn trước: cả hai phải đọc ra được. Không có dòng này thì lucide đổi
      // cách đặt class sẽ biến phép so thành `"" !== ""` — sai, nhưng theo
      // hướng nào thì cũng không còn đo gì nữa.
      expect(assistant).not.toBe("");
      expect(housekeeping).not.toBe("");
      // Lúc thanh thu về icon (mở trợ lý là nó tự thu), icon là thứ DUY NHẤT
      // còn lại để phân biệt hai mục cách nhau bốn dòng.
      expect(assistant).not.toBe(housekeeping);
    });

    it("chưa bật trợ lý thì không có cả nút lẫn vạch kẻ thừa", async () => {
      const notReady: AssistantSettings = {
        ...READY_SETTINGS,
        gate: { ready: false, missing: ["api_key"] },
      };
      setMockResponses({ get_assistant_settings: () => notReady });
      useAssistantStore.setState({ settings: notReady });

      const { container } = render(<MainShell />);
      await waitFor(() => expect(screen.getByText("Dashboard page")).toBeInTheDocument());

      const nav = landmarks(container).nav as HTMLElement;
      expect(within(nav).queryByRole("button", { name: /trợ lý/i })).toBeNull();
      // Vạch kẻ phải đi theo nút. Bọc-luôn-vẽ để lại một đường kẻ lơ lửng dưới
      // logo, và jsdom không nhìn thấy đường kẻ nên KHÔNG test dò chữ nào bắt
      // được — phải hỏi thẳng cái phần tử. `border-b` chỉ xuất hiện đúng một
      // lần trong `MainShell` (các vạch trong nav là `border-t`), nên phép hỏi
      // này trỏ đúng vào khối đang xét.
      expect(nav.querySelector("div.border-b")).toBeNull();
    });
  });
});
