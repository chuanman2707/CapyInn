import { describe, expect, it } from "vitest";

import { BALANCE_TONE_CLASS, balanceDisplay, bookingBalance } from "./bookingBalance";

describe("bookingBalance", () => {
    it("reports what is still owed", () => {
        expect(bookingBalance(1_200_000, 500_000)).toEqual({ kind: "owed", amount: 700_000 });
    });

    it("reports a settled booking as settled, not as owing zero", () => {
        // Khác nhau ở nhãn: "Đã đủ" và "Còn nợ 0đ" không đọc giống nhau ở quầy.
        expect(bookingBalance(1_200_000, 1_200_000)).toEqual({ kind: "settled" });
    });

    it("reports an overpayment as a refund owed to the guest", () => {
        expect(bookingBalance(1_200_000, 5_000_000)).toEqual({
            kind: "overpaid",
            refund: 3_800_000,
        });
    });

    it("counts a refund as positive, never as a negative debt", () => {
        const balance = bookingBalance(1_200_000, 5_000_000);

        // Cả hai màn hình cũ đều hiện −3.800.000đ. Con số có nghĩa là 3.800.000đ
        // phải trả lại khách; dấu trừ thuộc về nhãn, không thuộc về số tiền.
        expect(balance.kind === "overpaid" && balance.refund).toBeGreaterThan(0);
    });

    it("treats a zero-total booking with a payment as overpaid", () => {
        expect(bookingBalance(0, 100_000)).toEqual({ kind: "overpaid", refund: 100_000 });
    });
});

describe("balanceDisplay", () => {
    it("labels an overpayment as money going back to the guest", () => {
        const display = balanceDisplay(1_200_000, 5_000_000);

        expect(display.label).toBe("Trả lại khách");
        expect(display.text).toContain("3.800.000");
        expect(display.text).not.toContain("-");
        expect(display.tone).toBe("overpaid");
    });

    it("labels a debt as a debt", () => {
        const display = balanceDisplay(1_200_000, 500_000);

        expect(display.label).toBe("Còn nợ");
        expect(display.text).toContain("700.000");
        expect(display.tone).toBe("owed");
    });

    it("does not paint an overpayment the same colour as a debt", () => {
        // Đỏ ở đây là sai nghĩa: không ai đang nợ tiền nhà.
        expect(BALANCE_TONE_CLASS.overpaid).not.toBe(BALANCE_TONE_CLASS.owed);
        expect(BALANCE_TONE_CLASS.overpaid).not.toBe(BALANCE_TONE_CLASS.settled);
    });
});
