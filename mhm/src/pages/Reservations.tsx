import { useState, useEffect, useMemo, useRef, type MouseEvent as ReactMouseEvent } from "react";
import { listen } from "@tauri-apps/api/event";
import { Search, ChevronLeft, ChevronRight, Plus } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useHotelStore } from "@/stores/useHotelStore";
import { invoke } from "@tauri-apps/api/core";
import { getRoomTypeLabel } from "@/lib/constants";
import { createCorrelationId } from "@/lib/correlationId";
import { formatAppError } from "@/lib/appError";
import { createDeferredCleanup } from "@/lib/deferredCleanup";
import { invokeWriteCommand } from "@/lib/invokeCommand";
import { toast } from "sonner";
import BookingDetailPopup from "@/components/BookingDetailPopup";
import InvoiceDialog from "@/components/InvoiceDialog";
import { useInvoiceDialog } from "@/hooks/useInvoiceDialog";
import ReservationSheet from "@/components/ReservationSheet";
import BackfillSheet, { type BackfillPrefill } from "@/components/BackfillSheet";
import RoomDrawer from "@/components/RoomDrawer";
import { resolveSelection, localDateIso, type TimelineSelectionRange } from "@/lib/timelineSelection";
import type { BookingStatus, BookingWithGuest } from "@/types";

type BookingBar = BookingWithGuest & {
    left: number;
    width: number;
    clippedLeft: boolean;
    clippedRight: boolean;
    color: string;
    statusLabel: string;
    isBooked: boolean;
};

const DAY_MS = 24 * 60 * 60 * 1000;
const VISIBLE_DAYS = 16;
const COL_WIDTH = 80;
/** Nhận phòng buổi chiều, trả phòng buổi sáng: bar lệch nửa ô ở cả hai đầu. */
const HALF_DAY = 0.5;
/** Khách nhận và trả cùng ngày vẫn phải nhìn thấy được. */
const MIN_BAR_DAYS = 0.5;

