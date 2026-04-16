import { describe, expect, it } from "vitest";
import { fmtDate, fmtDateShort, fmtNumber, fmtMoney } from "./format";

describe("format", () => {
  describe("fmtDate", () => {
    it("formats a valid ISO date string correctly", () => {
      // By using a local ISO string (no 'Z' suffix), parsing and outputing
      // relies on the same local timezone, making it stable across CI environments.
      expect(fmtDate("2024-03-15T19:30:00")).toBe("19:30 15/03/2024");
    });

    it("returns the original string if date is invalid", () => {
      expect(fmtDate("invalid-date")).toBe("invalid-date");
    });

    it("returns the original string if empty string is provided", () => {
      expect(fmtDate("")).toBe("");
    });
  });

  describe("fmtDateShort", () => {
    it("formats a valid ISO date string correctly", () => {
      expect(fmtDateShort("2024-03-15T19:30:00")).toBe("15/03/2024");
    });

    it("returns the original string if date is invalid", () => {
      expect(fmtDateShort("not a date")).toBe("not a date");
    });

    it("returns the original string if empty string is provided", () => {
      expect(fmtDateShort("")).toBe("");
    });
  });

  describe("fmtNumber", () => {
    it("formats numbers with vi-VN locale", () => {
      expect(fmtNumber(1000000)).toBe("1.000.000");
    });

    it("rounds numbers before formatting", () => {
      expect(fmtNumber(1000000.5)).toBe("1.000.001");
      expect(fmtNumber(1000000.4)).toBe("1.000.000");
    });
  });

  describe("fmtMoney", () => {
    it("formats numbers as money with vi-VN locale", () => {
      expect(fmtMoney(1500000)).toBe("1.500.000đ");
    });
  });
});
