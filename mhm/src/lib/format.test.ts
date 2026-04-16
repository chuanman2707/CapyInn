import { describe, it, expect } from "vitest";
import { fmtDateShort } from "./format";

describe("fmtDateShort", () => {
  it("formats valid date string to short date format in vi-VN locale", () => {
    // Note: Use local time string to prevent timezone related flakiness
    // e.g. "2023-10-15T12:00:00" -> "15/10/2023"
    const result = fmtDateShort("2023-10-15T12:00:00");
    expect(result).toMatch(/15\/10\/2023/);
  });

  it("returns the original string if the date is invalid", () => {
    const invalidInputs = ["invalid-date", "abc", "not a date"];
    invalidInputs.forEach((input) => {
      expect(fmtDateShort(input)).toBe(input);
    });
  });

  it("returns the original empty string if input is empty", () => {
    expect(fmtDateShort("")).toBe("");
  });
});