function startOfLocalDay(date: Date): Date {
    return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

function differenceInCalendarDays(left: Date, right: Date): number {
    const leftUtc = Date.UTC(left.getFullYear(), left.getMonth(), left.getDate());
    const rightUtc = Date.UTC(right.getFullYear(), right.getMonth(), right.getDate());
    return Math.round((leftUtc - rightUtc) / DAY_MS);
}

function getDateRange(offset: number) {
    const today = startOfLocalDay(new Date());
    const todayKey = localDateIso(today);
    return Array.from({ length: VISIBLE_DAYS }, (_, i) => {
        const d = new Date(today);
        d.setDate(today.getDate() + i - 3 + offset);
        const fullDate = localDateIso(d);
        return {
            day: d.toLocaleDateString("vi-VN", { weekday: "short" }).replace(".", ""),
            date: d.getDate(),
            fullDate,
            isToday: fullDate === todayKey,
            dateObj: d,
        };
    });
}

type TimelineDay = ReturnType<typeof getDateRange>[number];

function formatRangeLabel(days: TimelineDay[]): string {
    const first = days[0].dateObj;
    const last = days[days.length - 1].dateObj;
    const firstMonth = first.getMonth() + 1;
    const lastMonth = last.getMonth() + 1;
    const firstYear = first.getFullYear();
    const lastYear = last.getFullYear();

    if (firstYear !== lastYear) {
        return `${firstMonth}/${firstYear} – ${lastMonth}/${lastYear}`;
    }

    if (firstMonth !== lastMonth) {
        return `THÁNG ${firstMonth}–${lastMonth} / ${firstYear}`;
    }

    return `THÁNG ${firstMonth} NĂM ${firstYear}`;
}

function parseDate(s: string): Date {
    const dateOnly = /^(\d{4})-(\d{2})-(\d{2})$/.exec(s);
    if (dateOnly) {
        return new Date(Number(dateOnly[1]), Number(dateOnly[2]) - 1, Number(dateOnly[3]));
    }

    const parsed = new Date(s);
    return Number.isNaN(parsed.getTime()) ? parsed : startOfLocalDay(parsed);
}

// Chỉ những status này mới có bar trên lịch. DANH SÁCH TRẮNG — cố ý, không
// phải danh sách đen (`status !== "cancelled"` cũ). Một status mới thêm vào
// BookingStatus mà không có mặt ở đây thì KHÔNG vẽ bar, thay vì lọt qua mặc
// định như đường đen — đúng chiều an toàn khi status là dữ liệu người dùng
// không kiểm soát được (đọc từ backend, có thể là bug tầng SQL sót lại).
const VISIBLE_BOOKING_STATUSES: readonly BookingStatus[] = [
    "active",
    "booked",
    "checked_out",
    "no_show",
];

/** Không bao giờ được gọi nếu switch bên dưới xét đủ mọi nhánh của BookingStatus. */
function assertUnreachableStatus(status: never): never {
    throw new Error(`Thiếu nhánh xử lý cho status: ${String(status)}`);
}

// switch cạn kiệt (exhaustive) thay vì chuỗi if/else: thêm một status mới vào
// BookingStatus mà quên xử lý ở đây là LỖI BIÊN DỊCH, không phải một bar màu
// cam âm thầm hiện ra. Đây chính là rào chắn C1 yêu cầu — trước đây if/else
// với nhánh mặc định (`return status`) không ép tsc bắt lỗi thiếu nhánh.
function getBookingBarColor(status: BookingStatus): string {
    switch (status) {
        case "booked":
            return "bg-blue-100 text-blue-700 border-blue-300";
        case "active":
            return "bg-emerald-100 text-emerald-700 border-emerald-300";
        case "checked_out":
            return "bg-slate-100 text-slate-500 border-slate-200";
        case "no_show":
            return "bg-orange-100 text-orange-700 border-orange-200";
        // cancelled/voided không bao giờ tới đây (đã lọc ở VISIBLE_BOOKING_STATUSES)
        // — màu xám trung tính, không phải cam (cam đang là màu "đến hạn/chú ý").
        case "cancelled":
        case "voided":
            return "bg-slate-100 text-slate-400 border-slate-200";
        default:
            return assertUnreachableStatus(status);
    }
}

function getStatusLabel(status: BookingStatus): string {
    switch (status) {
        case "booked":
            return "Đặt trước";
        case "active":
            return "Đang ở";
        case "checked_out":
            return "Đã trả";
        case "no_show":
            return "Không đến";
        case "cancelled":
            return "Đã hủy";
        case "voided":
            return "Đã xóa";
        default:
            return assertUnreachableStatus(status);
    }
}

// `get_all_bookings` là lệnh ĐỌC: chữ ký của nó là `Result<_, String>`, nên
// thứ về tới đây là một chuỗi thô, không phải phong bì AppError.
// `normalizeAppError` chỉ nhận diện phong bì và quy mọi thứ khác về một câu
// chung chung, nên `formatAppError` dùng một mình ở đây sẽ nuốt mất dòng chẩn
// đoán duy nhất — đúng thứ đã chỉ ra "no such column: b.guests" hôm 31/07/2026.
// Phong bì thật thì vẫn để formatAppError lo, vì nó còn gắn mã hỗ trợ và mã
// theo dõi.
function describeLoadError(error: unknown): string {
    if (typeof error === "string") return error;
    if (error instanceof Error) return error.message;
    return formatAppError(error);
}

export default function Reservations() {
    const { rooms, fetchRooms, setCheckinOpen, setRoomChangeOpen } = useHotelStore();
    const [bookings, setBookings] = useState<BookingWithGuest[]>([]);
    const [loadError, setLoadError] = useState<string | null>(null);
    const [searchQuery, setSearchQuery] = useState("");
    const [dateOffset, setDateOffset] = useState(0);
    const [sheetOpen, setSheetOpen] = useState(false);
    const [selectedBooking, setSelectedBooking] = useState<BookingWithGuest | null>(null);
    const [editBooking, setEditBooking] = useState<BookingWithGuest | null>(null);
    const [drawerRoomId, setDrawerRoomId] = useState<string | null>(null);
    const [dragSel, setDragSel] = useState<TimelineSelectionRange | null>(null);
    const [reservationPrefill, setReservationPrefill] = useState<{ roomId: string; checkIn: string; checkOut: string } | null>(null);
    const [backfillPrefill, setBackfillPrefill] = useState<BackfillPrefill | null>(null);
    const { invoiceOpen, invoiceData, invoiceLoading, viewInvoice, closeInvoice } = useInvoiceDialog();

    const DAYS = useMemo(() => getDateRange(dateOffset), [dateOffset]);
    const rangeLabel = formatRangeLabel(DAYS);

    useEffect(() => { fetchRooms(); }, []);

    // Lỗi đọc booking phải hiện ra, không được nuốt. Bản trước làm
    // `.catch(() => setBookings([]))`, biến mọi thất bại thành đúng cái màn
    // hình mà một khách sạn chưa có booking nào cũng thấy. Ngày 31/07/2026,
    // cột `bookings.guests` thiếu trên máy chủ khách sạn (migration v22 bị bỏ
    // qua vì schema_version đã là 23) nên query đổ lỗi "no such column:
    // b.guests"; chủ khách sạn đọc màn hình trống ra là mất sạch dữ liệu và đi
    // tìm bản backup, trong khi 25 booking vẫn nằm nguyên trong database.
    //
    // `setBookings` cũng không bị đụng tới ở nhánh lỗi: một lần nạp lại hỏng
    // (ổ đĩa bận, database khoá) mà xoá sạch lịch là tự tay biến sự cố tạm
    // thời thành cảnh mất dữ liệu.
    const loadBookings = () => {
        invoke<BookingWithGuest[]>("get_all_bookings", { filter: null })
            .then((rows) => {
                setBookings(rows);
                setLoadError(null);
            })
            .catch((e) => setLoadError(describeLoadError(e)));
    };

    useEffect(() => { loadBookings(); }, []);

    // `bookings` là state cục bộ của trang này, và listener toàn cục ở
    // RuntimeStateProvider chỉ làm mới rooms/stats — không ai làm mới bookings.
    // Đường check-in kéo từ lịch bàn giao hẳn cho CheckinSheet ở MainShell nên
    // cũng không có callback nào quay về đây: check-in xong, lịch đứng im,
    // không có bar nào hiện ra, và chủ kéo lại đúng ô đó thì ăn lỗi "phòng
    // không còn trống". Backend phát "db-updated" sau MỌI lệnh ghi
    // (commands/mod.rs::emit_db_update), nên nghe đúng một chỗ này là ba đường
    // kéo (check-in / đặt trước / ghi bù) làm mới giống hệt nhau.
    //
    // Vì mọi lệnh ghi đều đi qua đây, các lời gọi loadBookings() rải trong
    // handler đóng sheet / confirm / cancel đã được bỏ: giữ lại chúng sẽ nạp
    // hai lần cho cùng một thao tác. Lần nạp lúc mount ở trên vẫn cần — không
    // có sự kiện nào phát khi trang vừa mở.
    useEffect(() => {
        const cleanup = createDeferredCleanup(
            listen<{ entity: string }>("db-updated", () => { loadBookings(); }),
        );
        return cleanup;
    }, []);

    const roomGroups = Object.values(
        rooms.reduce<Record<string, { name: string; rooms: { id: string; type: string }[] }>>((groups, room) => {
            const existing = groups[room.type] ?? {
                name: getRoomTypeLabel(room.type),
                rooms: [],
            };

            existing.rooms.push({ id: room.id, type: room.type });
            groups[room.type] = existing;
            return groups;
        }, {}),
    ).sort((left, right) => left.name.localeCompare(right.name, "vi"));

    const normalizedQuery = searchQuery.trim().toLocaleLowerCase();
    const visibleBookings = normalizedQuery
        ? bookings.filter((booking) => {
            const searchHaystack = [
                booking.guest_name,
                booking.room_id,
                booking.id,
                booking.source,
            ]
                .filter(Boolean)
                .join(" ")
                .toLocaleLowerCase();

            return searchHaystack.includes(normalizedQuery);
        })
        : bookings;

    const activeCount = visibleBookings.filter(b => b.status === "active").length;
    const bookedCount = visibleBookings.filter(b => b.status === "booked").length;
    const checkedOutCount = visibleBookings.filter(b => b.status === "checked_out").length;
    // M6 (rà cuối trước merge): trước đây lọc bằng danh sách ĐEN
    // (`status !== "voided"`) nên "Tổng" vẫn cộng cả lượt "cancelled" dù bar
    // trên lịch dùng danh sách TRẮNG `VISIBLE_BOOKING_STATUSES` (không có
    // "cancelled") — hai chỗ lệch nhau trong cùng một commit. Dùng chung
    // đúng một danh sách trắng cho cả hai: "Tổng" mô tả các lượt còn đang
    // hiện diện trên lịch (có bar), không phải toàn bộ lịch sử đã từng tạo.
    const totalCount = visibleBookings.filter(b => VISIBLE_BOOKING_STATUSES.includes(b.status)).length;

    function getBookingBars(roomId: string): BookingBar[] {
        return visibleBookings
            .filter(b => b.room_id === roomId && VISIBLE_BOOKING_STATUSES.includes(b.status))
            .flatMap((b): BookingBar[] => {
                const checkIn = parseDate(b.scheduled_checkin || b.check_in_at);
                // Booking đã trả: bar dừng đúng lúc trả phòng thực tế, kể cả khi trước đó lỡ extend.
                const checkOut = parseDate(
                    b.status === "checked_out" && b.actual_checkout
                        ? b.actual_checkout
                        : b.scheduled_checkout || b.expected_checkout,
                );
                const startDay = DAYS[0].dateObj;

                const rawStart = differenceInCalendarDays(checkIn, startDay) + HALF_DAY;
                const rawEnd = Math.max(
                    differenceInCalendarDays(checkOut, startDay) + HALF_DAY,
                    rawStart + MIN_BAR_DAYS,
                );

                // Lọc trước khi clamp — clamp trước sẽ kéo mọi booking quá khứ về cột 0.
                if (rawStart >= VISIBLE_DAYS || rawEnd <= 0) return [];

                const visStart = Math.max(0, rawStart);
                const visEnd = Math.min(VISIBLE_DAYS, rawEnd);

                return [{
                    ...b,
                    left: visStart * COL_WIDTH,
                    width: (visEnd - visStart) * COL_WIDTH,
                    clippedLeft: rawStart < 0,
                    clippedRight: rawEnd > VISIBLE_DAYS,
                    color: getBookingBarColor(b.status),
                    statusLabel: getStatusLabel(b.status),
                    isBooked: b.status === "booked",
                }];
            })
    }

    async function handleConfirmReservation(bookingId: string) {
        const correlationId = createCorrelationId();
        try {
            await invokeWriteCommand("confirm_reservation", { bookingId }, { correlationId });
            toast.success("Check-in reservation thành công!");
            setSelectedBooking(null);
        } catch (e) {
            toast.error(formatAppError(e));
        }
    }

    async function handleCancelReservation(bookingId: string) {
        const correlationId = createCorrelationId();
        try {
            await invokeWriteCommand("cancel_reservation", { bookingId }, { correlationId });
            toast.success("Đã hủy reservation. Tiền cọc được giữ lại.");
            setSelectedBooking(null);
        } catch (e) {
            toast.error(formatAppError(e));
        }
    }

    // Dải ô của đúng hàng đang kéo, ghim lúc nhấn chuột. Đọc lại rect ở mỗi
    // lần chuột nhúc nhích chứ không nhớ sẵn một con số: lịch cuộn ngang được,
    // và cuộn giữa chừng cú kéo sẽ làm mọi toạ độ nhớ sẵn thành sai.
    const dragGridRef = useRef<HTMLElement | null>(null);

    function handleCellMouseDown(roomId: string, colIndex: number, event: ReactMouseEvent<HTMLDivElement>) {
        if (event.button !== 0) return;
        event.preventDefault(); // chặn select text khi kéo
        dragGridRef.current = event.currentTarget.parentElement;
        setDragSel({ roomId, startIndex: colIndex, endIndex: colIndex });
    }

    // Cột dưới con trỏ, suy từ TOẠ ĐỘ chuột — không phải từ sự kiện hover.
    //
    // Bản trước gắn onMouseEnter lên từng ô. WebKit (WKWebView, engine của app
    // trên macOS) không cập nhật hover khi đang giữ nút chuột: nó ngưng bắn cặp
    // mouseover/mouseout suốt cú kéo, mà React suy ra onMouseEnter chính từ cặp
    // đó. Hệ quả: giữ chuột kéo qua các ô kế tiếp thì không ô nào nhận được gì,
    // vùng chọn đứng nguyên ở ô đầu, và cú kéo 2/8 → 5/8 ra một đêm thay vì
    // bốn. Chromium bắn hover bình thường nên bug vô hình khi thử ngoài app.
    //
    // `mousemove` thì mọi engine đều bắn suốt cú kéo, nên tính cột từ clientX
    // là cách duy nhất không phụ thuộc engine.
    useEffect(() => {
        if (!dragSel) return;
        const onMouseMove = (event: MouseEvent) => {
            const grid = dragGridRef.current;
            if (!grid) return;
            const col = Math.floor((event.clientX - grid.getBoundingClientRect().left) / COL_WIDTH);
            const clamped = Math.max(0, Math.min(DAYS.length - 1, col));
            setDragSel((prev) => (prev && prev.endIndex !== clamped ? { ...prev, endIndex: clamped } : prev));
        };
        const onMouseUp = () => {
            const resolved = resolveSelection(dragSel, DAYS, localDateIso(new Date()));
            setDragSel(null);
            if (resolved.kind === "checkin") {
                setCheckinOpen(true, resolved.roomId, resolved.nights);
            } else if (resolved.kind === "reservation") {
                setReservationPrefill({ roomId: resolved.roomId, checkIn: resolved.checkInDate, checkOut: resolved.checkOutDate });
                setSheetOpen(true);
            } else {
                setBackfillPrefill({
                    roomId: resolved.roomId,
                    checkInDate: resolved.checkInDate,
                    checkOutDate: resolved.checkOutDate,
                    stillStaying: resolved.stillStaying,
                });
            }
        };
        const onKeyDown = (event: KeyboardEvent) => {
            if (event.key === "Escape") setDragSel(null);
        };
        const onBlur = () => setDragSel(null);
        window.addEventListener("mousemove", onMouseMove);
        window.addEventListener("mouseup", onMouseUp);
        window.addEventListener("keydown", onKeyDown);
        window.addEventListener("blur", onBlur);
        return () => {
            window.removeEventListener("mousemove", onMouseMove);
            window.removeEventListener("mouseup", onMouseUp);
            window.removeEventListener("keydown", onKeyDown);
            window.removeEventListener("blur", onBlur);
        };
    }, [dragSel, DAYS]);

    return (
        <div className="flex flex-col h-full bg-white rounded-3xl shadow-soft overflow-hidden">

            {/* Toolbar */}
            <div className="flex items-center justify-between p-5 border-b border-slate-100 bg-white z-20">
                <div className="flex items-center gap-3">
                    <Badge className="bg-emerald-50 text-emerald-700 border border-emerald-200 rounded-lg px-3 py-1 text-xs font-bold">
                        Đang ở <span className="ml-1 bg-emerald-200 text-emerald-800 rounded px-1.5 py-0.5 text-[10px]">{activeCount}</span>
                    </Badge>
                    <Badge className="bg-blue-50 text-blue-700 border border-blue-200 rounded-lg px-3 py-1 text-xs font-bold">
                        Đặt trước <span className="ml-1 bg-blue-200 text-blue-800 rounded px-1.5 py-0.5 text-[10px]">{bookedCount}</span>
                    </Badge>
                    <Badge className="bg-slate-50 text-slate-500 border border-slate-200 rounded-lg px-3 py-1 text-xs font-bold">
                        Đã trả <span className="ml-1 bg-slate-200 text-slate-600 rounded px-1.5 py-0.5 text-[10px]">{checkedOutCount}</span>
                    </Badge>
                    <Badge className="bg-orange-50 text-orange-600 border border-orange-200 rounded-lg px-3 py-1 text-xs font-bold">
                        Tổng <span data-testid="total-booking-count" className="ml-1 bg-orange-200 text-orange-700 rounded px-1.5 py-0.5 text-[10px]">{totalCount}</span>
                    </Badge>
                </div>

                <div className="flex items-center gap-3">
                    <div className="flex items-center gap-1.5 pr-3 border-r border-slate-100">
                        <button
                            aria-label="Tuần trước"
                            className="text-slate-400 hover:text-slate-600 cursor-pointer p-1"
                            onClick={() => setDateOffset(o => o - 7)}
                        >
                            <ChevronLeft size={16} />
                        </button>
                        <span
                            data-testid="timeline-range-label"
                            className="text-xs font-bold text-slate-600 uppercase whitespace-nowrap min-w-[150px] text-center"
                        >
                            {rangeLabel}
                        </span>
                        <button
                            aria-label="Tuần sau"
                            className="text-slate-400 hover:text-slate-600 cursor-pointer p-1"
                            onClick={() => setDateOffset(o => o + 7)}
                        >
                            <ChevronRight size={16} />
                        </button>
                        {dateOffset !== 0 && (
                            <Button
                                size="sm"
                                variant="outline"
                                className="h-8 px-3 rounded-lg text-xs cursor-pointer"
                                onClick={() => setDateOffset(0)}
                            >
                                Hôm nay
                            </Button>
                        )}
                    </div>
                    <div className="relative w-56">
                        <Search className="absolute left-3 top-1/2 -translate-y-1/2 text-slate-400" size={16} />
                        <Input
                            placeholder="Tìm khách..."
                            className="pl-9 bg-slate-50 border-transparent rounded-xl h-9"
                            value={searchQuery}
                            onChange={(event) => setSearchQuery(event.target.value)}
                        />
                    </div>
                    <Button
                        size="sm"
                        className="bg-blue-600 hover:bg-blue-700 text-white rounded-xl h-9 px-4 gap-1.5 cursor-pointer"
                        onClick={() => setSheetOpen(true)}
                    >
                        <Plus size={14} /> Đặt phòng
                    </Button>
                </div>
            </div>

            {/* Timeline Grid */}
            <div className="flex-1 flex flex-col min-h-0 overflow-hidden relative">

                {/* Day Headers */}
                <div className="flex border-b border-slate-100 bg-white sticky top-0 z-10 w-max min-w-full">
                    <div className="w-[140px] shrink-0 border-r border-slate-100 bg-white shadow-[2px_0_10px_rgba(0,0,0,0.02)] sticky left-0 z-20 flex items-center px-4">
                        <span className="text-xs font-semibold text-slate-500">Rooms</span>
                    </div>

                    {DAYS.map((d, i) => (
                        <div key={i} className={`w-[80px] shrink-0 border-r border-slate-200 flex flex-col items-center justify-center py-2.5 ${d.isToday ? "bg-blue-50/40" : ""}`}>
                            <span className={`text-[10px] font-semibold uppercase ${d.isToday ? "text-brand-primary" : "text-slate-400"}`}>{d.day}</span>
                            <span className={`text-sm font-bold ${d.isToday ? "text-brand-primary" : "text-slate-700"}`}>{d.date}</span>
                        </div>
                    ))}
                </div>

                {/* Timeline Body */}
                <div className="flex-1 overflow-auto w-max min-w-full">
                    {roomGroups.map((group) => (
                        <div key={group.name}>
                            <div className="flex h-[36px] bg-slate-50/80 border-b border-slate-100">
                                <div className="w-[140px] shrink-0 border-r border-slate-100 bg-slate-50 sticky left-0 z-10 flex items-center px-4">
                                    <span className="text-xs font-bold text-slate-500 uppercase tracking-wider">{group.name}</span>
                                </div>
                                <div className="flex">
                                    {DAYS.map((d, i) => (
                                        <div key={i} className={`w-[80px] shrink-0 border-r border-slate-200 ${d.isToday ? "bg-blue-50/20" : ""}`} />
                                    ))}
                                </div>
                            </div>

                            {group.rooms.map((room) => {
                                const bars = getBookingBars(room.id);
                                return (
                                    <div key={room.id} className="flex group border-b border-slate-100 h-[64px]">
                                        <div className="w-[140px] shrink-0 border-r border-slate-100 bg-white shadow-[2px_0_10px_rgba(0,0,0,0.02)] sticky left-0 z-10 flex items-center px-4 group-hover:bg-slate-50/50 transition-colors">
                                            <span className="font-bold text-sm text-slate-700">Room {room.id}</span>
                                        </div>

                                        <div className="flex relative w-max">
                                            {DAYS.map((d, colIndex) => {
                                                const inSelection = dragSel !== null
                                                    && dragSel.roomId === room.id
                                                    && colIndex >= Math.min(dragSel.startIndex, dragSel.endIndex)
                                                    && colIndex <= Math.max(dragSel.startIndex, dragSel.endIndex);
                                                return (
                                                    <div
                                                        key={colIndex}
                                                        data-testid={`cell-${room.id}-${colIndex}`}
                                                        onMouseDown={(event) => handleCellMouseDown(room.id, colIndex, event)}
                                                        // Ô đang chọn KHÔNG mang lớp hover: `group-hover:` có
                                                        // độ ưu tiên CSS cao hơn `bg-blue-100/70` nên nó đè mất
                                                        // màu vùng chọn. Mà kéo thì con trỏ luôn nằm trên chính
                                                        // hàng đang kéo, nên trước đây vùng chọn vô hình suốt cú
                                                        // kéo và chỉ lộ ra khi con trỏ rời sang hàng khác — đo
                                                        // trong trình duyệt thật: nền ô đã chọn ra slate-50/30.
                                                        className={`w-[80px] shrink-0 border-r border-slate-200 select-none cursor-pointer transition-colors ${inSelection
                                                            ? "bg-blue-100/70"
                                                            : `${d.isToday ? "bg-blue-50/10" : ""} group-hover:bg-slate-50/30`
                                                            }`}
                                                    />
                                                );
                                            })}

                                            {DAYS.some(d => d.isToday) && (
                                                <div data-testid="timeline-today-marker" className="absolute top-0 bottom-0 w-[2px] bg-brand-primary/60 z-20 pointer-events-none" style={{ left: `${DAYS.findIndex(d => d.isToday) * COL_WIDTH + COL_WIDTH / 2}px` }} />
                                            )}

                                            {bars.map((bar) => (
                                                <div
                                                    key={bar.id}
                                                    data-testid={`booking-bar-${bar.id}`}
                                                    // `inset-y-0`, KHÔNG phải `top-1/2
                                                    // -translate-y-1/2`: khung bọc cao bằng thanh
                                                    // 42px sẽ chừa 11px hở trên và 11px hở dưới
                                                    // trong hàng 64px, và `mousedown` ở dải đó rơi
                                                    // xuống ô ngày bên dưới. Bấm trúng dải ấy trên
                                                    // một phòng đang có khách mở biểu mẫu đặt phòng
                                                    // cho đúng ngày khách đang ở — cảnh báo đỏ, nút
                                                    // bấm chết. Cả chiều cao hàng thuộc về khách
                                                    // đang chiếm ngày đó.
                                                    className="absolute inset-y-0 px-0.5 z-10 cursor-pointer flex items-center"
                                                    style={{ left: `${bar.left}px`, width: `${bar.width}px` }}
                                                    onClick={() => {
                                                        if (bar.status === "active") setDrawerRoomId(bar.room_id);
                                                        else if (bar.status === "booked" || bar.status === "checked_out") setSelectedBooking(bar);
                                                    }}
                                                >
                                                    <div className={`h-[42px] w-full ${bar.color} border rounded-xl ${bar.clippedLeft ? "rounded-l-none" : ""} ${bar.clippedRight ? "rounded-r-none" : ""} px-3 flex flex-col justify-center hover:shadow-md hover:-translate-y-0.5 transition-all`}>
                                                        <span className="font-semibold text-xs truncate">{bar.guest_name}</span>
                                                        <div className="flex items-center gap-1.5 mt-0.5">
                                                            <span className="text-[9px] opacity-70">{bar.source || "walk-in"}</span>
                                                            <Badge className={`text-[8px] px-1 py-0 h-3.5 rounded border-0 ${bar.isBooked
                                                                ? "bg-blue-200 text-blue-800"
                                                                : bar.status === "active"
                                                                    ? "bg-emerald-200 text-emerald-800"
                                                                    : "bg-slate-200 text-slate-600"
                                                                }`}>
                                                                {bar.statusLabel}
                                                            </Badge>
                                                        </div>
                                                    </div>
                                                </div>
                                            ))}
                                        </div>
                                    </div>
                                );
                            })}
                        </div>
                    ))}

                    {/* Lỗi thắng ô trống: "Chưa có booking nào" chỉ đúng khi
                        đọc THÀNH CÔNG và kết quả rỗng thật. */}
                    {loadError && (
                        <div className="mx-4 my-4 rounded-xl border border-red-200 bg-red-50 p-4 text-sm text-red-700">
                            <div className="font-semibold">Không đọc được danh sách booking</div>
                            <div className="mt-1 whitespace-pre-line opacity-90">{loadError}</div>
                            <div className="mt-2 text-xs opacity-80">
                                Dữ liệu vẫn nằm trong máy — đây là lỗi lúc đọc, không phải mất booking.
                            </div>
                            <Button
                                className="mt-3 h-9 rounded-lg text-sm font-semibold"
                                onClick={loadBookings}
                            >
                                Thử lại
                            </Button>
                        </div>
                    )}

                    {!loadError && visibleBookings.length === 0 && (
                        <div className="flex items-center justify-center py-20 text-brand-muted">
                            {bookings.length === 0
                                ? 'Chưa có booking nào — Nhấn "+ Đặt phòng" để tạo reservation'
                                : "Không tìm thấy booking phù hợp"}
                        </div>
                    )}
                </div>
            </div>

            {/* Reservation Action Popup */}
            {selectedBooking && (
                <BookingDetailPopup
                    booking={selectedBooking}
                    mode={selectedBooking.status === "checked_out" ? "readonly" : "reservation"}
                    onClose={() => setSelectedBooking(null)}
                    onConfirm={handleConfirmReservation}
                    onEdit={(booking) => { setEditBooking(booking); setSelectedBooking(null); }}
                    onCancel={handleCancelReservation}
                    onViewInvoice={viewInvoice}
                    onRoomChange={(bookingId) => setRoomChangeOpen(true, bookingId)}
                    invoiceLoading={invoiceLoading}
                />
            )}

            {/* Room Drawer for active bookings */}
            <RoomDrawer
                open={!!drawerRoomId}
                onClose={() => setDrawerRoomId(null)}
                roomId={drawerRoomId}
            />

            <InvoiceDialog
                open={invoiceOpen}
                onOpenChange={(nextOpen) => {
                    if (!nextOpen) closeInvoice();
                }}
                data={invoiceData}
            />

            {/* Reservation Sheet */}
            <ReservationSheet
                open={sheetOpen || !!editBooking}
                onOpenChange={(v) => {
                    setSheetOpen(v);
                    if (!v) { setEditBooking(null); setReservationPrefill(null); }
                }}
                editBooking={editBooking || undefined}
                preSelectedRoomId={reservationPrefill?.roomId}
                prefillDates={reservationPrefill ? { checkIn: reservationPrefill.checkIn, checkOut: reservationPrefill.checkOut } : undefined}
            />

            {/* Backfill Sheet — ghi bù khách đã ở nhưng chưa nhập, mở từ ô ngày quá khứ */}
            <BackfillSheet
                open={!!backfillPrefill}
                onOpenChange={(v) => {
                    if (!v) setBackfillPrefill(null);
                }}
                prefill={backfillPrefill ?? undefined}
            />
        </div>
    );
}
