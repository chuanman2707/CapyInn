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
      guest_count: 2,
      nights: 3,
      source: "phone",
      notes_present: true,
    } as const;

    await captureCommandFailure(
      "check_in",
      {
        code: "AUTH_INVALID_PIN",
        message: "Mã PIN không đúng",
        kind: "user",
        support_id: null,
      },
      {
        correlationId: "COR-8F3A1C7D",
        monitoringContext,
      },
    );

    expect(invoke).toHaveBeenCalledWith("get_crash_reporting_preference");
    expect(submitCommandFailureEvent).toHaveBeenCalledWith({
      command: "check_in",
      code: "AUTH_INVALID_PIN",
      kind: "user",
      support_id: null,
      correlation_id: "COR-8F3A1C7D",
      context: monitoringContext,
    });
  });

  it("does not report commands outside the allowlist", async () => {
    await captureCommandFailure(
      "login",
      {
        code: "AUTH_INVALID_PIN",
        message: "Mã PIN không đúng",
        kind: "user",
        support_id: null,
      },
      {
        correlationId: "COR-8F3A1C7D",
        monitoringContext: {
          guest_count: 2,
          nights: 3,
          source: "phone",
          notes_present: true,
        },
      },
    );

    expect(invoke).not.toHaveBeenCalled();
    expect(submitCommandFailureEvent).not.toHaveBeenCalled();
  });

  it("does not report when remote crash reporting is disabled", async () => {
    hasRemoteCrashReporting.mockReturnValue(false);

    await captureCommandFailure(
      "check_out",
      {
        code: "AUTH_INVALID_PIN",
        message: "Mã PIN không đúng",
        kind: "user",
        support_id: null,
      },
      {
        correlationId: "COR-8F3A1C7D",
        monitoringContext: { settlement_mode: "actual_nights" },
      },
    );

    expect(invoke).not.toHaveBeenCalled();
    expect(submitCommandFailureEvent).not.toHaveBeenCalled();
  });

  it("does not submit when the user has not consented", async () => {
    vi.mocked(invoke).mockResolvedValueOnce(false);

    await captureCommandFailure(
      "run_night_audit",
      {
        code: "AUTH_INVALID_PIN",
        message: "Mã PIN không đúng",
        kind: "user",
        support_id: null,
      },
      {
        correlationId: "COR-8F3A1C7D",
        monitoringContext: { notes_present: false },
      },
    );

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
    await captureCommandFailure(
      "create_reservation",
      {
        code: "AUTH_INVALID_PIN",
        message: "Mã PIN không đúng",
        kind: "user",
        support_id: null,
      },
      options as never,
    );

    expect(invoke).not.toHaveBeenCalled();
    expect(submitCommandFailureEvent).not.toHaveBeenCalled();
  });
});
