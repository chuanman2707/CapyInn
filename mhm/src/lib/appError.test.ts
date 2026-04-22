import { beforeEach, describe, expect, it, vi } from "vitest";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

import { invoke } from "@tauri-apps/api/core";

import errorCodes from "../../shared/error-codes.json";
import { invokeCommand } from "./invokeCommand";
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
    expect(
      formatAppError({
        code: "SYSTEM_INTERNAL_ERROR",
        message: "Có lỗi hệ thống, vui lòng thử lại",
        kind: "system",
        support_id: "SUP-ABCD1234",
      }),
    ).toBe("Có lỗi hệ thống, vui lòng thử lại (Mã hỗ trợ: SUP-ABCD1234)");
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

    const promise = invokeCommand("login", { req: { pin: "0000" } });

    await expect(promise).rejects.toMatchObject({
      name: "AppError",
      code: "AUTH_INVALID_PIN",
      message: "Mã PIN không đúng",
      kind: "user",
      support_id: null,
    });

    await promise.catch((error) => {
      expect(error).toBeInstanceOf(Error);
      expect(error).toMatchObject({
        name: "AppError",
        code: "AUTH_INVALID_PIN",
        message: "Mã PIN không đúng",
        kind: "user",
        support_id: null,
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
