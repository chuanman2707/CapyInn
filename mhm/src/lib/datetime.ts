/**
 * Datetime strings in the shape the Rust side produces.
 *
 * The backend stamps `Local::now().to_rfc3339()`, which carries the machine's
 * UTC offset. `Date.prototype.toISOString()` does not — it converts to UTC and
 * appends `Z`. That difference is invisible most of the day and wrong near
 * midnight: at 00:30 on the 21st in Vietnam, `toISOString()` yields the 20th.
 * Pricing truncates the first ten characters to look up `special_dates`, so a
 * UTC string asks about the wrong day's holiday surcharge.
 */

function pad(value: number): string {
    return String(value).padStart(2, "0");
}

/**
 * `YYYY-MM-DDTHH:mm:ss±HH:MM` in the machine's own timezone.
 *
 * Matches what `chrono::Local::now().to_rfc3339()` writes, minus sub-second
 * precision, which nothing in the pricing path reads.
 */
export function localRfc3339(date: Date): string {
    // getTimezoneOffset is minutes *behind* UTC, so the sign is inverted.
    const offsetMinutes = -date.getTimezoneOffset();
    const sign = offsetMinutes < 0 ? "-" : "+";
    const absolute = Math.abs(offsetMinutes);

    return (
        `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}` +
        `T${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}` +
        `${sign}${pad(Math.floor(absolute / 60))}:${pad(absolute % 60)}`
    );
}

/** The same instant `nights` days later, which is how check-in derives its expected checkout. */
export function addDays(date: Date, days: number): Date {
    return new Date(date.getTime() + days * 86_400_000);
}

/**
 * The local `YYYY-MM-DD` a price depends on.
 *
 * Every input that can move a nightly price keys off the check-in *date*: the
 * `special_dates` lookup truncates to ten characters, and the weekend uplift
 * walks calendar days from it. The clock time within that day changes nothing.
 * So a quote only goes stale when this string changes.
 */
export function localDayKey(date: Date): string {
    return localRfc3339(date).slice(0, 10);
}
