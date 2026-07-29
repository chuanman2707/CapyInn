import { useMemo } from "react";

import { useStayPrices } from "./useStayPrices";
import type { PricingResult } from "@/types";

/**
 * One room's price, asked of the backend rather than guessed.
 *
 * A thin wrapper over {@link useStayPrices}; see that module for why the quote
 * exists at all and why it is keyed on room type.
 */
interface UseStayPriceOptions {
    roomType: string | undefined;
    nights: number;
    /** A `YYYY-MM-DD` in the future makes this a reservation quote. */
    checkInDate?: string;
    /** Check-in derives this itself; `"nightly"` is what it defaults to. */
    pricingType?: string;
    disabled?: boolean;
}

export function useStayPrice({
    roomType,
    nights,
    checkInDate,
    pricingType = "nightly",
    disabled = false,
}: UseStayPriceOptions): { price: PricingResult | null; loading: boolean; failed: boolean } {
    const roomTypes = useMemo(() => (roomType ? [roomType] : []), [roomType]);

    const { byType, loading, failed } = useStayPrices({
        roomTypes,
        nights,
        checkInDate,
        pricingType,
        disabled: disabled || !roomType,
    });

    return {
        price: roomType ? byType[roomType] ?? null : null,
        loading,
        failed,
    };
}
