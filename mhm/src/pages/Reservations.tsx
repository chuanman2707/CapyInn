import { useState, useEffect, useMemo, type MouseEvent as ReactMouseEvent } from "react";
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

function getBookingBarColor(status: BookingStatus): string {
    if (status === "booked") return "bg-blue-100 text-blue-700 border-blue-300";
    if (status === "active") return "bg-emerald-100 text-emerald-700 border-emerald-300";
    if (status === "checked_out") return "bg-slate-100 text-slate-500 border-slate-200";
    return "bg-orange-100 text-orange-700 border-orange-200";
}

function getStatusLabel(status: BookingStatus): string {
    if (status === "booked") return "Đặt trước";
    if (status === "active") return "Đang ở";
    if (status === "checked_out") return "Đã trả";
    if (status === "no_show") return "Không đến";
    return status;
}

export default function Reservations() {
    const { rooms, fetchRooms, setCheckinOpen } = useHotelStore();
    const [bookings, setBookings] = useState<BookingWithGuest[]>([]);
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

    const loadBookings = () => {
        invoke<BookingWithGuest[]>("get_all_bookings", { filter: null })
            .then(setBookings)
            .catch(() => setBookings([]));
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
    const totalCount = visibleBookings.length;

    function getBookingBars(roomId: string): BookingBar[] {
        return visibleBookings
            .filter(b => b.room_id === roomId && b.status !== "cancelled")
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

    function handleCellMouseDown(roomId: string, colIndex: number, event: ReactMouseEvent) {
        if (event.button !== 0) return;
        event.preventDefault(); // chặn select text khi kéo
        setDragSel({ roomId, startIndex: colIndex, endIndex: colIndex });
    }

    function handleCellMouseEnter(roomId: string, colIndex: number) {
        setDragSel((prev) => (prev && prev.roomId === roomId ? { ...prev, endIndex: colIndex } : prev));
    }

    useEffect(() => {
        if (!dragSel) return;
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
        window.addEventListener("mouseup", onMouseUp);
        window.addEventListener("keydown", onKeyDown);
        window.addEventListener("blur", onBlur);
        return () => {
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
                        Tổng <span className="ml-1 bg-orange-200 text-orange-700 rounded px-1.5 py-0.5 text-[10px]">{totalCount}</span>
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
                                                        onMouseEnter={() => handleCellMouseEnter(room.id, colIndex)}
                                                        className={`w-[80px] shrink-0 border-r border-slate-200 select-none cursor-pointer ${inSelection ? "bg-blue-100/70" : d.isToday ? "bg-blue-50/10" : ""} group-hover:bg-slate-50/30 transition-colors`}
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
                                                    className="absolute top-1/2 -translate-y-1/2 px-0.5 z-10 cursor-pointer"
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

                    {visibleBookings.length === 0 && (
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
