import { describe, expect, it } from "vitest";

import { nightlyRateDisplay } from "./roomTypeRate";
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
