import errorRegistry from "../../../shared/error-codes.json";

import type { AppError, AppErrorRegistryEntry } from "./types";

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

export function getAppErrorDefinition(code: string): AppErrorRegistryEntry | undefined {
  return APP_ERROR_BY_CODE[code];
}

export function isKnownAppErrorCode(code: string): boolean {
  return Boolean(getAppErrorDefinition(code));
}
