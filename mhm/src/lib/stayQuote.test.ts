import { describe, expect, it } from "vitest";

import { localDayKey } from "./datetime";
import {
    addDaysToDateString,
    isReservationDate,
    localToday,
    stayQuoteWindow,
} from "./stayQuote";

/**
 * Nothing pins `TZ` for this suite and CI runs at UTC, so these assert
 * properties that hold at any offset. Where a case needs a specific "now", it is
 * passed in rather than read from the clock.
 */
describe("localToday", () => {
    it("is the local calendar day, which is what the backend compares against", () => {
        expect(localToday(new Date(2026, 3, 20, 23, 59, 59))).toBe("2026-04-20");
        expect(localToday(new Date(2026, 3, 21, 0, 0, 1))).toBe("2026-04-21");
    });
});

describe("isReservationDate", () => {
    const now = new Date(2026, 3, 20, 10, 0, 0);

    it("treats today as a walk-in even when the date was supplied explicitly", () => {
        // `group_lifecycle.rs` compares `date != today_str`, so an explicit
        // today is a walk-in there. Disagreeing here priced from a bare date
        // while the backend priced from the current instant.
        expect(isReservationDate("2026-04-20", now)).toBe(false);
    });

    it("treats any other date as a reservation", () => {
        expect(isReservationDate("2026-04-21", now)).toBe(true);
        expect(isReservationDate("2026-04-19", now)).toBe(true);
    });
});

describe("addDaysToDateString", () => {
    it("stays in bare-date space", () => {
        expect(addDaysToDateString("2026-04-20", 1)).toBe("2026-04-21");
        expect(addDaysToDateString("2026-04-20", 3)).toBe("2026-04-23");
    });

    it("crosses month and year boundaries", () => {
        expect(addDaysToDateString("2026-04-29", 3)).toBe("2026-05-02");
        expect(addDaysToDateString("2026-12-30", 3)).toBe("2027-01-02");
    });

    it("handles a leap day", () => {
        expect(addDaysToDateString("2028-02-28", 1)).toBe("2028-02-29");
        expect(addDaysToDateString("2028-02-28", 2)).toBe("2028-03-01");
    });

    it("treats zero as no move", () => {
        expect(addDaysToDateString("2026-04-20", 0)).toBe("2026-04-20");
    });
});

describe("stayQuoteWindow", () => {
    const now = new Date(2026, 3, 20, 14, 30, 0);

    it("prices a walk-in from the current instant, with an offset", () => {
        const window = stayQuoteWindow(2, undefined, now);

        expect(window.checkIn).toMatch(/^2026-04-20T14:30:00[+-]\d{2}:\d{2}$/);
        expect(window.checkOut).toMatch(/^2026-04-22T14:30:00[+-]\d{2}:\d{2}$/);
    });

    it("prices a reservation from bare dates, which is what the backend passes", () => {
        // `group_lifecycle.rs` hands `checkin_date` / `checkout_date` — the bare
        // strings — into pricing, not the `14:00` timestamps beside them.
        expect(stayQuoteWindow(2, "2026-04-25", now)).toEqual({
            checkIn: "2026-04-25",
            checkOut: "2026-04-27",
        });
    });

    it("treats an explicit today as a walk-in, not a reservation", () => {
        const window = stayQuoteWindow(1, "2026-04-20", now);

        expect(window.checkIn).toContain("T14:30:00");
        expect(window.checkOut).not.toBe("2026-04-21");
    });

    it("spans exactly the nights asked for, in both modes", () => {
        const walkIn = stayQuoteWindow(3, undefined, now);
        expect(Date.parse(walkIn.checkOut) - Date.parse(walkIn.checkIn)).toBe(3 * 86_400_000);

        // Hardcoded, not `addDaysToDateString(reserved.checkIn, 3)` — that is
        // the implementation, so asserting it proves only that the code equals
        // itself. An off-by-one inside the helper would sail through.
        const reserved = stayQuoteWindow(3, "2026-04-25", now);
        expect(reserved).toEqual({ checkIn: "2026-04-25", checkOut: "2026-04-28" });
    });

    it("never emits a UTC-converted day", () => {
        const window = stayQuoteWindow(1, undefined, now);

        for (const stamp of [window.checkIn, window.checkOut]) {
            expect(stamp).not.toContain("Z");
        }
        // The check-in day is what `special_dates` is looked up by. A
        // `toISOString()` stamp would report the 19th or the 21st here,
        // depending on which side of UTC the machine sits.
        expect(window.checkIn.slice(0, 10)).toBe(localDayKey(now));
        expect(window.checkOut.slice(0, 10)).toBe("2026-04-21");
    });
});
