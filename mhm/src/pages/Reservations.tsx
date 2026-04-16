import { useState, useEffect } from "react";
import { Search, ChevronLeft, ChevronRight, Plus, CheckCircle2, XCircle, Pencil } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useHotelStore } from "@/stores/useHotelStore";
import { invoke } from "@tauri-apps/api/core";
import { fmtNumber } from "@/lib/format";
import { toast } from "sonner";
import ReservationSheet from "@/components/ReservationSheet";
import RoomDrawer from "@/components/RoomDrawer";
import type { BookingWithGuest } from "@/types";

function getDateRange(offset: number) {
    return Array.from({ length: 16 }, (_, i) => {
        const d = new Date();
        d.setDate(d.getDate() + i - 3 + offset);
        return {
            day: d.toLocaleDateString("vi-VN", { weekday: "short" }).replace(".", ""),
            date: d.getDate(),
            fullDate: d.toISOString().split("T")[0],
            isToday: i === 3 && offset === 0,
            dateObj: d,
        };
    });
}

function parseDate(s: string): Date {
    // Handle both ISO datetime and YYYY-MM-DD format
    if (s.includes("T")) return new Date(s);
    return new Date(s + "T12:00:00");
}

function getBookingBarColor(status: string): string {
    if (status === "booked") return "bg-blue-100 text-blue-700 border-blue-300";
    if (status === "active") return "bg-emerald-100 text-emerald-700 border-emerald-300";
    if (status === "checked_out") return "bg-slate-100 text-slate-500 border-slate-200";
    return "bg-orange-100 text-orange-700 border-orange-200";
}

function getStatusLabel(status: string): string {
    if (status === "booked") return "Đặt trước";
    if (status === "active") return "Đang ở";
    if (status === "checked_out") return "Đã trả";
    return status;
}

