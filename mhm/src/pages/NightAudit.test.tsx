import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

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
        { correlationId: "COR-5E6F7A8B" },
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
        { correlationId: "COR-5E6F7A8B" },
      );
    });
    expect(toastError).toHaveBeenCalledWith(formatAppError(error));
  });
});
