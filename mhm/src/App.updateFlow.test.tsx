import type { ButtonHTMLAttributes, HTMLAttributes } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import App from "./App";
import { clearMockResponses, setMockResponses } from "./__mocks__/tauri-core";
import { resetEventMocks } from "./__mocks__/tauri-event";
import { clearMockUpdate, setMockAvailableUpdate } from "./__mocks__/tauri-updater";
import { useAuthStore } from "./stores/useAuthStore";
import { useHotelStore } from "./stores/useHotelStore";

vi.mock("./pages/Dashboard", () => ({ default: () => <div>Dashboard page</div> }));
vi.mock("./pages/Rooms", () => ({ default: () => <div>Rooms page</div> }));
vi.mock("./pages/Reservations", () => ({ default: () => <div>Reservations page</div> }));
vi.mock("./pages/Guests", () => ({ default: () => <div>Guests page</div> }));
vi.mock("./pages/Housekeeping", () => ({ default: () => <div>Housekeeping page</div> }));
vi.mock("./pages/Analytics", () => ({ default: () => <div>Analytics page</div> }));
vi.mock("./pages/settings", () => ({ default: () => <div>Settings page</div> }));
vi.mock("./pages/NightAudit", () => ({ default: () => <div>Night Audit page</div> }));
vi.mock("./pages/LoginScreen", () => ({ default: () => <div>Login page</div> }));
vi.mock("./pages/onboarding", () => ({ default: () => <div>Onboarding page</div> }));
vi.mock("./components/CheckinSheet", () => ({ default: () => null }));
vi.mock("./components/GroupCheckinSheet", () => ({ default: () => null }));
vi.mock("./pages/GroupManagement", () => ({ default: () => <div>Group page</div> }));
vi.mock("./components/AppLogo", () => ({ default: () => <div>Logo</div> }));
vi.mock("@/components/ui/badge", () => ({
  Badge: ({ children, ...props }: HTMLAttributes<HTMLDivElement>) => <div {...props}>{children}</div>,
}));
vi.mock("@/components/ui/button", () => ({
  Button: ({
    children,
    ...props
  }: ButtonHTMLAttributes<HTMLButtonElement>) => <button {...props}>{children}</button>,
}));
vi.mock("sonner", () => ({
  toast: Object.assign(vi.fn(), { error: vi.fn() }),
  Toaster: () => <div data-testid="toaster" />,
}));

function setUserAgent(value: string) {
  Object.defineProperty(window.navigator, "userAgent", {
    value,
    configurable: true,
  });
}

describe("App update flow", () => {
  beforeEach(() => {
    clearMockResponses();
    clearMockUpdate();
    resetEventMocks();
    vi.clearAllMocks();

    useHotelStore.setState({
      rooms: [],
      stats: null,
      roomDetail: null,
      activeTab: "dashboard",
      housekeepingTasks: [],
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
  });

  it("does not auto-check before the app reaches the main shell", async () => {
    setUserAgent("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)");
    setMockAvailableUpdate({ version: "0.2.0" });
    setMockResponses({
      get_bootstrap_status: () => ({
        setup_completed: false,
        app_lock_enabled: false,
        current_user: null,
      }),
    });

    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("Onboarding page")).toBeInTheDocument();
    });
    expect(screen.queryByRole("button", { name: "UPDATE" })).not.toBeInTheDocument();
  });

  it("checks after shell gates clear, downloads silently from UPDATE, and keeps restart pending after Later", async () => {
    const user = userEvent.setup();

    setUserAgent("Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0)");
    setMockAvailableUpdate({ version: "0.2.0" });
    setMockResponses({
      get_bootstrap_status: () => ({
        setup_completed: true,
        app_lock_enabled: false,
        current_user: {
          id: "admin-1",
          name: "Owner",
          role: "admin",
          active: true,
          created_at: "2026-04-18T00:00:00.000Z",
        },
      }),
      gateway_get_status: () => ({ running: true }),
    });

    render(<App />);

    await waitFor(() => {
      expect(screen.getByText("Overview")).toBeInTheDocument();
    });

    const updateButton = await screen.findByRole("button", { name: "UPDATE" });
    await user.click(updateButton);

    expect(await screen.findByText("Restart to update")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Later" }));

    expect(screen.queryByText("Restart to update")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "RESTART TO UPDATE" }));

    expect(screen.getByText("Restart to update")).toBeInTheDocument();
  });
});
