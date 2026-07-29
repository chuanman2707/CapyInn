import { describe, expect, it } from "vitest";

import { addDays, localDayKey, localRfc3339 } from "./datetime";

/**
 * Nothing pins `TZ` for this suite, and CI runs at UTC while the product runs at
 * UTC+07:00. So these assert properties that hold in *every* timezone rather
 * than picking a moment that only straddles midnight at one offset — a test that
 * silently stops discriminating on the CI runner is worse than no test.
 */
describe("localRfc3339", () => {
    it("writes an offset the Rust side can parse, never a bare Z", () => {
        const stamped = localRfc3339(new Date(2026, 3, 20, 14, 5, 9));

        expect(stamped).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}[+-]\d{2}:\d{2}$/);
        expect(stamped).not.toContain("Z");
    });

    it("keeps the wall clock it was given, in every timezone", () => {
        // These are local-time constructor arguments. `toISOString()` converts to
        // UTC and would return different digits everywhere except UTC itself —
        // that conversion is what mispriced a holiday check-in near midnight.
        for (const [year, month, day, hour, minute, second, expected] of [
            [2026, 3, 21, 0, 30, 0, "2026-04-21T00:30:00"],
            [2026, 3, 20, 23, 30, 0, "2026-04-20T23:30:00"],
            [2026, 0, 1, 0, 0, 0, "2026-01-01T00:00:00"],
            [2026, 11, 31, 23, 59, 59, "2026-12-31T23:59:59"],
        ] as const) {
            const stamped = localRfc3339(new Date(year, month, day, hour, minute, second));
            expect(stamped.slice(0, 19)).toBe(expected);
        }
    });

    it("disagrees with toISOString exactly when the machine is not on UTC", () => {
        const nearMidnight = new Date(2026, 3, 21, 0, 30, 0);
        const local = localRfc3339(nearMidnight).slice(0, 19);
        const utc = nearMidnight.toISOString().slice(0, 19);

        if (nearMidnight.getTimezoneOffset() === 0) {
            expect(local).toBe(utc);
        } else {
            expect(local).not.toBe(utc);
        }
    });
});

describe("addDays", () => {
    it("advances by whole days, not hours", () => {
        const twoNightsOn = addDays(new Date(2026, 3, 20, 14, 0, 0), 2);

        expect(localRfc3339(twoNightsOn).slice(0, 19)).toBe("2026-04-22T14:00:00");
    });

    it("crosses a month boundary", () => {
        expect(localDayKey(addDays(new Date(2026, 3, 29, 14, 0, 0), 3))).toBe("2026-05-02");
    });

    it("treats zero as no move", () => {
        const start = new Date(2026, 3, 20, 14, 0, 0);

        expect(addDays(start, 0).getTime()).toBe(start.getTime());
    });
});

describe("localDayKey", () => {
    it("is the local calendar day, and ignores the time within it", () => {
        const day = "2026-04-20";

        expect(localDayKey(new Date(2026, 3, 20, 0, 0, 0))).toBe(day);
        expect(localDayKey(new Date(2026, 3, 20, 12, 0, 0))).toBe(day);
        expect(localDayKey(new Date(2026, 3, 20, 23, 59, 59))).toBe(day);
    });

    it("turns over at local midnight, which is when a quote goes stale", () => {
        expect(localDayKey(new Date(2026, 3, 20, 23, 59, 59))).toBe("2026-04-20");
        expect(localDayKey(new Date(2026, 3, 21, 0, 0, 0))).toBe("2026-04-21");
    });
});
