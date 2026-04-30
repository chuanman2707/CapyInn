import type { AppError } from "./appError";

export type MoneyVnd = number;

function moneyValidationError(message: string): Error & AppError {
  return Object.assign(new Error(message), {
    code: "VALIDATION_INVALID_INPUT",
    message,
    kind: "user" as const,
    support_id: null,
  });
}

export function assertMoneyVnd(value: number, field: string): MoneyVnd {
  if (!Number.isInteger(value) || !Number.isSafeInteger(value)) {
    throw moneyValidationError(`${field} must be a safe integer VND value`);
  }
  return value;
}

export function assertNonNegativeMoneyVnd(
  value: number,
  field: string,
): MoneyVnd {
  const checked = assertMoneyVnd(value, field);
  if (checked < 0) {
    throw moneyValidationError(`${field} must be greater than or equal to 0`);
  }
  return checked;
}

export function optionalMoneyVnd(
  value: number | null | undefined,
  field: string,
): MoneyVnd | undefined {
  if (value === null || value === undefined) {
    return undefined;
  }
  return assertNonNegativeMoneyVnd(value, field);
}