export default function Reservations() {
    const { rooms, fetchRooms } = useHotelStore();
    const [bookings, setBookings] = useState<BookingWithGuest[]>([]);
    const [searchQuery, setSearchQuery] = useState("");
    const [dateOffset, setDateOffset] = useState(0);
    const [currentMonth] = useState(new Date().toLocaleDateString("vi-VN", { month: "long", year: "numeric" }));
    const [sheetOpen, setSheetOpen] = useState(false);
    const [selectedBooking, setSelectedBooking] = useState<BookingWithGuest | null>(null);
    const [editBooking, setEditBooking] = useState<BookingWithGuest | null>(null);
    const [drawerRoomId, setDrawerRoomId] = useState<string | null>(null);

    const DAYS = getDateRange(dateOffset);

    useEffect(() => { fetchRooms(); }, []);

    const loadBookings = () => {
        invoke<BookingWithGuest[]>("get_all_bookings", { filter: null })
            .then(setBookings)
            .catch(() => setBookings([]));
    };

    useEffect(() => { loadBookings(); }, []);

    const roomGroups = [
        { name: "Deluxe", rooms: rooms.filter((r) => r.type === "deluxe").map((r) => ({ id: r.id, type: r.type })) },
        { name: "Standard", rooms: rooms.filter((r) => r.type === "standard").map((r) => ({ id: r.id, type: r.type })) },
    ];

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

    function getBookingBars(roomId: string) {
        return visibleBookings
            .filter(b => b.room_id === roomId && b.status !== "cancelled")
            .map(b => {
                const checkIn = parseDate(b.scheduled_checkin || b.check_in_at);
                const checkOut = parseDate(b.scheduled_checkout || b.expected_checkout);
                const startDay = DAYS[0].dateObj;

                const startCol = Math.max(0, Math.floor((checkIn.getTime() - startDay.getTime()) / (1000 * 60 * 60 * 24)));
                const endCol = Math.max(startCol + 1, Math.ceil((checkOut.getTime() - startDay.getTime()) / (1000 * 60 * 60 * 24)));
                const length = endCol - startCol;

                if (startCol >= 16 || endCol <= 0) return null;

                const clampedStart = Math.max(0, startCol);
                const clampedLength = Math.min(length, 16 - clampedStart);

                const isBooked = b.status === "booked";
                const color = getBookingBarColor(b.status);
                const statusLabel = getStatusLabel(b.status);

                return { ...b, startCol: clampedStart, length: clampedLength, color, statusLabel, isBooked };
            })
            .filter(Boolean);
    }

    async function handleConfirmReservation(bookingId: string) {
        try {
            await invoke("confirm_reservation", { bookingId });
            toast.success("Check-in reservation thành công!");
            loadBookings();
            fetchRooms();
            setSelectedBooking(null);
        } catch (e) {
            toast.error(String(e));
        }
    }

    async function handleCancelReservation(bookingId: string) {
        try {
            await invoke("cancel_reservation", { bookingId });
            toast.success("Đã hủy reservation. Tiền cọc được giữ lại.");
            loadBookings();
            fetchRooms();
            setSelectedBooking(null);
        } catch (e) {
            toast.error(String(e));
        }
    }

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
                    <div className="w-[140px] shrink-0 border-r border-slate-100 bg-white shadow-[2px_0_10px_rgba(0,0,0,0.02)] sticky left-0 z-20 flex items-center justify-between px-4">
                        <span className="text-xs font-semibold text-slate-500">Rooms</span>
                        <div className="flex items-center gap-1">
                            <button className="text-slate-400 hover:text-slate-600 cursor-pointer" onClick={() => setDateOffset(o => o - 7)}><ChevronLeft size={14} /></button>
                            <span className="text-[10px] font-bold text-slate-600 uppercase">{currentMonth}</span>
                            <button className="text-slate-400 hover:text-slate-600 cursor-pointer" onClick={() => setDateOffset(o => o + 7)}><ChevronRight size={14} /></button>
                        </div>
                    </div>

                    {DAYS.map((d, i) => (
                        <div key={i} className={`w-[80px] shrink-0 border-r border-slate-50 flex flex-col items-center justify-center py-2.5 ${d.isToday ? "bg-blue-50/40" : ""}`}>
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
                                        <div key={i} className={`w-[80px] shrink-0 border-r border-slate-50 ${d.isToday ? "bg-blue-50/20" : ""}`} />
                                    ))}
                                </div>
                            </div>

                            {group.rooms.map((room) => {
                                const bars = getBookingBars(room.id);
                                return (
                                    <div key={room.id} className="flex group border-b border-slate-50 h-[64px]">
                                        <div className="w-[140px] shrink-0 border-r border-slate-100 bg-white shadow-[2px_0_10px_rgba(0,0,0,0.02)] sticky left-0 z-10 flex items-center px-4 group-hover:bg-slate-50/50 transition-colors">
                                            <span className="font-bold text-sm text-slate-700">Room {room.id}</span>
                                        </div>

                                        <div className="flex relative w-max">
                                            {DAYS.map((d, colIndex) => (
                                                <div key={colIndex} className={`w-[80px] shrink-0 border-r border-slate-50 ${d.isToday ? "bg-blue-50/10" : ""} group-hover:bg-slate-50/30 transition-colors`} />
                                            ))}

                                            {DAYS.some(d => d.isToday) && (
                                                <div className="absolute top-0 bottom-0 w-[2px] bg-brand-primary/60 z-20" style={{ left: `${DAYS.findIndex(d => d.isToday) * 80 + 40}px` }} />
                                            )}

                                            {bars.map((bar: any) => (
                                                <div
                                                    key={bar.id}
                                                    className="absolute top-1/2 -translate-y-1/2 px-0.5 z-10 cursor-pointer"
                                                    style={{ left: `${bar.startCol * 80}px`, width: `${bar.length * 80}px` }}
                                                    onClick={() => {
                                                        if (bar.isBooked) setSelectedBooking(bar);
                                                        else if (bar.status === "active") setDrawerRoomId(bar.room_id);
                                                    }}
                                                >
                                                    <div className={`h-[42px] w-full ${bar.color} border rounded-xl px-3 flex flex-col justify-center hover:shadow-md hover:-translate-y-0.5 transition-all`}>
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
                <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30" onClick={() => setSelectedBooking(null)}>
                    <div className="bg-white rounded-2xl shadow-2xl p-6 w-[380px] space-y-4" onClick={(e) => e.stopPropagation()}>
                        <h3 className="font-bold text-lg text-slate-800">Reservation — {selectedBooking.guest_name}</h3>
                        <div className="space-y-2 text-sm text-slate-600">
                            <div className="flex justify-between">
                                <span>Phòng</span>
                                <span className="font-semibold">{selectedBooking.room_id}</span>
                            </div>
                            <div className="flex justify-between">
                                <span>Check-in</span>
                                <span className="font-semibold">{selectedBooking.scheduled_checkin || selectedBooking.check_in_at}</span>
                            </div>
                            <div className="flex justify-between">
                                <span>Check-out</span>
                                <span className="font-semibold">{selectedBooking.scheduled_checkout || selectedBooking.expected_checkout}</span>
                            </div>
                            <div className="flex justify-between">
                                <span>Số đêm</span>
                                <span className="font-semibold">{selectedBooking.nights}</span>
                            </div>
                            <div className="flex justify-between">
                                <span>Tổng tiền</span>
                                <span className="font-bold text-slate-800">{fmtNumber(selectedBooking.total_price)}₫</span>
                            </div>
                            {(selectedBooking.deposit_amount || 0) > 0 && (
                                <div className="flex justify-between">
                                    <span>Đã cọc</span>
                                    <span className="font-semibold text-emerald-600">{fmtNumber(selectedBooking.deposit_amount || 0)}₫</span>
                                </div>
                            )}
                            {selectedBooking.guest_phone && (
                                <div className="flex justify-between">
                                    <span>SĐT</span>
                                    <span className="font-semibold">{selectedBooking.guest_phone}</span>
                                </div>
                            )}
                        </div>
                        <div className="flex gap-2 pt-2">
                            <Button
                                className="flex-1 bg-emerald-600 hover:bg-emerald-700 text-white rounded-xl h-10 gap-1.5 cursor-pointer"
                                onClick={() => handleConfirmReservation(selectedBooking.id)}
                            >
                                <CheckCircle2 size={14} /> Check-in
                            </Button>
                            <Button
                                variant="outline"
                                className="flex-1 border-blue-200 text-blue-600 hover:bg-blue-50 rounded-xl h-10 gap-1.5 cursor-pointer"
                                onClick={() => { setEditBooking(selectedBooking); setSelectedBooking(null); }}
                            >
                                <Pencil size={14} /> Chỉnh sửa
                            </Button>
                            <Button
                                variant="outline"
                                className="flex-1 border-red-200 text-red-600 hover:bg-red-50 rounded-xl h-10 gap-1.5 cursor-pointer"
                                onClick={() => handleCancelReservation(selectedBooking.id)}
                            >
                                <XCircle size={14} /> Hủy
                            </Button>
                        </div>
                    </div>
                </div>
            )}

            {/* Reservation Sheet */}
            {/* Room Drawer for active bookings */}
            <RoomDrawer
                open={!!drawerRoomId}
                onClose={() => { setDrawerRoomId(null); loadBookings(); fetchRooms(); }}
                roomId={drawerRoomId}
            />

            <ReservationSheet
                open={sheetOpen || !!editBooking}
                onOpenChange={(v) => {
                    setSheetOpen(v);
                    if (!v) { setEditBooking(null); loadBookings(); }
                }}
                editBooking={editBooking || undefined}
            />
        </div>
    );
}
