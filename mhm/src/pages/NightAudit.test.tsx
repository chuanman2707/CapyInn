import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  createAppErrorException,
  formatAppError,
  type AppError,
} from "@/lib/appError";
import { useAuthStore } from "@/stores/useAuthStore";

const invoke = vi.hoisted(() => vi.fn());
const invokeCommand = vi.hoisted(() => vi.fn());
const createCorrelationId = vi.hoisted(() => vi.fn());
const toastError = vi.hoisted(() => vi.fn());
const toastSuccess = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke,
}));

vi.mock("@/lib/invokeCommand", () => ({
  invokeCommand,
}));

vi.mock("@/lib/correlationId", () => ({
  createCorrelationId,
}));

vi.mock("sonner", () => ({
  toast: {
    error: toastError,
    success: toastSuccess,
  },
}));

import NightAudit from "./NightAudit";

const auditRunError: AppError = {
  code: "AUDIT_DATE_ALREADY_RUN",
  message: "Đã audit ngày 2026-04-20 rồi!",
  kind: "user",
  support_id: null,
};

describe("NightAudit", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invoke.mockResolvedValue([]);
    invokeCommand.mockResolvedValue({
      id: "audit-1",
      audit_date: "2026-04-20",
      total_revenue: 1200000,
      room_revenue: 800000,
      folio_revenue: 400000,
      total_expenses: 200000,
      occupancy_pct: 30,
      rooms_sold: 3,
      total_rooms: 10,
      notes: "Đã kiểm tra kho",
      created_at: "2026-04-20T23:59:59+07:00",
    });
    createCorrelationId.mockReturnValue("COR-5E6F7A8B");
    useAuthStore.setState({
      user: { id: "u1", name: "Admin", role: "admin", active: true, created_at: "" },
      isAuthenticated: true,
      loading: false,
      error: null,
    });
  });

  it("uses invokeCommand with a generated correlation ID when running night audit", async () => {
    const user = userEvent.setup();
    const { container } = render(<NightAudit />);
    const dateInput = container.querySelector('input[type="date"]');

    expect(dateInput).not.toBeNull();

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("get_audit_logs");
    });

    fireEvent.change(dateInput!, { target: { value: "2026-04-20" } });
    await user.type(
      screen.getByPlaceholderText("VD: Đã kiểm tra kho..."),
      "Đã kiểm tra kho",
    );
    await user.click(screen.getByRole("button", { name: /chạy audit/i }));

    expect(createCorrelationId).toHaveBeenCalledTimes(1);
    await waitFor(() => {
      expect(invokeCommand).toHaveBeenCalledWith(
        "run_night_audit",
        {
          auditDate: "2026-04-20",
          notes: "Đã kiểm tra kho",
        },
        {
          correlationId: "COR-5E6F7A8B",
          monitoringContext: { notes_present: true },
        },
      );
    });
    expect(toastSuccess).toHaveBeenCalledWith(
      "Night Audit ngày 2026-04-20 hoàn tất!",
    );
  });

  it("formats invokeCommand failures with the correlation ID", async () => {
    const user = userEvent.setup();
    const error = createAppErrorException(auditRunError, undefined, {
      correlation_id: "COR-5E6F7A8B",
    });
    invokeCommand.mockRejectedValue(error);

    render(<NightAudit />);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("get_audit_logs");
    });

    await user.click(screen.getByRole("button", { name: /chạy audit/i }));

    await waitFor(() => {
      expect(invokeCommand).toHaveBeenCalledWith(
        "run_night_audit",
        expect.objectContaining({
          notes: null,
        }),
        {
          correlationId: "COR-5E6F7A8B",
          monitoringContext: { notes_present: false },
        },
      );
    });
    expect(toastError).toHaveBeenCalledWith(formatAppError(error));
  });
});

describe("NightAudit default audit date", () => {
  beforeEach(() => {
    invoke.mockReset();
    invokeCommand.mockReset();
    invoke.mockResolvedValue([]);
    useAuthStore.setState({
      user: { id: "u1", name: "Admin", role: "admin", active: true, created_at: "" },
      isAuthenticated: true,
      loading: false,
      error: null,
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  function renderAt(when: Date) {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(when);
    const { container } = render(<NightAudit />);
    return (container.querySelector('input[type="date"]') as HTMLInputElement).value;
  }

  /// The audit is one-shot per date — `run_night_audit` refuses a re-run and
  /// `mark_bookings_audited_tx` stamps the bookings — and nothing stops it
  /// auditing a day still in progress. So the default date decides whether a
  /// date gets permanently closed with partial revenue.
  it("defaults to the local day that has ended, on the night shift", () => {
    // 01:00 on the 22nd in Vietnam. The day that just ended is the 21st.
    expect(renderAt(new Date("2026-04-22T01:00:00+07:00"))).toBe("2026-04-21");
  });

  it("defaults to the same completed day during the working day", () => {
    // The old `toISOString()` value flipped here — 09:00 gave the 22nd, 01:00
    // gave the 21st — a rule that changes at 07:00 for no stateable reason.
    expect(renderAt(new Date("2026-04-22T09:00:00+07:00"))).toBe("2026-04-21");
  });

  it("never defaults to a day still in progress", () => {
    for (const hour of ["00:30", "06:59", "07:01", "23:30"]) {
      vi.useRealTimers();
      const value = renderAt(new Date(`2026-04-22T${hour}:00+07:00`));
      expect(value).not.toBe("2026-04-22");
      expect(value).toBe("2026-04-21");
    }
  });

  it("crosses a month boundary through the calendar, not by subtracting a day of milliseconds", () => {
    expect(renderAt(new Date("2026-05-01T02:00:00+07:00"))).toBe("2026-04-30");
  });
});
