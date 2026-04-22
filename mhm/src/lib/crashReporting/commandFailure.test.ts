import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const { hasRemoteCrashReporting, submitCommandFailureEvent } = vi.hoisted(() => ({
  hasRemoteCrashReporting: vi.fn(),
  submitCommandFailureEvent: vi.fn(),
}));
vi.mock("./sentry", () => ({
  hasRemoteCrashReporting,
  submitCommandFailureEvent,
}));

import { invoke } from "@tauri-apps/api/core";

import { captureCommandFailure } from "./commandFailure";

describe("captureCommandFailure", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    hasRemoteCrashReporting.mockReturnValue(true);
  });

  it("sends allowlisted command failures when consent and DSN are enabled", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(true);

    const monitoringContext = {
      guest_count: 1,
      nights: 1,
      source: "phone",
      notes_present: false,
    } as const;

    await captureCommandFailure({
      command: "check_in",
      appError: {
        code: "BOOKING_GUEST_REQUIRED",
        message: "Phải có ít nhất 1 khách",
        kind: "user",
        support_id: null,
      },
      correlationId: "COR-8F3A1C7D",
      monitoringContext,
    });

    expect(invoke).toHaveBeenCalledWith("get_crash_reporting_preference");
    expect(submitCommandFailureEvent).toHaveBeenCalledWith(
      expect.objectContaining({
        command: "check_in",
        appError: {
          code: "BOOKING_GUEST_REQUIRED",
          message: "Phải có ít nhất 1 khách",
          kind: "user",
          support_id: null,
        },
        correlationId: "COR-8F3A1C7D",
        monitoringContext,
      }),
    );
  });

  it("sends system command failures when consent and DSN are enabled", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(true);

    const monitoringContext = {
      settlement_mode: "actual_nights",
    } as const;

    await captureCommandFailure({
      command: "check_out",
      appError: {
        code: "SYSTEM_INTERNAL_ERROR",
        message: "Có lỗi hệ thống, vui lòng thử lại",
        kind: "system",
        support_id: null,
      },
      correlationId: "COR-8F3A1C7D",
      monitoringContext,
    });

    expect(invoke).toHaveBeenCalledWith("get_crash_reporting_preference");
    expect(submitCommandFailureEvent).toHaveBeenCalledWith(
      expect.objectContaining({
        command: "check_out",
        appError: {
          code: "SYSTEM_INTERNAL_ERROR",
          message: "Có lỗi hệ thống, vui lòng thử lại",
          kind: "system",
          support_id: null,
        },
        correlationId: "COR-8F3A1C7D",
        monitoringContext,
      }),
    );
  });

  it("does not report commands outside the allowlist", async () => {
    await captureCommandFailure({
      command: "login",
      appError: {
        code: "AUTH_NOT_AUTHENTICATED",
        message: "Chưa đăng nhập",
        kind: "user",
        support_id: null,
      },
      correlationId: "COR-8F3A1C7D",
      monitoringContext: {
        guest_count: 1,
        nights: 1,
        source: "phone",
        notes_present: false,
      },
    });

    expect(invoke).not.toHaveBeenCalled();
    expect(submitCommandFailureEvent).not.toHaveBeenCalled();
  });

  it("does not report when remote crash reporting is disabled", async () => {
    hasRemoteCrashReporting.mockReturnValue(false);

    await captureCommandFailure({
      command: "run_night_audit",
      appError: {
        code: "AUDIT_INVALID_DATE",
        message: "Ngày kiểm toán không hợp lệ",
        kind: "user",
        support_id: null,
      },
      correlationId: "COR-8F3A1C7D",
      monitoringContext: { notes_present: false },
    });

    expect(invoke).not.toHaveBeenCalled();
    expect(submitCommandFailureEvent).not.toHaveBeenCalled();
  });

  it("does not submit when the user has not consented", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(false);

    await captureCommandFailure({
      command: "run_night_audit",
      appError: {
        code: "AUDIT_INVALID_DATE",
        message: "Ngày kiểm toán không hợp lệ",
        kind: "user",
        support_id: null,
      },
      correlationId: "COR-8F3A1C7D",
      monitoringContext: { notes_present: false },
    });

    expect(invoke).toHaveBeenCalledWith("get_crash_reporting_preference");
    expect(submitCommandFailureEvent).not.toHaveBeenCalled();
  });

  it.each([
    {
      name: "missing correlation id",
      options: {
        monitoringContext: {
          nights: 2,
          deposit_present: true,
          source: "online",
          notes_present: false,
        },
      },
    },
    {
      name: "missing monitoring context",
      options: {
        correlationId: "COR-8F3A1C7D",
      },
    },
  ])("does not report when $name", async ({ options }) => {
    await captureCommandFailure({
      command: "create_reservation",
      appError: {
        code: "BOOKING_INVALID_NIGHTS",
        message: "Số đêm phải lớn hơn 0",
        kind: "user",
        support_id: null,
      },
      correlationId: "correlationId" in options ? options.correlationId : undefined,
      monitoringContext:
        "monitoringContext" in options ? options.monitoringContext : undefined,
    });

    expect(invoke).not.toHaveBeenCalled();
    expect(submitCommandFailureEvent).not.toHaveBeenCalled();
  });
});
