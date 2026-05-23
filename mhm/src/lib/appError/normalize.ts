import type { AppError, AppErrorKind } from "./types";
import { FALLBACK_SYSTEM_APP_ERROR, isKnownAppErrorCode } from "./registry";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isAppErrorKind(value: unknown): value is AppErrorKind {
  return value === "user" || value === "system";
}

function isValidSupportId(value: unknown): value is string | null | undefined {
  return value === undefined || value === null || typeof value === "string";
}

export function normalizeAppError(error: unknown): AppError {
  if (!isRecord(error)) {
    return FALLBACK_SYSTEM_APP_ERROR;
  }

  const { code, message, kind, support_id } = error;
  if (
    typeof code !== "string" ||
    typeof message !== "string" ||
    !isAppErrorKind(kind) ||
    !isValidSupportId(support_id) ||
    !isKnownAppErrorCode(code)
  ) {
    return FALLBACK_SYSTEM_APP_ERROR;
  }

  return {
    code,
    message,
    kind,
    support_id: support_id ?? null,
  };
}
