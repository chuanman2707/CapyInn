export interface TimelineSelectionRange {
    roomId: string;
    startIndex: number;
    endIndex: number;
}

export type SelectionKind = "checkin" | "reservation" | "backfill";

export interface ResolvedSelection {
    kind: SelectionKind;
    roomId: string;
    checkInDate: string;
    checkOutDate: string;
    nights: number;
    stillStaying: boolean;
}

export function localDateIso(d: Date): string {
    const mm = String(d.getMonth() + 1).padStart(2, "0");
    const dd = String(d.getDate()).padStart(2, "0");
    return `${d.getFullYear()}-${mm}-${dd}`;
}

export function addDaysIso(date: string, days: number): string {
    const [y, m, d] = date.split("-").map(Number);
    return localDateIso(new Date(y, m - 1, d + days));
}

// Ngày dạng "YYYY-MM-DD" (không giờ) được JS phân giải theo UTC — dùng nguyên
// dạng đó (không thêm "T00:00:00") để tránh lệch ngày ở múi giờ dương như
// Asia/Ho_Chi_Minh khi quy đổi qua toISOString(). Dùng chung cho
// BackfillSheet.tsx và ReservationSheet.tsx — trước đây mỗi nơi tự giữ một
// bản sao y hệt, giờ chỉ còn một chỗ để sửa nếu logic múi giờ đổi.
export function nightsBetween(checkIn: string, checkOut: string): number {
    if (!checkIn || !checkOut) return 0;
    const ms = new Date(checkOut).getTime() - new Date(checkIn).getTime();
    if (Number.isNaN(ms)) return 0;
    return Math.round(ms / 86_400_000);
}

/** Mỗi ô được chọn = một đêm ở; ngày ra = ngày bắt đầu + số ô. */
export function resolveSelection(
    range: TimelineSelectionRange,
    days: { fullDate: string }[],
    todayKey: string,
): ResolvedSelection {
    const lo = Math.min(range.startIndex, range.endIndex);
    const hi = Math.max(range.startIndex, range.endIndex);
    const nights = hi - lo + 1;
    const checkInDate = days[lo].fullDate;
    const checkOutDate = addDaysIso(checkInDate, nights);
    const kind: SelectionKind =
        checkInDate === todayKey ? "checkin"
        : checkInDate > todayKey ? "reservation"
        : "backfill";
    return {
        kind,
        roomId: range.roomId,
        checkInDate,
        checkOutDate,
        nights,
        stillStaying: kind === "backfill" && checkOutDate > todayKey,
    };
}
