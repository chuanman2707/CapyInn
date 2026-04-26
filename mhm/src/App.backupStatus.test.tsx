import type { ButtonHTMLAttributes, HTMLAttributes } from "react";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { toast } from "sonner";

import App from "./App";
import { clearMockResponses, setMockResponses } from "./__mocks__/tauri-core";
import { emitTestEvent, resetEventMocks } from "./__mocks__/tauri-event";
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
vi.mock("sonner", () => {
  const mockedToast = Object.assign(vi.fn(), { error: vi.fn() });
  return {
    toast: mockedToast,
    Toaster: () => <div data-testid="toaster" />,
  };
});

describe("App backup status integration", () => {
  beforeEach(() => {
    clearMockResponses();
    resetEventMocks();
    vi.useFakeTimers();
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
    });
  });

  it("keeps the saving indicator visible until pending_jobs reaches zero", async () => {
    render(<App />);

    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByText("Overview")).toBeInTheDocument();

    await act(async () => {
      await emitTestEvent("backup-status", {
        job_id: "job-1",
        state: "started",
        reason: "checkout",
        pending_jobs: 2,
      });
    });
    expect(screen.getByText("Đang sao lưu dữ liệu...")).toBeInTheDocument();

    await act(async () => {
      await emitTestEvent("backup-status", {
        job_id: "job-1",
        state: "completed",
        reason: "checkout",
        pending_jobs: 1,
        path: "/tmp/job-1.db",
      });
    });
    expect(screen.getByText("Đang sao lưu dữ liệu...")).toBeInTheDocument();

    await act(async () => {
      await emitTestEvent("backup-status", {
        job_id: "job-2",
        state: "completed",
        reason: "app_exit",
        pending_jobs: 0,
        path: "/tmp/job-2.db",
      });
    });
    expect(screen.getByText("Đã sao lưu")).toBeInTheDocument();

    await act(async () => {
      vi.advanceTimersByTime(1799);
    });
    expect(screen.getByText("Đã sao lưu")).toBeInTheDocument();

    await act(async () => {
      vi.advanceTimersByTime(1);
    });
    expect(screen.queryByText("Đã sao lưu")).not.toBeInTheDocument();
  });

  it("shows a failure toast and clears the saved hide timer when a new job starts", async () => {
    render(<App />);

    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByText("Overview")).toBeInTheDocument();

    await act(async () => {
      await emitTestEvent("backup-status", {
        job_id: "job-1",
        state: "failed",
        reason: "manual",
        pending_jobs: 1,
        message: "Ổ đĩa đầy",
      });
    });
    const status = screen.getByRole("status");
    expect(status).toHaveAttribute("data-phase", "failed");
    expect(within(status).getByText("Sao lưu thất bại")).toBeInTheDocument();
    expect(toast.error).toHaveBeenCalledWith("Ổ đĩa đầy");

    await act(async () => {
      await emitTestEvent("backup-status", {
        job_id: "job-2",
        state: "completed",
        reason: "manual",
        pending_jobs: 0,
        path: "/tmp/job-2.db",
      });
    });
    expect(screen.getByText("Đã sao lưu")).toBeInTheDocument();

    await act(async () => {
      vi.advanceTimersByTime(1000);
      await emitTestEvent("backup-status", {
        job_id: "job-3",
        state: "started",
        reason: "settings",
        pending_jobs: 1,
      });
    });
    expect(screen.getByText("Đang sao lưu dữ liệu...")).toBeInTheDocument();

    await act(async () => {
      vi.advanceTimersByTime(1000);
    });
    expect(screen.getByText("Đang sao lưu dữ liệu...")).toBeInTheDocument();
  });

  it("shows a persistent alert alongside the failed status indicator", async () => {
    render(<App />);

    await act(async () => {
      await Promise.resolve();
    });

    await act(async () => {
      await emitTestEvent("backup-status", {
        job_id: "job-1",
        state: "failed",
        reason: "manual",
        pending_jobs: 1,
        message: "Ổ đĩa đầy",
      });
    });

    const alert = screen.getByRole("alert", { name: "Sao lưu thất bại" });
    expect(within(alert).getByText("Ổ đĩa đầy")).toBeInTheDocument();
    expect(within(alert).getByText("Nguồn: Thủ công")).toBeInTheDocument();

    const status = screen.getByRole("status");
    expect(status).toHaveAttribute("data-phase", "failed");
    expect(within(status).getByText("Sao lưu thất bại")).toBeInTheDocument();
  });

  it("uses fallback copy and source label for scheduled failures without a backend message", async () => {
    render(<App />);

    await act(async () => {
      await Promise.resolve();
    });

    await act(async () => {
      await emitTestEvent("backup-status", {
        job_id: "scheduled-1",
        state: "failed",
        reason: "scheduled",
        pending_jobs: 0,
      });
    });

    const alert = screen.getByRole("alert", { name: "Sao lưu thất bại" });
    expect(
      within(alert).getByText(
        "Không thể tạo bản sao lưu. Vui lòng kiểm tra dung lượng ổ đĩa hoặc thử lại.",
      ),
    ).toBeInTheDocument();
    expect(within(alert).getByText("Nguồn: Tự động")).toBeInTheDocument();
  });

  it("dismisses only the current failed job and reopens for a later failure", async () => {
    render(<App />);

    await act(async () => {
      await Promise.resolve();
    });

    await act(async () => {
      await emitTestEvent("backup-status", {
        job_id: "job-1",
        state: "failed",
        reason: "manual",
        pending_jobs: 1,
        message: "Ổ đĩa đầy",
      });
    });

    fireEvent.click(screen.getByRole("button", { name: "Đóng cảnh báo sao lưu" }));
    expect(screen.queryByRole("alert", { name: "Sao lưu thất bại" })).not.toBeInTheDocument();

    await act(async () => {
      await emitTestEvent("backup-status", {
        job_id: "job-2",
        state: "started",
        reason: "scheduled",
        pending_jobs: 1,
      });
    });
    expect(screen.queryByRole("alert", { name: "Sao lưu thất bại" })).not.toBeInTheDocument();

    await act(async () => {
      await emitTestEvent("backup-status", {
        job_id: "job-2",
        state: "completed",
        reason: "scheduled",
        pending_jobs: 1,
        path: "/tmp/job-2.db",
      });
    });
    expect(screen.queryByRole("alert", { name: "Sao lưu thất bại" })).not.toBeInTheDocument();

    await act(async () => {
      await emitTestEvent("backup-status", {
        job_id: "job-3",
        state: "failed",
        reason: "checkout",
        pending_jobs: 0,
        message: "Không thể ghi file",
      });
    });

    const alert = screen.getByRole("alert", { name: "Sao lưu thất bại" });
    expect(within(alert).getByText("Không thể ghi file")).toBeInTheDocument();
    expect(within(alert).getByText("Nguồn: Trả phòng")).toBeInTheDocument();
  });

  it("keeps the alert while queued jobs remain and clears it after the queue drains", async () => {
    render(<App />);

    await act(async () => {
      await Promise.resolve();
    });

    await act(async () => {
      await emitTestEvent("backup-status", {
        job_id: "job-1",
        state: "failed",
        reason: "manual",
        pending_jobs: 2,
        message: "Ổ đĩa đầy",
      });
    });
    expect(screen.getByRole("alert", { name: "Sao lưu thất bại" })).toBeInTheDocument();

    await act(async () => {
      await emitTestEvent("backup-status", {
        job_id: "job-2",
        state: "completed",
        reason: "scheduled",
        pending_jobs: 1,
        path: "/tmp/job-2.db",
      });
    });
    expect(screen.getByRole("alert", { name: "Sao lưu thất bại" })).toBeInTheDocument();

    await act(async () => {
      await emitTestEvent("backup-status", {
        job_id: "job-3",
        state: "completed",
        reason: "scheduled",
        pending_jobs: 0,
        path: "/tmp/job-3.db",
      });
    });
    expect(screen.queryByRole("alert", { name: "Sao lưu thất bại" })).not.toBeInTheDocument();
  });

  it("handles scheduled backup status events", async () => {
    render(<App />);

    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByText("Overview")).toBeInTheDocument();

    await act(async () => {
      await emitTestEvent("backup-status", {
        job_id: "scheduled-1",
        state: "started",
        reason: "scheduled",
        pending_jobs: 1,
      });
    });
    expect(screen.getByText("Đang sao lưu dữ liệu...")).toBeInTheDocument();

    await act(async () => {
      await emitTestEvent("backup-status", {
        job_id: "scheduled-1",
        state: "completed",
        reason: "scheduled",
        pending_jobs: 0,
        path: "/tmp/scheduled-1.db",
      });
    });
    expect(screen.getByText("Đã sao lưu")).toBeInTheDocument();
  });
});
