import type { ButtonHTMLAttributes, HTMLAttributes } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import App from "./App";
import { clearMockResponses, invoke, setMockResponses } from "./__mocks__/tauri-core";
import { listen, resetEventMocks } from "./__mocks__/tauri-event";
import * as experimentalProfile from "@/lib/experimentalProfile";
import type { ExperimentalRuntimeStatus } from "@/lib/experimentalProfile";
import { useAuthStore } from "./stores/useAuthStore";
import { useHotelStore } from "./stores/useHotelStore";

vi.mock("./pages/Dashboard", () => ({ default: () => <div>Dashboard page</div> }));
vi.mock("./pages/Rooms", () => ({ default: () => <div>Rooms page</div> }));
vi.mock("./pages/Reservations", () => ({ default: () => <div>Reservations page</div> }));
vi.mock("./pages/Guests", () => ({ default: () => <div>Guests page</div> }));
vi.mock("./pages/Analytics", () => ({ default: () => <div>Analytics page</div> }));
vi.mock("./pages/settings", () => ({ default: () => <div>Settings page</div> }));
vi.mock("./pages/NightAudit", () => ({ default: () => <div>Night Audit page</div> }));
vi.mock("./pages/LoginScreen", () => ({ default: () => <div>Login page</div> }));
vi.mock("./pages/onboarding", () => ({ default: () => <div>Onboarding page</div> }));
vi.mock("./pages/GroupManagement", () => ({ default: () => <div>Group page</div> }));
vi.mock("./components/CheckinSheet", () => ({ default: () => null }));
vi.mock("./components/GroupCheckinSheet", () => ({ default: () => null }));
vi.mock("./components/AppLogo", () => ({ default: () => <div>Logo</div> }));
vi.mock("./hooks/useAppUpdateController", () => ({
  useAppUpdateController: () => ({
    supported: false,
    phase: "idle",
    currentVersion: "0.1.1",
    availableVersion: null,
    restartPromptOpen: false,
    errorMessage: null,
    canCheck: false,
    checkForUpdates: vi.fn(),
    downloadUpdate: vi.fn(),
    dismissRestartPrompt: vi.fn(),
    openRestartPrompt: vi.fn(),
    confirmInstall: vi.fn(),
  }),
}));
vi.mock("@/components/ui/badge", () => ({
  Badge: ({ children, ...props }: HTMLAttributes<HTMLDivElement>) => (
    <div {...props}>{children}</div>
  ),
}));
vi.mock("@/components/ui/button", () => ({
  Button: ({ children, ...props }: ButtonHTMLAttributes<HTMLButtonElement>) => (
    <button {...props}>{children}</button>
  ),
}));
vi.mock("sonner", () => ({
  toast: Object.assign(vi.fn(), { error: vi.fn() }),
  Toaster: () => <div data-testid="toaster" />,
}));

function setupAuthenticatedShell(
  runtimeStatus = {
    experimental_runtime_enabled: false,
    gateway_runtime_enabled: false,
    agent_runtime_enabled: false,
    gateway_disabled_by_override: false,
    agent_disabled_by_override: false,
  },
) {
  useAuthStore.setState({
    user: { id: "admin-1", name: "Owner", role: "admin", active: true, created_at: "" },
    isAuthenticated: true,
    loading: false,
    error: null,
  });
  useHotelStore.setState({
    rooms: [],
    stats: null,
    roomDetail: null,
    activeTab: "dashboard",
    loading: false,
    isCheckinOpen: false,
    checkinRoomId: null,
    isGroupCheckinOpen: false,
    groups: [],
  });
  setMockResponses({
    get_bootstrap_status: () => ({
      setup_completed: true,
      app_lock_enabled: false,
      current_user: { id: "admin-1", name: "Owner", role: "admin", active: true, created_at: "" },
    }),
    get_experimental_runtime_status: () => runtimeStatus,
    get_rooms: () => [],
    get_dashboard_stats: () => ({
      total_rooms: 10,
      occupied: 0,
      vacant: 10,
      cleaning: 0,
      revenue_today: 0,
    }),
  });
}

