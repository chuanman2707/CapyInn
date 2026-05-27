import type { AppError, NormalizedAppErrorException } from "./types";

export function createAppErrorException(
  appError: AppError,
  cause?: unknown,
  options?: { correlation_id?: string | null },
): NormalizedAppErrorException {
  const error = new Error(appError.message) as NormalizedAppErrorException;
  error.name = "AppError";
  error.code = appError.code;
  error.message = appError.message;
  error.kind = appError.kind;
  error.support_id = appError.support_id;
  if (options?.correlation_id !== undefined) {
    error.correlation_id = options.correlation_id;
  }
  if (cause !== undefined) {
    error.cause = cause;
  }
  return error;
}
