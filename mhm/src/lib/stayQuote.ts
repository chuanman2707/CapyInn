/**
 * The two timestamps a stay is priced between.
 *
 * These must mirror what the backend derives, or the quote answers a different
 * question from the charge. Both check-in paths pick their window the same way:
 *
 * - **walk-in** — `Local::now()` and `now + nights days`, as RFC3339
 *   (`stay_lifecycle.rs` `check_in_tx`, `group_lifecycle.rs` `group_check_in_tx`).
 * - **reservation** — the bare `YYYY-MM-DD` check-in date and that date plus
 *   `nights` days, also bare. Group reservations pass the date strings straight
 *   into pricing rather than the composed `14:00` timestamps sitting beside them.
 *
 * A reservation is one whose date is not today. "Today" is a *local* calendar
 * day on the Rust side (`now.format("%Y-%m-%d")`), which is why nothing here may
 * reach for `toISOString()`.
 */

import { addDays, localDayKey, localRfc3339 } from "./datetime";

export interface StayQuoteWindow {
    checkIn: string;
    checkOut: string;
}

/** Today, as the backend spells it. */
export function localToday(now: Date = new Date()): string {
    return localDayKey(now);
}

/**
 * Whether a check-in date makes this a reservation rather than a walk-in.
 *
 * Mirrors `group_lifecycle.rs`: a date equal to today is *not* a reservation,
 * even though it was supplied explicitly.
 */
export function isReservationDate(checkInDate: string, now: Date = new Date()): boolean {
    return checkInDate !== localToday(now);
}

/** Adds whole days to a bare `YYYY-MM-DD`, staying in bare-date space. */
export function addDaysToDateString(date: string, days: number): string {
    const [year, month, day] = date.split("-").map(Number);
    // Local-noon anchor: far enough from either midnight that a DST shift cannot
    // move the calendar day out from under the arithmetic.
    return localDayKey(addDays(new Date(year, month - 1, day, 12), days));
}

/**
 * `checkInDate` undefined — or equal to today — means a walk-in priced from the
 * current instant. Anything else is a reservation priced from bare dates.
 */
export function stayQuoteWindow(
    nights: number,
    checkInDate?: string,
    now: Date = new Date(),
): StayQuoteWindow {
    if (checkInDate && isReservationDate(checkInDate, now)) {
        return {
            checkIn: checkInDate,
            checkOut: addDaysToDateString(checkInDate, nights),
        };
    }

    return {
        checkIn: localRfc3339(now),
        checkOut: localRfc3339(addDays(now, nights)),
    };
}
