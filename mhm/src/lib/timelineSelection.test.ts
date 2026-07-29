import { describe, expect, it } from "vitest";
import { addDaysIso, localDateIso, resolveSelection } from "./timelineSelection";

const days = Array.from({ length: 16 }, (_, i) => ({
    // cột 0 = 26/07, cột 3 = hôm nay 29/07 (khớp layout thật: today − 3)
    fullDate: addDaysIso("2026-07-26", i),
}));
const todayKey = "2026-07-29";

describe("resolveSelection", () => {
    it("bấm 1 ô hôm nay → check-in 1 đêm", () => {
        const r = resolveSelection({ roomId: "1A", startIndex: 3, endIndex: 3 }, days, todayKey);
        expect(r).toEqual({
            kind: "checkin", roomId: "1A", checkInDate: "2026-07-29",
            checkOutDate: "2026-07-30", nights: 1, stillStaying: false,
        });
    });

    it("kéo 3 ô tương lai → reservation 3 đêm", () => {
        const r = resolveSelection({ roomId: "2B", startIndex: 5, endIndex: 7 }, days, todayKey);
        expect(r.kind).toBe("reservation");
        expect(r.checkInDate).toBe("2026-07-31");
        expect(r.checkOutDate).toBe("2026-08-03");
        expect(r.nights).toBe(3);
    });

    it("kéo ngược (phải sang trái) chuẩn hoá lại", () => {
        const r = resolveSelection({ roomId: "2B", startIndex: 7, endIndex: 5 }, days, todayKey);
        expect(r.checkInDate).toBe("2026-07-31");
        expect(r.nights).toBe(3);
    });

    it("ô quá khứ, kết thúc trước hôm nay → backfill đã trả", () => {
        const r = resolveSelection({ roomId: "3B", startIndex: 0, endIndex: 1 }, days, todayKey);
        expect(r.kind).toBe("backfill");
        expect(r.checkInDate).toBe("2026-07-26");
        expect(r.checkOutDate).toBe("2026-07-28");
        expect(r.stillStaying).toBe(false);
    });

    it("kéo từ quá khứ vắt qua hôm nay → backfill khách còn ở", () => {
        const r = resolveSelection({ roomId: "3B", startIndex: 1, endIndex: 4 }, days, todayKey);
        expect(r.kind).toBe("backfill");
        expect(r.checkInDate).toBe("2026-07-27");
        expect(r.checkOutDate).toBe("2026-07-31");
        expect(r.nights).toBe(4);
        expect(r.stillStaying).toBe(true);
    });
});

describe("date helpers", () => {
    it("addDaysIso vượt tháng", () => {
        expect(addDaysIso("2026-07-30", 3)).toBe("2026-08-02");
        expect(addDaysIso("2026-08-02", -3)).toBe("2026-07-30");
    });
    it("localDateIso định dạng YYYY-MM-DD", () => {
        expect(localDateIso(new Date(2026, 6, 29))).toBe("2026-07-29");
    });
});
