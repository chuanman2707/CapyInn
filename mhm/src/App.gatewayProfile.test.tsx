import type { ButtonHTMLAttributes, HTMLAttributes } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { listen } from "@tauri-apps/api/event";

import App from "./App";
import { clearMockResponses, invoke, setMockResponses } from "./__mocks__/tauri-core";
import { resetEventMocks } from "./__mocks__/tauri-event";
import { useAuthStore } from "./stores/useAuthStore";
import { useHotelStore } from "./stores/useHotelStore";

const wasEventListenerRegistered = (eventName: string) =>
  vi.mocked(listen).mock.calls.some(([registeredEventName]) => registeredEventName === eventName);

vi.mock("@/pages/Dashboard", () => ({ default: () => <div>Dashboard page</div> }));
vi.mock("@/pages/Rooms", () => ({ default: () => <div>Rooms page</div> }));
vi.mock("@/pages/Reservations", () => ({ default: () => <div>Reservations page</div> }));
vi.mock("@/pages/Guests", () => ({ default: () => <div>Guests page</div> }));
vi.mock("@/pages/Analytics", () => ({ default: () => <div>Analytics page</div> }));
vi.mock("@/pages/settings", () => ({ default: () => <div>Settings page</div> }));
vi.mock("@/pages/NightAudit", () => ({ default: () => <div>Night Audit page</div> }));
vi.mock("@/pages/LoginScreen", () => ({ default: () => <div>Login page</div> }));
vi.mock("@/pages/onboarding", () => ({ default: () => <div>Onboarding page</div> }));
vi.mock("@/components/CheckinSheet", () => ({ default: () => null }));
vi.mock("@/components/GroupCheckinSheet", () => ({ default: () => null }));
vi.mock("@/pages/GroupManagement", () => ({ default: () => <div>Group page</div> }));
vi.mock("@/components/AppLogo", () => ({ default: () => <div>Logo</div> }));
vi.mock("@/components/ui/badge", () => ({
  Badge: ({ children, ...props }: HTMLAttributes<HTMLDivElement>) => <div {...props}>{children}</div>,
}));
vi.mock("@/components/ui/button", () => ({
  Button: ({ children, ...props }: ButtonHTMLAttributes<HTMLButtonElement>) => <button {...props}>{children}</button>,
}));
vi.mock("sonner", () => ({
  toast: Object.assign(vi.fn(), { error: vi.fn() }),
  Toaster: () => <div data-testid="toaster" />,
}));

function resetStores() {
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
  useAuthStore.setState({
    user: null,
    isAuthenticated: false,
    loading: false,
    error: null,
  });
}

function mockUnlockedShell() {
  setMockResponses({
    get_bootstrap_status: () => ({
      setup_completed: true,
      app_lock_enabled: false,
      current_user: {
        id: "admin-1",
        name: "Owner",
        role: "admin",
        active: true,
        created_at: "2026-05-13T00:00:00.000Z",
      },
    }),
    get_experimental_runtime_status: () => ({
      experimental_runtime_enabled: false,
      gateway_runtime_enabled: false,
      agent_runtime_enabled: false,
      gateway_disabled_by_override: false,
      agent_disabled_by_override: false,
    }),
    gateway_get_status: () => ({ running: true }),
  });
}

describe("App gateway profile", () => {
  beforeEach(() => {
    clearMockResponses();
    resetEventMocks();
    vi.clearAllMocks();
    resetStores();
    mockUnlockedShell();
  });

  it("hides gateway and MCP shell surfaces in the normal profile", async () => {
    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("Overview")).toBeInTheDocument();
    });

    await waitFor(() => {
      expect(invoke.mock.calls.some(([command]) => command === "get_crash_reporting_preference")).toBe(true);
    });

    expect(screen.queryByText("● MCP Gateway")).not.toBeInTheDocument();
    expect(screen.queryByText("○ Gateway Off")).not.toBeInTheDocument();
    expect(invoke.mock.calls.some(([command]) => command === "gateway_get_status")).toBe(false);
    expect(wasEventListenerRegistered("mcp_reservation_created")).toBe(false);
  });

  it("enables gateway status and MCP listener in the experimental profile", async () => {
    setMockResponses({
      get_experimental_runtime_status: () => ({
        experimental_runtime_enabled: true,
        gateway_runtime_enabled: true,
        agent_runtime_enabled: false,
        gateway_disabled_by_override: false,
        agent_disabled_by_override: false,
      }),
    });

    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("● MCP Gateway")).toBeInTheDocument();
    });

    expect(invoke.mock.calls.some(([command]) => command === "gateway_get_status")).toBe(true);
    expect(wasEventListenerRegistered("mcp_reservation_created")).toBe(true);
  });
});
