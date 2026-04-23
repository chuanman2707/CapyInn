import { beforeEach, describe, expect, it, vi } from "vitest";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const { captureCommandFailure } = vi.hoisted(() => ({
  captureCommandFailure: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("./crashReporting/commandFailure", () => ({ captureCommandFailure }));

import { invoke } from "@tauri-apps/api/core";

import errorCodes from "../../shared/error-codes.json";
import { invokeCommand } from "./invokeCommand";
import { createAppErrorException } from "./appError";
import {
  APP_ERROR_REGISTRY,
  FALLBACK_SYSTEM_APP_ERROR,
  formatAppError,
  normalizeAppError,
} from "./appError";

describe("appError", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("normalizes migrated user errors without changing the contract shape", () => {
    const error = normalizeAppError({
      code: "AUTH_INVALID_PIN",
      message: "Mã PIN không đúng",
      kind: "user",
      support_id: null,
    });

    expect(error).toEqual({
      code: "AUTH_INVALID_PIN",
      message: "Mã PIN không đúng",
      kind: "user",
      support_id: null,
    });
  });

  it("falls back to the generic system error for malformed payloads", () => {
    expect(
      normalizeAppError({
        code: "AUTH_INVALID_PIN",
        message: 123,
        kind: "user",
      }),
    ).toEqual(FALLBACK_SYSTEM_APP_ERROR);
  });

  it("falls back to the generic system error for unknown codes", () => {
    expect(
      normalizeAppError({
        code: "UNKNOWN_FRONTEND_CODE",
        message: "Some message",
        kind: "user",
        support_id: null,
      }),
    ).toEqual(FALLBACK_SYSTEM_APP_ERROR);
  });

  it("formats system support ids alongside the generic message", () => {
    const error = createAppErrorException(
      {
        code: "SYSTEM_INTERNAL_ERROR",
        message: "Có lỗi hệ thống, vui lòng thử lại",
        kind: "system",
        support_id: "SUP-ABCD1234",
      },
      undefined,
      {
        correlation_id: "COR-8F3A1C7D",
      },
    );

    expect(formatAppError(error)).toBe(
      "Có lỗi hệ thống, vui lòng thử lại (Mã hỗ trợ: SUP-ABCD1234)\nMã theo dõi: COR-8F3A1C7D",
    );
  });

  it("does not change invoke payloads when no correlation options are provided", async () => {
    vi.mocked(invoke).mockResolvedValueOnce("ok");

    await invokeCommand("login", { req: { pin: "0000" } });

    expect(invoke).toHaveBeenCalledWith("login", { req: { pin: "0000" } });
  });

  it("does not leak monitoring context into the invoke payload when remote capture rejects", async () => {
    const payload = {
      code: "BOOKING_GUEST_REQUIRED",
      message: "Phải có ít nhất 1 khách",
      kind: "user",
      support_id: null,
    };
    const monitoringContext = {
      guest_count: 0,
      nights: 1,
      source: null,
      notes_present: false,
    } as const;

    vi.mocked(invoke).mockRejectedValueOnce(payload);
    vi.mocked(captureCommandFailure).mockRejectedValueOnce(new Error("remote capture failed"));

    const promise = invokeCommand(
      "check_in",
      { req: { room_id: "room-101" } },
      {
        correlationId: "COR-8F3A1C7D",
        monitoringContext,
      },
    );

    expect(invoke).toHaveBeenCalledWith("check_in", {
      req: { room_id: "room-101" },
      correlationId: "COR-8F3A1C7D",
    });

    await expect(promise).rejects.toMatchObject({
      name: "AppError",
      code: "BOOKING_GUEST_REQUIRED",
      message: "Phải có ít nhất 1 khách",
      kind: "user",
      support_id: null,
      correlation_id: "COR-8F3A1C7D",
    });

    expect(captureCommandFailure).toHaveBeenCalledWith({
      command: "check_in",
      appError: payload,
      correlationId: "COR-8F3A1C7D",
      monitoringContext,
    });

    await promise.catch((error) => {
      expect(error).toBeInstanceOf(Error);
      expect(error).toMatchObject({
        name: "AppError",
        code: "BOOKING_GUEST_REQUIRED",
        message: "Phải có ít nhất 1 khách",
        kind: "user",
        support_id: null,
        correlation_id: "COR-8F3A1C7D",
      });
      expect(error.cause).toEqual(payload);
    });
  });

  it("stays aligned with the shared error registry", () => {
    expect(APP_ERROR_REGISTRY).toEqual(errorCodes);
    expect(FALLBACK_SYSTEM_APP_ERROR.message).toBe(
      errorCodes.find((entry) => entry.code === "SYSTEM_INTERNAL_ERROR")?.defaultMessage ??
        "Có lỗi hệ thống, vui lòng thử lại",
    );
  });

  it("rethrows structured auth errors as Error-like app errors", async () => {
    vi.mocked(invoke).mockRejectedValueOnce({
      code: "AUTH_INVALID_PIN",
      message: "Mã PIN không đúng",
      kind: "user",
      support_id: null,
    });

    const promise = invokeCommand("login", { req: { pin: "0000" } }, {
      correlationId: "COR-8F3A1C7D",
    });

    expect(invoke).toHaveBeenCalledWith("login", {
      req: { pin: "0000" },
      correlationId: "COR-8F3A1C7D",
    });

    await expect(promise).rejects.toMatchObject({
      name: "AppError",
      code: "AUTH_INVALID_PIN",
      message: "Mã PIN không đúng",
      kind: "user",
      support_id: null,
      correlation_id: "COR-8F3A1C7D",
    });

    await promise.catch((error) => {
      expect(error).toBeInstanceOf(Error);
      expect(error).toMatchObject({
        name: "AppError",
        code: "AUTH_INVALID_PIN",
        message: "Mã PIN không đúng",
        kind: "user",
        support_id: null,
        correlation_id: "COR-8F3A1C7D",
      });
      expect(error.cause).toEqual({
        code: "AUTH_INVALID_PIN",
        message: "Mã PIN không đúng",
        kind: "user",
        support_id: null,
      });
    });
  });

  it.each([
    {
      name: "unknown code",
      payload: {
        code: "UNKNOWN_FRONTEND_CODE",
        message: "Some message",
        kind: "user",
        support_id: null,
      },
    },
    {
      name: "malformed payload",
      payload: {
        code: "AUTH_INVALID_PIN",
        message: 123,
        kind: "user",
      },
    },
  ])("falls back to the generic system error for $name invoke payloads", async ({ payload }) => {
    vi.mocked(invoke).mockRejectedValueOnce(payload);

    const promise = invokeCommand("unknown_command");

    await promise.catch((error) => {
      expect(error).toBeInstanceOf(Error);
      expect(error).toMatchObject({
        name: "AppError",
        ...FALLBACK_SYSTEM_APP_ERROR,
      });
      expect(error.cause).toEqual(payload);
    });
  });
});
