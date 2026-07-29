import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { localDayKey } from "@/lib/datetime";
import { stayQuoteWindow } from "@/lib/stayQuote";
import type { PricingResult } from "@/types";

/**
 * The price each room type will actually cost, asked of the backend.
 *
 * Both check-in sheets used to compute their totals in JavaScript as
 * `base_price × nights`. The backend charges through `pricing_rules` — the
 * configured nightly rate, the weekend uplift, and any special-date surcharge.
 * Those two numbers agree only for a hotel that has configured nothing and books
 * no holidays, and not even then: the derived fallback rule carries a 20% weekend
 * uplift of its own.
 *
 * Keyed on room *type*, not room id, because that is what the charge resolves to
 * as well — `load_stay_pricing_inputs_tx` turns the room id into its type and
 * then runs the same type-keyed lookup the preview does. So ten rooms of one type
 * cost one invoke, not ten.
 */

/**
 * How often to check whether the quotes have gone stale.
 *
 * A sheet stays open for as long as it takes to scan documents and type guest
 * details. Check-in stamps its own `Local::now()` at submit, so a walk-in quote
 * taken at 23:50 on a holiday is charged against the next day's rules if the
 * staff member submits at 00:05. Only the check-in *date* can move a nightly
 * price, so re-quoting when the local date turns keeps the invoke count at
 * roughly zero while closing that window.
 */
const STALE_CHECK_MS = 30_000;

interface UseStayPricesOptions {
    /** Distinct room types to quote. Duplicates cost nothing but are pointless. */
    roomTypes: string[];
    nights: number;
    /** A `YYYY-MM-DD` in the future makes these reservation quotes. */
    checkInDate?: string;
    pricingType?: string;
    disabled?: boolean;
}

export interface StayPrices {
    /** Empty until every requested type has answered. */
    byType: Record<string, PricingResult>;
    loading: boolean;
    /** At least one type could not be priced. No partial total is offered. */
    failed: boolean;
}

export function useStayPrices({
    roomTypes,
    nights,
    checkInDate,
    pricingType = "nightly",
    disabled = false,
}: UseStayPricesOptions): StayPrices {
    const [byType, setByType] = useState<Record<string, PricingResult>>({});
    const [loading, setLoading] = useState(false);
    const [failed, setFailed] = useState(false);
    const [dayKey, setDayKey] = useState(() => localDayKey(new Date()));
    const requestIdRef = useRef(0);

    // A stable key so a new array with the same contents does not re-fetch.
    //
    // Serialised, not delimiter-joined. `rooms.type` holds the room type's
    // display *name*, and the shipped ones are "Standard Room" and "Deluxe
    // Balcony" — joining on a space and splitting it back apart tore those into
    // "Standard", "Room", "Deluxe", "Balcony". Every fragment then priced
    // successfully, because an unknown type falls back to the house default
    // rather than erroring, so nothing failed and no total ever appeared.
    const typeKey = useMemo(
        () => JSON.stringify(Array.from(new Set(roomTypes)).sort()),
        [roomTypes],
    );

    const reset = useCallback(() => {
        requestIdRef.current += 1;
        setByType({});
        setLoading(false);
        setFailed(false);
    }, []);

    useEffect(() => {
        if (disabled) return;

        const timer = setInterval(() => setDayKey(localDayKey(new Date())), STALE_CHECK_MS);
        return () => clearInterval(timer);
    }, [disabled]);

    useEffect(() => {
        const types: string[] = JSON.parse(typeKey);
        if (disabled || types.length === 0 || nights <= 0) {
            reset();
            return;
        }

        const requestId = requestIdRef.current + 1;
        requestIdRef.current = requestId;
        let active = true;

        const { checkIn, checkOut } = stayQuoteWindow(nights, checkInDate);

        const run = async () => {
            setLoading(true);
            setFailed(false);
            try {
                const results = await Promise.all(
                    types.map((roomType) =>
                        invoke<PricingResult>("calculate_price_preview", {
                            roomType,
                            checkIn,
                            checkOut,
                            pricingType,
                        }).then((price) => [roomType, price] as const),
                    ),
                );
                if (active && requestIdRef.current === requestId) {
                    setByType(Object.fromEntries(results));
                }
            } catch {
                // Deliberately no fallback number, and no partial map. A total
                // that silently omits one room type is the same class of lie the
                // JavaScript multiplication was.
                if (active && requestIdRef.current === requestId) {
                    setByType({});
                    setFailed(true);
                }
            } finally {
                if (active && requestIdRef.current === requestId) {
                    setLoading(false);
                }
            }
        };

        void run();

        return () => {
            active = false;
        };
        // `dayKey` is not read inside the effect — it is the staleness trigger.
    }, [checkInDate, dayKey, disabled, nights, pricingType, reset, typeKey]);

    return { byType, loading, failed };
}

/**
 * Sums a set of rooms against quotes taken per type.
 *
 * `null` when any room's type has not been priced, so a caller cannot show a
 * total that quietly skips a room.
 */
export function sumRoomPrices(
    rooms: { type: string }[],
    byType: Record<string, PricingResult>,
): number | null {
    let total = 0;
    for (const room of rooms) {
        const price = byType[room.type];
        if (!price) return null;
        total += price.total;
    }
    return total;
}