describe("App experimental runtime gates", () => {
  beforeEach(() => {
    clearMockResponses();
    resetEventMocks();
    invoke.mockClear();
    vi.clearAllMocks();
    localStorage.setItem("sidebar-collapsed", "false");
  });

  it("normal profile does not call gateway status, render gateway badge, or subscribe to MCP events", async () => {
    setupAuthenticatedShell();

    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("Overview")).toBeInTheDocument();
    });

    await waitFor(() => {
      expect(invoke.mock.calls.some(([command]) => command === "get_experimental_runtime_status")).toBe(true);
    });
    expect(invoke.mock.calls.some(([command]) => command === "gateway_get_status")).toBe(false);
    expect(screen.queryByText(/Gateway/i)).not.toBeInTheDocument();
    expect(listen).not.toHaveBeenCalledWith("mcp_reservation_created", expect.any(Function));
  });

  it("experimental gateway profile checks gateway status and subscribes to MCP events", async () => {
    setupAuthenticatedShell({
      experimental_runtime_enabled: false,
      gateway_runtime_enabled: true,
      agent_runtime_enabled: false,
      gateway_disabled_by_override: false,
      agent_disabled_by_override: false,
    });
    setMockResponses({
      gateway_get_status: () => ({ running: true, port: 61239, has_api_keys: true }),
    });

    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("● MCP Gateway")).toBeInTheDocument();
    });

    expect(invoke.mock.calls.some(([command]) => command === "gateway_get_status")).toBe(true);
    expect(listen).toHaveBeenCalledWith("mcp_reservation_created", expect.any(Function));
  });

  // ─── Công tắc tắt của trợ lý quầy ───
  //
  // Trợ lý đã tách khỏi CAPYINN_EXPERIMENTAL_AGENT_RUNTIME. Cờ đó chỉ bật được
  // bằng biến môi trường lúc khởi động, mà lễ tân thì bấm icon để mở app — nên
  // nó giấu tính năng khỏi đúng những người tính năng sinh ra để phục vụ.
  //
  // Công tắc tắt bây giờ là `gate.ready` (API key + opt-in, lưu trong
  // database). Hai test dưới đây khoá cả hai chiều: không cờ vẫn phải mở được,
  // và tắt opt-in vẫn phải đóng được. Thiếu chiều thứ hai thì "gỡ cờ" biến
  // thành "gỡ luôn cách tắt".

  const READY_ASSISTANT_SETTINGS = {
    config: { preset: "deep_seek", base_url: "https://api.deepseek.com/v1", model: "deepseek-chat" },
    has_api_key: true,
    cloud_data_opt_in: true,
    gate: { ready: true, missing: [] },
  };

  it("không cờ thí nghiệm nào bật, lễ tân vẫn thấy nút Trợ lý", async () => {
    setupAuthenticatedShell();
    setMockResponses({ get_assistant_settings: () => READY_ASSISTANT_SETTINGS });

    render(<App />);

    await waitFor(() => {
      expect(screen.getByRole("button", { name: /trợ lý/i })).toBeInTheDocument();
    });
    expect(invoke.mock.calls.some(([command]) => command === "get_assistant_settings")).toBe(true);
  });

  it("chưa bật opt-in thì không có nút Trợ lý, dù cờ agent runtime có bật", async () => {
    setupAuthenticatedShell({
      experimental_runtime_enabled: false,
      gateway_runtime_enabled: false,
      agent_runtime_enabled: true,
      gateway_disabled_by_override: false,
      agent_disabled_by_override: false,
    });
    setMockResponses({
      get_assistant_settings: () => ({
        ...READY_ASSISTANT_SETTINGS,
        cloud_data_opt_in: false,
        gate: { ready: false, missing: ["cloud_data_opt_in"] },
      }),
    });

    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("Overview")).toBeInTheDocument();
    });

    expect(screen.queryByRole("button", { name: /trợ lý/i })).not.toBeInTheDocument();
  });

  it("ignores gateway status responses from a disabled profile generation", async () => {
    setupAuthenticatedShell();

    let runtimeStatus: ExperimentalRuntimeStatus = {
      experimentalRuntimeEnabled: false,
      gatewayRuntimeEnabled: true,
      agentRuntimeEnabled: false,
      gatewayDisabledByOverride: false,
      agentDisabledByOverride: false,
    };
    const runtimeSpy = vi
      .spyOn(experimentalProfile, "useExperimentalRuntimeStatus")
      .mockImplementation(() => runtimeStatus);

    let resolveFirstGatewayStatus: (status: { running: boolean }) => void = () => {};
    const firstGatewayStatus = new Promise<{ running: boolean }>((resolve) => {
      resolveFirstGatewayStatus = resolve;
    });
    const secondGatewayStatus = new Promise<{ running: boolean }>(() => {});
    let gatewayStatusCalls = 0;

    setMockResponses({
      gateway_get_status: () => {
        gatewayStatusCalls += 1;
        return gatewayStatusCalls === 1 ? firstGatewayStatus : secondGatewayStatus;
      },
    });

    const { rerender } = render(<App />);

    await waitFor(() => {
      expect(gatewayStatusCalls).toBe(1);
    });

    await act(async () => {
      runtimeStatus = {
        ...runtimeStatus,
        gatewayRuntimeEnabled: false,
      };
      rerender(<App />);
    });

    await waitFor(() => {
      expect(screen.queryByText(/Gateway/i)).not.toBeInTheDocument();
    });

    await act(async () => {
      resolveFirstGatewayStatus({ running: true });
      await Promise.resolve();
    });

    await act(async () => {
      runtimeStatus = {
        ...runtimeStatus,
        gatewayRuntimeEnabled: true,
      };
      rerender(<App />);
    });

    await waitFor(() => {
      expect(gatewayStatusCalls).toBe(2);
    });

    expect(screen.getByText("○ Gateway Off")).toBeInTheDocument();
    expect(screen.queryByText("● MCP Gateway")).not.toBeInTheDocument();
    runtimeSpy.mockRestore();
  });
});
