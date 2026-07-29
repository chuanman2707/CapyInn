import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import { addDays, localDayKey, localRfc3339 } from "@/lib/datetime";
import type { PricingResult } from "@/types";

/**
 * How often to check whether the quote has gone stale.
 *
 * The sheet stays open for as long as it takes to scan documents and type guest
 * details. Check-in stamps its own `Local::now()` at submit, so a quote taken at
 * 23:50 on a holiday is charged against the next day's rules if the staff member
 * submits at 00:05. Re-quoting only when the local date turns keeps the invoke
 * count at roughly zero while closing that window.
 */
const STALE_CHECK_MS = 30_000;

/**
 * The price a stay will actually cost, asked of the backend rather than guessed.
 *
 * The check-in sheet used to show `base_price × nights`, computed in JavaScript.
 * The backend charges from `pricing_rules` — a configured nightly rate, plus the
 * weekend uplift, plus any special-date surcharge. Those two numbers agree only
 * for a hotel that has configured nothing and books no holidays. Everywhere else
 * the sheet quoted one figure and the folio recorded another, and the staff
 * member collecting a deposit read the wrong one.
 *
 * `calculate_price_preview` exists precisely to answer this and had no caller.
 * `the_preview_and_the_lifecycle_charge_agree_on_every_reachable_rule_source`
 * (Rust) is what pins the two to the same answer.
 */
interface UseStayPriceOptions {
    roomType: string | undefined;
    nights: number;
    /** Check-in derives this itself; `"nightly"` is what it defaults to. */
    pricingType?: string;
    disabled?: boolean;
}

export function useStayPrice({
    roomType,
    nights,
    pricingType = "nightly",
    disabled = false,
}: UseStayPriceOptions) {
    const [price, setPrice] = useState<PricingResult | null>(null);
    const [loading, setLoading] = useState(false);
    const [failed, setFailed] = useState(false);
    const [dayKey, setDayKey] = useState(() => localDayKey(new Date()));
    const requestIdRef = useRef(0);

    useEffect(() => {
        if (disabled) return;

        const timer = setInterval(() => setDayKey(localDayKey(new Date())), STALE_CHECK_MS);
        return () => clearInterval(timer);
    }, [disabled]);

    const reset = useCallback(() => {
        requestIdRef.current += 1;
        setPrice(null);
        setLoading(false);
        setFailed(false);
    }, []);

    useEffect(() => {
        if (disabled || !roomType || nights <= 0) {
            reset();
            return;
        }

        const requestId = requestIdRef.current + 1;
        requestIdRef.current = requestId;
        let active = true;

        // Check-in stamps `Local::now()` and adds `nights` days for the expected
        // checkout. Quoting from the same two instants is the whole point.
        const now = new Date();
        const checkIn = localRfc3339(now);
        const checkOut = localRfc3339(addDays(now, nights));

        const run = async () => {
            setLoading(true);
            setFailed(false);
            try {
                const result = await invoke<PricingResult>("calculate_price_preview", {
                    roomType,
                    checkIn,
                    checkOut,
                    pricingType,
                });
                if (active && requestIdRef.current === requestId) {
                    setPrice(result);
                }
            } catch {
                // Deliberately no fallback number. A wrong total shown with
                // confidence is worse than an honest blank — that was the bug.
                if (active && requestIdRef.current === requestId) {
                    setPrice(null);
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
    }, [dayKey, disabled, nights, pricingType, reset, roomType]);

    return { price, loading, failed, reset };
}
