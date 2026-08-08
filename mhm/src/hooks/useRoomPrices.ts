import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type { PricingResult } from "@/types";

/**
 * The batch form of {@link usePricePreview}: one quote per room, same command.
 *
 * A group check-in books many rooms at once, and hooks cannot be called in a
 * loop, so the single-room hook does not fit. It asks
 * `calculate_room_price_preview` per room rather than per room *type*, because
 * `rooms.extra_person_fee` is a per-room column — two rooms of one type may
 * legitimately surcharge an extra guest differently, and only the room-keyed
 * preview reads the room the guest is actually getting.
 */
interface UseRoomPricesOptions {
    roomIds: string[];
    checkIn: string;
    checkOut: string;
    /** `null` mirrors what the group charge passes, which is `None`. */
    guests: number | null;
    disabled?: boolean;
}

export function useRoomPrices({
    roomIds,
    checkIn,
    checkOut,
    guests,
    disabled = false,
}: UseRoomPricesOptions): {
    byRoomId: Record<string, PricingResult>;
    loading: boolean;
    failed: boolean;
} {
    const [byRoomId, setByRoomId] = useState<Record<string, PricingResult>>({});
    const [loading, setLoading] = useState(false);
    const [failed, setFailed] = useState(false);
    const requestIdRef = useRef(0);

    // `roomIds` is a fresh array on every render, so keying the effect on it
    // directly would refire forever. Serialised with JSON rather than joined on
    // a delimiter: room ids are operator-entered free text, and a delimiter that
    // appears inside one silently splits it into rooms that do not exist.
    const idKey = useMemo(
        () => JSON.stringify(Array.from(new Set(roomIds)).sort()),
        [roomIds],
    );

    useEffect(() => {
        const ids: string[] = JSON.parse(idKey);
        if (disabled || ids.length === 0 || !checkIn || !checkOut) {
            requestIdRef.current += 1;
            setByRoomId({});
            setLoading(false);
            setFailed(false);
            return;
        }

        const requestId = requestIdRef.current + 1;
        requestIdRef.current = requestId;
        let active = true;

        const run = async () => {
            setLoading(true);
            setFailed(false);
            try {
                const results = await Promise.all(
                    ids.map((roomId) =>
                        invoke<PricingResult>("calculate_room_price_preview", {
                            roomId,
                            checkIn,
                            checkOut,
                            pricingType: "nightly",
                            guests,
                        }).then((price) => [roomId, price] as const),
                    ),
                );
                if (active && requestIdRef.current === requestId) {
                    setByRoomId(Object.fromEntries(results));
                }
            } catch {
                // No fallback number and no partial map. A total that silently
                // omits one room is the same class of lie as multiplying
                // `base_price` in JavaScript.
                if (active && requestIdRef.current === requestId) {
                    setByRoomId({});
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
    }, [checkIn, checkOut, disabled, guests, idKey]);

    return { byRoomId, loading, failed };
}

/**
 * Adds up quotes for a set of rooms.
 *
 * `null` when any room has not been priced, so a caller cannot render a total
 * that quietly leaves a room out of it.
 */
export function sumRoomPrices(
    roomIds: string[],
    byRoomId: Record<string, PricingResult>,
): number | null {
    let total = 0;
    for (const roomId of roomIds) {
        const price = byRoomId[roomId];
        if (!price) return null;
        total += price.total;
    }
    return total;
}

/**
 * Same contract as {@link sumRoomPrices}, plus manual per-room override
 * prices — the group check-in "click the price, type a new one" flow.
 *
 * A room with an override never needs its engine quote: `override × nights`
 * stands in for it. A room WITHOUT an override still needs `byRoomId` to have
 * priced it, and if it has not, the whole total is `null` — same refusal as
 * `sumRoomPrices`. A `reduce` with `?? 0` here would silently count a still-
 * unpriced room as 0₫, which is the exact bug this function exists to avoid.
 */
export function sumRoomPricesWithOverrides(
    roomIds: string[],
    byRoomId: Record<string, PricingResult>,
    overridePerRoom: Record<string, number>,
    nights: number,
): number | null {
    let total = 0;
    for (const roomId of roomIds) {
        const override = overridePerRoom[roomId];
        if (override != null) {
            total += override * nights;
            continue;
        }
        const price = byRoomId[roomId];
        if (!price) return null;
        total += price.total;
    }
    return total;
}
