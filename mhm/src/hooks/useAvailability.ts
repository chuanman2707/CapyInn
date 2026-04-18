import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type { AvailabilityResult } from "@/types";

interface UseAvailabilityOptions {
    roomId: string;
    fromDate: string;
    toDate: string;
    disabled?: boolean;
    debounceMs?: number;
}

export function useAvailability({
    roomId,
    fromDate,
    toDate,
    disabled = false,
    debounceMs = 0,
}: UseAvailabilityOptions) {
    const [availability, setAvailability] = useState<AvailabilityResult | null>(null);
    const [loading, setLoading] = useState(false);
    const requestIdRef = useRef(0);

    const reset = useCallback(() => {
        requestIdRef.current += 1;
        setAvailability(null);
        setLoading(false);
    }, []);

    useEffect(() => {
        if (disabled || !roomId || !fromDate || !toDate) {
            reset();
            return;
        }

        const requestId = requestIdRef.current + 1;
        requestIdRef.current = requestId;
        let active = true;

        const run = async () => {
            setLoading(true);
            try {
                const result = await invoke<AvailabilityResult>("check_availability", {
                    roomId,
                    fromDate,
                    toDate,
                });
                if (active && requestIdRef.current === requestId) {
                    setAvailability(result);
                }
            } catch {
                if (active && requestIdRef.current === requestId) {
                    setAvailability(null);
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
    }, [debounceMs, disabled, fromDate, reset, roomId, toDate]);

    return {
        availability,
        loading,
        reset,
    };
}
