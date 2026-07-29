import { describe, expect, it } from "vitest";

import { groupSpecialDates, overlappingDates, type SpecialDateRow } from "./specialDateRanges";

function row(date: string, label = "Tết", uplift_pct = 40): SpecialDateRow {
    return { id: `id-${date}`, date, label, uplift_pct };
}

describe("groupSpecialDates", () => {
    it("gom chín ngày liền nhau thành một khoảng", () => {
        const rows = Array.from({ length: 9 }, (_, index) => row(`2026-02-${14 + index}`));

        const ranges = groupSpecialDates(rows);

        expect(ranges).toHaveLength(1);
        expect(ranges[0].from).toBe("2026-02-14");
        expect(ranges[0].to).toBe("2026-02-22");
        expect(ranges[0].days).toBe(9);
        expect(ranges[0].dates).toHaveLength(9);
    });

    it("tách khi hở một ngày", () => {
        const ranges = groupSpecialDates([row("2026-02-14"), row("2026-02-16")]);

        expect(ranges).toHaveLength(2);
        expect(ranges.map((range) => range.days)).toEqual([1, 1]);
    });

    it("tách khi liền ngày nhưng khác nhãn", () => {
        const ranges = groupSpecialDates([
            row("2026-02-14", "Tết"),
            row("2026-02-15", "Hè"),
        ]);

        expect(ranges).toHaveLength(2);
    });

    it("tách khi liền ngày, cùng nhãn, nhưng khác mức", () => {
        const ranges = groupSpecialDates([
            row("2026-02-14", "Tết", 40),
            row("2026-02-15", "Tết", 25),
        ]);

        expect(ranges).toHaveLength(2);
    });

    it("gom đúng dù đầu vào không theo thứ tự ngày", () => {
        const ranges = groupSpecialDates([
            row("2026-02-16"),
            row("2026-02-14"),
            row("2026-02-15"),
        ]);

        expect(ranges).toHaveLength(1);
        expect(ranges[0].dates).toEqual(["2026-02-14", "2026-02-15", "2026-02-16"]);
    });

    it("vắt qua ranh giới tháng", () => {
        const ranges = groupSpecialDates([row("2026-02-28"), row("2026-03-01")]);

        expect(ranges).toHaveLength(1);
        expect(ranges[0].days).toBe(2);
    });

    it("vắt qua ranh giới năm", () => {
        const ranges = groupSpecialDates([row("2026-12-31"), row("2027-01-01")]);

        expect(ranges).toHaveLength(1);
        expect(ranges[0].days).toBe(2);
    });

    it("đầu vào rỗng trả mảng rỗng", () => {
        expect(groupSpecialDates([])).toEqual([]);
    });

    it("bỏ qua ngày sai định dạng thay vì sập", () => {
        // Cột `date` không có ràng buộc CHECK dưới DB, nên rác là có thật.
        const ranges = groupSpecialDates([row("khong-phai-ngay"), row("2026-02-14")]);

        expect(ranges).toHaveLength(1);
        expect(ranges[0].from).toBe("2026-02-14");
    });
});

describe("overlappingDates", () => {
    it("chỉ trả những ngày đã khai nằm trong khoảng mới", () => {
        const rows = [row("2026-02-19"), row("2026-02-20"), row("2026-02-21")];

        const clashes = overlappingDates(rows, "2026-02-20", "2026-02-28");

        expect(clashes.map((clash) => clash.date)).toEqual(["2026-02-20", "2026-02-21"]);
    });

    it("không tính ngày của chính cụm đang sửa là trùng", () => {
        const rows = [row("2026-02-20"), row("2026-02-21")];

        const clashes = overlappingDates(rows, "2026-02-20", "2026-02-28", [
            "2026-02-20",
            "2026-02-21",
        ]);

        expect(clashes).toEqual([]);
    });
});
