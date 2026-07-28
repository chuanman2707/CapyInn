import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type { PricingResult } from "@/types";

interface UsePricePreviewOptions {
    roomId: string;
    checkIn: string;
    checkOut: string;
    guests: number;
    debounceMs?: number;
}

/// Con số hiển thị phải do engine trả về, không phải do giao diện tự nhân —
/// đó là cách duy nhất để số trên màn hình bằng số ghi vào sổ.
export function usePricePreview({
    roomId,
    checkIn,
    checkOut,
    guests,
    debounceMs = 0,
}: UsePricePreviewOptions) {
    const [preview, setPreview] = useState<PricingResult | null>(null);
    const [loading, setLoading] = useState(false);
    // Simple boolean, no error taxonomy — the box only needs to know whether
    // the last lookup failed so it can stop rendering an empty/misleading box.
    const [error, setError] = useState(false);
    const requestIdRef = useRef(0);

    useEffect(() => {
        if (!roomId || !checkIn || !checkOut) {
            requestIdRef.current += 1;
            setPreview(null);
            setLoading(false);
            setError(false);
            return;
        }

        const requestId = requestIdRef.current + 1;
        requestIdRef.current = requestId;
        let active = true;

        const run = async () => {
            setLoading(true);
            setError(false);
            try {
                const result = await invoke<PricingResult>("calculate_room_price_preview", {
                    roomId,
                    checkIn,
                    checkOut,
                    pricingType: "nightly",
                    guests,
                });
                if (active && requestIdRef.current === requestId) {
                    setPreview(result);
                }
            } catch {
                if (active && requestIdRef.current === requestId) {
                    setPreview(null);
                    setError(true);
                }
            } finally {
                if (active && requestIdRef.current === requestId) {
                    setLoading(false);
                }
            }
        };

        const timer = debounceMs > 0 ? window.setTimeout(run, debounceMs) : null;
        if (timer == null) {
            void run();
        }

        return () => {
            active = false;
            if (timer != null) {
                clearTimeout(timer);
            }
        };
    }, [checkIn, checkOut, debounceMs, guests, roomId]);

    return { preview, loading, error };
}
