import { beforeEach, describe, expect, it, vi } from "vitest";

const { captureEvent, flush, init } = vi.hoisted(() => ({
  captureEvent: vi.fn(),
  flush: vi.fn().mockResolvedValue(undefined),
  init: vi.fn(),
}));

vi.mock("@sentry/browser", () => ({
  captureEvent,
  flush,
  init,
}));

import { submitCommandFailureEvent } from "./sentry";

describe("submitCommandFailureEvent", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.stubGlobal("__SENTRY_DSN__", "dsn-for-test");
  });

  it.each([
    {
      name: "user",
      appError: {
        code: "BOOKING_GUEST_REQUIRED",
        message: "Phải có ít nhất 1 khách",
        kind: "user",
        support_id: "SUP-1",
      } as const,
      expectedLevel: "warning" as const,
    },
    {
      name: "system",
      appError: {
        code: "SYSTEM_INTERNAL_ERROR",
        message: "Có lỗi hệ thống, vui lòng thử lại",
        kind: "system",
        support_id: null,
      } as const,
      expectedLevel: "error" as const,
    },
  ])("builds the final Sentry payload for $name failures", async ({ appError, expectedLevel }) => {
    await submitCommandFailureEvent({
      command: "check_in",
      appError,
      correlationId: "COR-8F3A1C7D",
      monitoringContext: {
        guest_count: 1,
        nights: 1,
        source: "phone",
        notes_present: false,
      },
    });

    expect(captureEvent).toHaveBeenCalledWith({
      level: expectedLevel,
      message: `Command failure: check_in (${appError.code})`,
      fingerprint: ["command_failure", "check_in", appError.code],
      tags: {
        event_type: "command_failure",
        command: "check_in",
        code: appError.code,
        kind: appError.kind,
      },
      extra: {
        command: "check_in",
        code: appError.code,
        kind: appError.kind,
        support_id: appError.support_id,
        correlation_id: "COR-8F3A1C7D",
        context: {
          guest_count: 1,
          nights: 1,
          source: "phone",
          notes_present: false,
        },
      },
    });
    expect(flush).toHaveBeenCalledWith(2000);
  });
});
