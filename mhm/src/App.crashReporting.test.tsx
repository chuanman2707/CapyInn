import type { ButtonHTMLAttributes, HTMLAttributes } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

import App from "./App";
import { clearMockResponses, setMockResponses } from "./__mocks__/tauri-core";
import { resetEventMocks } from "./__mocks__/tauri-event";
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
  Button: ({ children, ...props }: ButtonHTMLAttributes<HTMLButtonElement>) => <button {...props}>{children}</button>,
}));
vi.mock("sonner", () => ({
  toast: Object.assign(vi.fn(), { error: vi.fn() }),
  Toaster: () => <div data-testid="toaster" />,
}));
vi.mock("@/lib/crashReporting/sentry", () => ({
  hasRemoteCrashReporting: () => false,
  submitCrashBundle: vi.fn(),
}));

describe("App crash reporting flow", () => {
  beforeEach(() => {
    clearMockResponses();
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

  it("does not show the crash prompt before locked apps authenticate", async () => {
    setMockResponses({
      get_bootstrap_status: () => ({
        setup_completed: true,
        app_lock_enabled: true,
        current_user: null,
      }),
      get_pending_crash_report: () => ({
        bundle_id: "bundle-1",
        crash_type: "rust_panic",
        occurred_at: "2026-04-20T10:00:00+07:00",
        app_version: "0.1.1",
        environment: "production",
        platform: "macos",
        arch: "aarch64",
        installation_id: "install-1",
        message: "startup boom",
        stacktrace: ["frame-a"],
        module_hint: null,
        attempt_count: 0,
      }),
    });

    render(<App />);

    await waitFor(() => expect(screen.getByText("Login page")).toBeInTheDocument());
    expect(screen.queryByText("App encountered a serious error")).not.toBeInTheDocument();
  });

  it("shows the crash prompt after shellReady and exports the report", async () => {
    const user = userEvent.setup();

    setMockResponses({
      get_bootstrap_status: () => ({
        setup_completed: true,
        app_lock_enabled: false,
        current_user: {
          id: "owner",
          name: "Owner",
          role: "admin",
          active: true,
          created_at: "2026-04-20T00:00:00+07:00",
        },
      }),
      get_crash_reporting_preference: () => false,
      get_pending_crash_report: () => ({
        bundle_id: "bundle-1",
        crash_type: "rust_panic",
        occurred_at: "2026-04-20T10:00:00+07:00",
        app_version: "0.1.1",
        environment: "production",
        platform: "macos",
        arch: "aarch64",
        installation_id: "install-1",
        message: "startup boom",
        stacktrace: ["frame-a"],
        module_hint: null,
        attempt_count: 0,
      }),
      export_crash_report: () => "/Users/test/CapyInn/exports/crash-reports/bundle-1.json",
    });

    render(<App />);

    await waitFor(() => expect(screen.getByText("Overview")).toBeInTheDocument());
    expect(await screen.findByText("App encountered a serious error")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Export report" }));

    expect(invoke).toHaveBeenCalledWith("export_crash_report", {
      bundle_id: "bundle-1",
    });

    expect(
      await screen.findByText("/Users/test/CapyInn/exports/crash-reports/bundle-1.json"),
    ).toBeInTheDocument();
  });
});
