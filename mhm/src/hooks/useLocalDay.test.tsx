import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useLocalDay } from "./useLocalDay";

/** Giờ địa phương (test chạy ở Asia/Ho_Chi_Minh, ghim trong vitest.config.ts). */
function localTime(iso: string) {
    return new Date(iso);
}

describe("useLocalDay", () => {
    beforeEach(() => {
        vi.useFakeTimers();
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    it("reports the local day, not the UTC day", () => {
        // 02:00 giờ Việt Nam ngày 21 = 19:00 UTC ngày 20. `toISOString()` sẽ trả
        // ngày 20 — đó là cả lý do hook này tồn tại.
        vi.setSystemTime(localTime("2026-04-21T02:00:00+07:00"));

        const { result } = renderHook(() => useLocalDay());

        expect(result.current).toBe("2026-04-21");
    });

    it("moves to the new day when local midnight passes", () => {
        vi.setSystemTime(localTime("2026-04-21T23:59:00+07:00"));

        const { result } = renderHook(() => useLocalDay());
        expect(result.current).toBe("2026-04-21");

        // Ca đêm: sheet mở từ trước nửa đêm, bấm nhận phòng sau nửa đêm.
        act(() => {
            vi.advanceTimersByTime(61_000 + 1_000);
        });

        expect(result.current).toBe("2026-04-22");
    });

    it("does not change before midnight, however long the form sits open", () => {
        vi.setSystemTime(localTime("2026-04-21T08:00:00+07:00"));

        const { result } = renderHook(() => useLocalDay());

        act(() => {
            vi.advanceTimersByTime(10 * 60 * 60 * 1_000);
        });

        // Không phải hook hết hạn theo thời gian — nó chỉ đổi khi *ngày* đổi.
        expect(result.current).toBe("2026-04-21");
    });

    it("keeps ticking across a second midnight", () => {
        vi.setSystemTime(localTime("2026-04-21T23:59:59+07:00"));

        const { result } = renderHook(() => useLocalDay());

        act(() => {
            vi.advanceTimersByTime(2_000);
        });
        expect(result.current).toBe("2026-04-22");

        // Hẹn lại được, chứ không phải chỉ bắn một lần: một máy để mở nhiều ngày
        // vẫn phải đúng ngày.
        act(() => {
            vi.advanceTimersByTime(24 * 60 * 60 * 1_000);
        });
        expect(result.current).toBe("2026-04-23");
    });

    it("stops its timer when unmounted", () => {
        vi.setSystemTime(localTime("2026-04-21T23:59:00+07:00"));

        const { unmount } = renderHook(() => useLocalDay());
        unmount();

        // Cập nhật state sau khi unmount là một cảnh báo React và một rò rỉ;
        // không có timer nào còn treo thì `getTimerCount()` phải về 0.
        expect(vi.getTimerCount()).toBe(0);
    });
});
