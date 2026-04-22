import errorRegistry from "../../shared/error-codes.json";

export type AppErrorKind = "user" | "system";

export interface AppError {
  code: string;
  message: string;
  kind: AppErrorKind;
  support_id: string | null;
}

export interface AppErrorRegistryEntry {
  code: string;
  kind: AppErrorKind;
  defaultMessage: string;
}

const LAST_RESORT_FALLBACK_ERROR_MESSAGE = "Có lỗi hệ thống, vui lòng thử lại";

const registryEntries = errorRegistry as AppErrorRegistryEntry[];

export const APP_ERROR_REGISTRY = Object.freeze(
  registryEntries.map((entry) => ({ ...entry })),
) as readonly AppErrorRegistryEntry[];

export const APP_ERROR_CODE_MAP = Object.freeze(
  APP_ERROR_REGISTRY.reduce(
    (codes, entry) => {
      codes[entry.code] = entry.code;
      return codes;
    },
    {} as Record<string, string>,
  ),
);

export const APP_ERROR_CODES = APP_ERROR_CODE_MAP;

export const SYSTEM_INTERNAL_ERROR = "SYSTEM_INTERNAL_ERROR";

const systemInternalErrorDefinition = APP_ERROR_REGISTRY.find(
  (entry) => entry.code === SYSTEM_INTERNAL_ERROR,
);

const FALLBACK_ERROR_MESSAGE =
  systemInternalErrorDefinition?.defaultMessage ?? LAST_RESORT_FALLBACK_ERROR_MESSAGE;

export const FALLBACK_SYSTEM_APP_ERROR: AppError = Object.freeze({
  code: SYSTEM_INTERNAL_ERROR,
  message: FALLBACK_ERROR_MESSAGE,
  kind: "system",
  support_id: null,
});

const APP_ERROR_BY_CODE = Object.freeze(
  APP_ERROR_REGISTRY.reduce(
    (definitions, entry) => {
      definitions[entry.code] = entry;
      return definitions;
    },
    {} as Record<string, AppErrorRegistryEntry>,
  ),
);

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isAppErrorKind(value: unknown): value is AppErrorKind {
  return value === "user" || value === "system";
}

function isValidSupportId(value: unknown): value is string | null | undefined {
  return value === undefined || value === null || typeof value === "string";
}

export function getAppErrorDefinition(code: string): AppErrorRegistryEntry | undefined {
  return APP_ERROR_BY_CODE[code];
}

export function isKnownAppErrorCode(code: string): boolean {
  return Boolean(getAppErrorDefinition(code));
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

function getLocalCorrelationId(error: unknown): string | null {
  if (!isRecord(error)) {
    return null;
  }

  const { correlation_id } = error;
  return typeof correlation_id === "string" ? correlation_id : null;
}

export function formatAppError(error: unknown): string {
  const normalized = normalizeAppError(error);
  const localCorrelationId = getLocalCorrelationId(error);
  const trackingLine = localCorrelationId ? `\nMã theo dõi: ${localCorrelationId}` : "";

  if (normalized.kind === "user") {
    return `${normalized.message}${trackingLine}`;
  }

  if (normalized.support_id) {
    return `${normalized.message} (Mã hỗ trợ: ${normalized.support_id})${trackingLine}`;
  }

  return `${normalized.message}${trackingLine}`;
}

export type NormalizedAppErrorException = Error &
  AppError & { correlation_id?: string | null; cause?: unknown };

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
