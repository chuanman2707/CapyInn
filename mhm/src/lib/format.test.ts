import { describe, it, expect } from "vitest";
import { fmtDateShort } from "./format";

describe("fmtDateShort", () => {
  it("formats a valid date string correctly", () => {
    // We use a specific date and format it. Note that the output might depend on the timezone,
    // but we can provide a date like "2023-05-15T00:00:00Z" and test its "vi-VN" formatted result.
    // In UTC it's 15/05/2023, let's use a local timezone independent approach if possible, or just a known string.
    // Given JS dates, let's just pass a simple date string like "2023-05-15"
    const dateStr = "2023-05-15";
    // For a local date string "2023-05-15" in local time
    const expected = new Date(dateStr).toLocaleDateString("vi-VN", {
      day: "2-digit",
      month: "2-digit",
      year: "numeric",
    });
    expect(fmtDateShort(dateStr)).toBe(expected);
  });

  it("returns the original value for an invalid date string", () => {
    const invalidDateStr = "invalid-date";
    expect(fmtDateShort(invalidDateStr)).toBe(invalidDateStr);
  });

  it("returns the original value for an empty string", () => {
    const emptyStr = "";
    expect(fmtDateShort(emptyStr)).toBe(emptyStr);
  });
});
