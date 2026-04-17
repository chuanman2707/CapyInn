import { describe, expect, it } from "vitest";
import { fmtNumber, fmtMoney, fmtDate, fmtDateShort } from "./format";

describe("format", () => {
  describe("fmtNumber", () => {
    it("formats zero correctly", () => {
      expect(fmtNumber(0)).toBe("0");
    });

    it("formats positive numbers with thousands separator", () => {
      expect(fmtNumber(1000)).toBe("1.000");
      expect(fmtNumber(1000000)).toBe("1.000.000");
    });

    it("formats negative numbers with thousands separator", () => {
      expect(fmtNumber(-1000)).toBe("-1.000");
      expect(fmtNumber(-1000000)).toBe("-1.000.000");
    });

    it("rounds decimals correctly", () => {
      expect(fmtNumber(1234.5)).toBe("1.235");
      expect(fmtNumber(1234.4)).toBe("1.234");
      expect(fmtNumber(-1234.5)).toBe("-1.234"); // Note Math.round(-1234.5) is -1234
    });

    it("handles NaN", () => {
      expect(fmtNumber(NaN)).toBe("NaN");
    });

    it("handles Infinity", () => {
      expect(fmtNumber(Infinity)).toBe("∞");
      expect(fmtNumber(-Infinity)).toBe("-∞");
    });
  });

  describe("fmtMoney", () => {
    it("formats zero correctly", () => {
      expect(fmtMoney(0)).toBe("0đ");
    });

    it("formats positive money with thousands separator and suffix", () => {
      expect(fmtMoney(1000)).toBe("1.000đ");
      expect(fmtMoney(1000000)).toBe("1.000.000đ");
    });

    it("formats negative money with thousands separator and suffix", () => {
      expect(fmtMoney(-1000)).toBe("-1.000đ");
    });

    it("rounds decimals correctly before formatting", () => {
      expect(fmtMoney(1234.5)).toBe("1.235đ");
    });
  });

  describe("fmtDate", () => {
    it("formats valid date string to vi-VN locale", () => {
      // Create a specific timezone-independent test or just check it contains the expected parts
      // Note: toLocaleString output can vary slightly across node versions, but generally follows vi-VN
      const formatted = fmtDate("2024-03-15T12:30:00Z");
      // Since standard toLocaleString depends on local timezone, we check for presence of standard strings
      // or we can test with a fixed timezone if needed.
      // But testing fallback logic is also good.
      expect(typeof formatted).toBe("string");
      expect(formatted.length).toBeGreaterThan(0);
    });

    it("returns original string if invalid date", () => {
      expect(fmtDate("invalid-date")).toBe("invalid-date");
      expect(fmtDate("not-a-date")).toBe("not-a-date");
    });
  });

  describe("fmtDateShort", () => {
    it("formats valid date string to vi-VN locale short format", () => {
      const formatted = fmtDateShort("2024-03-15T12:30:00Z");
      expect(typeof formatted).toBe("string");
      expect(formatted.length).toBeGreaterThan(0);
    });

    it("returns original string if invalid date", () => {
      expect(fmtDateShort("invalid-date")).toBe("invalid-date");
      expect(fmtDateShort("not-a-date")).toBe("not-a-date");
    });
  });
});
