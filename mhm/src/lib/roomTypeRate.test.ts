import { describe, expect, it } from "vitest";

import { basePriceIsUnused, basePriceRole, nightlyRateDisplay } from "./roomTypeRate";
import type { RoomTypeRate } from "@/types";

const rates: Record<string, RoomTypeRate> = {
    "Phòng Đôi": { room_type: "Phòng Đôi", nightly_rate: 640_000, configured: true },
    "Phòng Đơn": { room_type: "Phòng Đơn", nightly_rate: 420_000, configured: false },
};

describe("nightlyRateDisplay", () => {
    it("formats the rate configured for the room type", () => {
        const display = nightlyRateDisplay(rates, "Phòng Đôi");

        expect(display.text).toContain("640");
        expect(display.unknown).toBe(false);
        expect(display.derived).toBe(false);
    });

    it("flags a rate nobody configured, so a screen with room can say so", () => {
        expect(nightlyRateDisplay(rates, "Phòng Đơn").derived).toBe(true);
    });

    it("says it does not know rather than inventing a number when rates failed to load", () => {
        const display = nightlyRateDisplay(null, "Phòng Đôi");

        expect(display.text).toBe("—");
        expect(display.unknown).toBe(true);
    });

    it("says it does not know for a type the rate listing has no row for", () => {
        // Không rơi về 0đ: một loại phòng thiếu trong bảng giá là "chưa biết",
        // và 0đ đọc như phòng miễn phí.
        const display = nightlyRateDisplay(rates, "Phòng VIP");

        expect(display.text).toBe("—");
        expect(display.unknown).toBe(true);
    });

    it("shows a genuine zero as a price, not as unknown", () => {
        const free: Record<string, RoomTypeRate> = {
            Free: { room_type: "Free", nightly_rate: 0, configured: true },
        };

        const display = nightlyRateDisplay(free, "Free");

        expect(display.unknown).toBe(false);
        expect(display.text).not.toBe("—");
    });
});

describe("basePriceRole", () => {
    it("reports base_price as ignored, with the rate that wins instead", () => {
        const role = basePriceRole(rates, "Phòng Đôi");

        expect(role.kind).toBe("ignored");
        expect(role.kind === "ignored" && role.typeRateText).toContain("640");
    });

    it("reports base_price as deriving the type price when no rule is configured", () => {
        // Khác biệt quan trọng: ở trạng thái này số admin gõ *có* tác dụng —
        // nhưng chỉ với phòng có mã nhỏ nhất trong loại.
        expect(basePriceRole(rates, "Phòng Đơn").kind).toBe("derives-type-price");
    });

    it("claims nothing when the rates are unavailable", () => {
        expect(basePriceRole(null, "Phòng Đôi").kind).toBe("unknown");
        expect(basePriceRole(rates, "Phòng VIP").kind).toBe("unknown");
    });
});

describe("basePriceIsUnused", () => {
    it("is true when the type charges something other than the room's base price", () => {
        expect(basePriceIsUnused(rates, "Phòng Đôi", 300_000)).toBe(true);
    });

    it("is false when the room's base price is exactly what the type charges", () => {
        expect(basePriceIsUnused(rates, "Phòng Đôi", 640_000)).toBe(false);
    });

    it("stays quiet when the rates are unavailable", () => {
        // Cảnh báo đoán mò tệ hơn không cảnh báo: không biết giá loại phòng thì
        // không kết luận được số của phòng có được dùng hay không.
        expect(basePriceIsUnused(null, "Phòng Đôi", 300_000)).toBe(false);
        expect(basePriceIsUnused(rates, "Phòng VIP", 300_000)).toBe(false);
    });
});
