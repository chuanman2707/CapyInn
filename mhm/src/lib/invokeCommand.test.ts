import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const captureCommandFailure = vi.hoisted(() => vi.fn().mockResolvedValue(undefined));

vi.mock("@tauri-apps/api/core", () => ({
  invoke,
}));

vi.mock("./crashReporting/commandFailure", () => ({
  captureCommandFailure,
}));

import { invokeWriteCommand } from "./invokeCommand";

describe("invokeWriteCommand", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invoke.mockResolvedValue("ok");
  });

  it("adds a command-scoped idempotency key", async () => {
    await invokeWriteCommand("create_reservation", { req: { room_id: "R101" } });

    expect(invoke).toHaveBeenCalledWith("create_reservation", {
      req: { room_id: "R101" },
      idempotencyKey: expect.stringMatching(/^create_reservation:/),
    });
  });

  it("preserves correlation id and monitoring context without leaking monitoring into payload", async () => {
    const monitoringContext = {
      nights: 2,
      deposit_present: true,
      source: "phone",
      notes_present: false,
    } as const;

    await invokeWriteCommand(
      "modify_reservation",
      { req: { booking_id: "B101" } },
      {
        correlationId: "COR-8F3A1C7D",
        monitoringContext,
      },
    );

    expect(invoke).toHaveBeenCalledWith("modify_reservation", {
      req: { booking_id: "B101" },
      idempotencyKey: expect.stringMatching(/^modify_reservation:/),
      correlationId: "COR-8F3A1C7D",
    });
  });

  it("normalizes structured errors through the existing invokeCommand path", async () => {
    const structuredError = {
      code: "AUTH_NOT_AUTHENTICATED",
      message: "Chưa đăng nhập",
      kind: "user",
      support_id: null,
    };
    invoke.mockRejectedValueOnce(structuredError);

    const promise = invokeWriteCommand(
      "cancel_reservation",
      { bookingId: "B101" },
      {
        correlationId: "COR-8F3A1C7D",
        monitoringContext: { notes_present: false },
      },
    );

    await expect(promise).rejects.toMatchObject({
      name: "AppError",
      code: "AUTH_NOT_AUTHENTICATED",
      message: "Chưa đăng nhập",
      kind: "user",
      support_id: null,
      correlation_id: "COR-8F3A1C7D",
      cause: structuredError,
    });
    expect(captureCommandFailure).toHaveBeenCalledWith({
      command: "cancel_reservation",
      appError: structuredError,
      correlationId: "COR-8F3A1C7D",
      monitoringContext: { notes_present: false },
    });
  });
});
