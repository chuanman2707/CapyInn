import { describe, expect, it } from "vitest";

import {
  assertMoneyVnd,
  assertNonNegativeMoneyVnd,
  optionalMoneyVnd,
} from "./money";
import { formatAppError } from "./appError";

describe("MoneyVnd helpers", () => {
  it("accepts safe integer VND", () => {
    expect(assertMoneyVnd(500000, "amount")).toBe(500000);
  });

  it("rejects fractional money", () => {
    expect(() => assertMoneyVnd(500000.5, "amount")).toThrow(/amount/);
  });

  it("rejects unsafe integer money", () => {
    expect(() =>
      assertMoneyVnd(Number.MAX_SAFE_INTEGER + 1, "amount"),
    ).toThrow(/safe integer/);
  });

  it("rejects non-finite money", () => {
    expect(() => assertMoneyVnd(Number.NaN, "amount")).toThrow(
      /safe integer/,
    );
    expect(() => assertMoneyVnd(Number.POSITIVE_INFINITY, "amount")).toThrow(
      /safe integer/,
    );
  });

  it("rejects negative money for non-negative fields", () => {
    expect(() => assertNonNegativeMoneyVnd(-1, "paid_amount")).toThrow(
      /paid_amount/,
    );
  });

  it("normalizes empty optional money to undefined", () => {
    expect(optionalMoneyVnd(null, "deposit_amount")).toBeUndefined();
    expect(optionalMoneyVnd(undefined, "deposit_amount")).toBeUndefined();
  });

  it("throws user-facing app errors", () => {
    try {
      assertMoneyVnd(500000.5, "amount");
      throw new Error("expected validation to fail");
    } catch (error) {
      expect(formatAppError(error)).toBe(
        "amount must be a safe integer VND value",
      );
    }
  });
});
