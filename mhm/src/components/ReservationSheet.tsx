import { useState, useEffect, useRef } from "react";
import { useHotelStore } from "../stores/useHotelStore";
import { CalendarDays, User, Phone, CreditCard, AlertTriangle, FileText } from "lucide-react";
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";
import { useAvailability } from "@/hooks/useAvailability";
import { usePricePreview } from "@/hooks/usePricePreview";
import { useInvoiceDialog } from "@/hooks/useInvoiceDialog";
import { formatAppError } from "@/lib/appError";
import { createCorrelationId } from "@/lib/correlationId";
import { fmtNumber } from "@/lib/format";
import { invokeWriteCommand } from "@/lib/invokeCommand";
import { optionalMoneyVnd } from "@/lib/money";
import { toast } from "sonner";
import InvoiceDialog from "./InvoiceDialog";
import type { EditableBooking } from "@/types";

interface Props {
    open: boolean;
    onOpenChange: (v: boolean) => void;
    preSelectedRoomId?: string;
    editBooking?: EditableBooking;
    prefillDates?: { checkIn: string; checkOut: string };
}

function nightsBetween(checkIn: string, checkOut: string): number {
    if (!checkIn || !checkOut) return 0;
    // Ngày dạng "YYYY-MM-DD" (không có giờ) được JS phân giải theo UTC — dùng
    // nguyên dạng đó (không thêm "T00:00:00") để tránh lệch ngày ở múi giờ
    // dương như Asia/Ho_Chi_Minh (UTC+7) khi quy đổi qua toISOString().
    const ms = new Date(checkOut).getTime() - new Date(checkIn).getTime();
    if (Number.isNaN(ms)) return 0;
    return Math.round(ms / 86_400_000);
}

function addDays(date: string, days: number): string {
    const d = new Date(date);
    d.setDate(d.getDate() + days);
    return d.toISOString().split("T")[0];
}

// Trần số đêm cho một đặt phòng. Trước đây ngày đi chỉ đọc được, suy ra từ
// một ô số đêm mang `max={90}` — 90 đêm là trần cứng theo cấu trúc. Ô đó bị
// xoá để cho gõ tay ngày đi trực tiếp, và không gì thay thế trần cũ ở đây:
// một lỗi gõ năm (2036 thay vì 2026) tạo ra một đặt phòng khoảng 3650 đêm.
// Đây là khôi phục lại trần cũ, không phải đặt ra chính sách mới.
const MAX_RESERVATION_NIGHTS = 90;

export default function ReservationSheet({ open, onOpenChange, preSelectedRoomId, editBooking, prefillDates }: Props) {
    const { rooms, fetchRooms } = useHotelStore();
    const [roomId, setRoomId] = useState(preSelectedRoomId || "");
    const [guestName, setGuestName] = useState("");
    const [guestPhone, setGuestPhone] = useState("");
    const [guestDoc, setGuestDoc] = useState("");
    const [checkInDate, setCheckInDate] = useState("");
    const [checkOutDate, setCheckOutDate] = useState("");
    const [guests, setGuests] = useState(2);
    // Chỉ có ý nghĩa ở edit mode: người dùng đã đụng vào ô số khách trong lần
    // sửa này chưa. Dùng để phân biệt "chưa đụng tới" (giữ nguyên số đã lưu,
    // kể cả khi số đã lưu là NULL) với "đã chọn một số khách mới" — xem
    // handleSubmit và effect nạp placeholder bên dưới.
    const [guestsTouched, setGuestsTouched] = useState(false);
    const nights = nightsBetween(checkInDate, checkOutDate);
    const datesValid = nights > 0;
    const [deposit, setDeposit] = useState("");
    const [source, setSource] = useState("phone");
    const [notes, setNotes] = useState("");
    const [loading, setLoading] = useState(false);
    const { invoiceOpen, invoiceData, invoiceLoading, openInvoice, closeInvoice } = useInvoiceDialog();

    const handleInvoice = async () => {
        if (!editBooking) return;
        await openInvoice(editBooking.id);
    };

    const isEditMode = !!editBooking;
    const { availability, loading: checkingAvail, reset: resetAvailability } = useAvailability({
        roomId,
        fromDate: checkInDate,
        toDate: checkOutDate,
        disabled: isEditMode,
        debounceMs: 300,
    });
    const { preview, loading: pricing, error: pricingError } = usePricePreview({
        roomId,
        checkIn: checkInDate,
        checkOut: checkOutDate,
        guests,
        debounceMs: 300,
    });

    useEffect(() => {
        if (open) {
            fetchRooms();
            if (editBooking) {
                // Pre-fill form with existing booking data
                setRoomId(editBooking.room_id);
                setGuestName(editBooking.guest_name);
                setGuestPhone(editBooking.guest_phone || "");
                const cin = editBooking.scheduled_checkin || editBooking.check_in_at.split("T")[0];
                const cout = editBooking.scheduled_checkout || editBooking.expected_checkout.split("T")[0];
                setCheckInDate(cin);
                setCheckOutDate(cout);
                setDeposit(editBooking.deposit_amount ? String(editBooking.deposit_amount) : "");
                setSource(editBooking.source || "phone");
                // Giá trị tạm: nếu `editBooking.guests` là NULL (booking cũ,
                // chưa khai số khách), placeholder trung tính đúng đắn là
                // `max_guests` của phòng — effect bên dưới sẽ nạp lại ngay khi
                // `rooms` có phòng này. `?? 1` chỉ là chỗ giữ chỗ tối thiểu
                // (input có `min={1}`) cho khoảnh khắc trước khi effect đó chạy.
                setGuests(editBooking.guests ?? 1);
                setGuestsTouched(false);
            } else if (prefillDates) {
                // Đến từ kéo-thả trên lịch (Task 8): số đêm không cần tính ở
                // đây — `nights` luôn được suy ra từ checkInDate/checkOutDate
                // qua nightsBetween() ở trên, múi giờ-an toàn như mọi nơi khác
                // trong file này.
                setCheckInDate(prefillDates.checkIn);
                setCheckOutDate(prefillDates.checkOut);
            } else {
                // Mặc định: nhận phòng ngày mai, ở 1 đêm.
                const tomorrow = addDays(new Date().toISOString().split("T")[0], 1);
                setCheckInDate(tomorrow);
                setCheckOutDate(addDays(tomorrow, 1));
            }
        }
    }, [open, editBooking, prefillDates?.checkIn, prefillDates?.checkOut]);

    useEffect(() => {
        if (preSelectedRoomId) setRoomId(preSelectedRoomId);
    }, [preSelectedRoomId]);

    // Tracks the room the guest count was last loaded for, so this effect only
    // reloads on an actual room *change* — not merely because `rooms` got a
    // new array identity (fetchRooms() re-`set`s a fresh array on every
    // unrelated DB write; see useHotelStore.fetchRooms and the
    // "db-updated"/"mcp_reservation_created" listeners in RuntimeStateProvider).
    // Without this guard, any unrelated write elsewhere silently wipes out a
    // guest count the user had just typed in this form.
    const guestsLoadedForRoomId = useRef<string | null>(null);

    useEffect(() => {
        if (isEditMode) return;
        if (!roomId) {
            // Cleared selection: forget what we loaded, so re-picking the same
            // room later is treated as a fresh choice and reloads again.
            guestsLoadedForRoomId.current = null;
            return;
        }
        if (guestsLoadedForRoomId.current === roomId) return;
        const room = rooms.find((r) => r.id === roomId);
        if (room) {
            setGuests(room.max_guests);
            guestsLoadedForRoomId.current = roomId;
        }
    }, [roomId, rooms, isEditMode]);

    // Edit mode, riêng trường hợp booking cũ (guests NULL): hiện max_guests
    // của phòng làm placeholder trung tính, chứ không phải số 2 hardcode.
    // Vô hại về giá — chỉ hiện lên, không tự gửi: handleSubmit chỉ gửi giá
    // trị này khi `guestsTouched`, còn không thì gửi `null` để giữ nguyên
    // NULL đã lưu. Chỉ chạy khi thực sự cần: guests đã lưu là NULL và người
    // dùng chưa đụng vào ô này trong lần sửa này.
    useEffect(() => {
        if (!isEditMode || !editBooking) return;
        if (editBooking.guests != null) return;
        if (guestsTouched) return;
        const room = rooms.find((r) => r.id === editBooking.room_id);
        if (room) setGuests(room.max_guests);
    }, [isEditMode, editBooking, rooms, guestsTouched]);

    async function handleSubmit() {
        if (!roomId || !checkInDate || !checkOutDate) {
            toast.error("Vui lòng điền đầy đủ thông tin");
            return;
        }
        if (nights <= 0) {
            toast.error("Ngày đi phải sau ngày đến");
            return;
        }
        if (nights > MAX_RESERVATION_NIGHTS) {
            // Ô ngày đi cho gõ tay trực tiếp, nên `max` trên input không chặn
            // được — chặn ở đây để một lỗi gõ năm không khoá phòng cả thập kỷ.
            toast.error(`Số đêm không được vượt quá ${MAX_RESERVATION_NIGHTS} đêm`);
            return;
        }
        if (!isEditMode && !guestName) {
            toast.error("Vui lòng nhập tên khách");
            return;
        }
        if (availability && !availability.available) {
            toast.error("Phòng không available trong khoảng ngày này");
            return;
        }
        setLoading(true);
        try {
            const correlationId = createCorrelationId();
            if (isEditMode && editBooking) {
                // Booking cũ (guests NULL) mà người dùng chưa đụng vào ô số
                // khách trong lần sửa này: gửi `null`, không phải một số bịa
                // ra. Backend đọc `new_guests: null` là "giữ nguyên số đã
                // lưu" (modify_reservation_tx: `req.new_guests.or(reservation.guests)`),
                // nên NULL vẫn ở lại NULL và giá không đổi. Người dùng có gõ
                // một số khách mới thì gửi đúng số đó.
                const newGuests =
                    editBooking.guests == null && !guestsTouched ? null : guests;
                await invokeWriteCommand("modify_reservation", {
                    req: {
                        booking_id: editBooking.id,
                        new_check_in_date: checkInDate,
                        new_check_out_date: checkOutDate,
                        new_nights: nights,
                        new_guests: newGuests,
                    },
                }, {
                    correlationId,
                    monitoringContext: {
                        nights,
                        deposit_present: false,
                        source: null,
                        notes_present: false,
                    },
                });
                toast.success("Đã cập nhật đặt phòng!");
            } else {
                const depositAmount =
                    optionalMoneyVnd(deposit.trim() ? Number(deposit) : null, "deposit_amount") ?? null;
                await invokeWriteCommand("create_reservation", {
                    req: {
                        room_id: roomId,
                        guest_name: guestName,
                        guest_phone: guestPhone || null,
                        guest_doc_number: guestDoc || null,
                        check_in_date: checkInDate,
                        check_out_date: checkOutDate,
                        nights,
                        guests,
                        deposit_amount: depositAmount,
                        source,
                        notes: notes || null,
                    },
                }, {
                    correlationId,
                    monitoringContext: {
                        nights,
                        deposit_present: deposit.trim().length > 0,
                        source: source || null,
                        notes_present: notes.trim().length > 0,
                    },
                });
                toast.success("Đặt phòng thành công!");
            }
            resetForm();
            onOpenChange(false);
            fetchRooms();
        } catch (e) {
            toast.error(formatAppError(e));
        }
        setLoading(false);
    }

    function resetForm() {
        setRoomId(preSelectedRoomId || "");
        setGuestName("");
        setGuestPhone("");
        setGuestDoc("");
        setDeposit("");
        setSource("phone");
        setNotes("");
        setGuests(2);
        setGuestsTouched(false);
        resetAvailability();
    }

    const vacantRooms = rooms.filter((r) => r.status === "vacant" || r.status === "booked");

    return (
        <Sheet open={open} onOpenChange={onOpenChange}>
            <SheetContent side="right" className="w-[480px] sm:w-[520px] overflow-y-auto p-0">
                <SheetHeader className="p-6 pb-4 border-b border-slate-100">
                    <SheetTitle className="flex items-center gap-2 text-lg">
                        <CalendarDays size={20} className="text-blue-600" />
                        {isEditMode ? "Chỉnh sửa đặt phòng" : "Đặt phòng trước"}
                    </SheetTitle>
                </SheetHeader>

                <div className="p-6 space-y-5">
                    {/* Room Selection */}
                    <div className="space-y-1.5">
                        <label htmlFor="reservation-room" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">Phòng</label>
                        <select
                            id="reservation-room"
                            className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200 disabled:opacity-60"
                            value={roomId}
                            onChange={(e) => setRoomId(e.target.value)}
                            disabled={isEditMode}
                        >
                            <option value="">— Chọn phòng —</option>
                            {vacantRooms.map((r) => (
                                <option key={r.id} value={r.id}>
                                    {r.name} ({r.type}) — {fmtNumber(r.base_price)}₫/đêm
                                </option>
                            ))}
                        </select>
                    </div>

                    {/* Dates */}
                    <div className="grid grid-cols-2 gap-3">
                        <div className="space-y-1.5">
                            <label htmlFor="reservation-check-in" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">Ngày đến</label>
                            <input
                                id="reservation-check-in"
                                type="date"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200"
                                value={checkInDate}
                                min={new Date().toISOString().split("T")[0]}
                                onChange={(e) => {
                                    const nextCheckIn = e.target.value;
                                    setCheckInDate(nextCheckIn);
                                    // Giữ ngày đi hợp lệ: đẩy nó ra sau ngày đến mới.
                                    if (nextCheckIn && nightsBetween(nextCheckIn, checkOutDate) <= 0) {
                                        setCheckOutDate(addDays(nextCheckIn, 1));
                                    }
                                }}
                            />
                        </div>
                        <div className="space-y-1.5">
                            <label htmlFor="reservation-check-out" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">Ngày đi</label>
                            <input
                                id="reservation-check-out"
                                type="date"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200"
                                value={checkOutDate}
                                min={checkInDate ? addDays(checkInDate, 1) : undefined}
                                max={checkInDate ? addDays(checkInDate, MAX_RESERVATION_NIGHTS) : undefined}
                                onChange={(e) => setCheckOutDate(e.target.value)}
                            />
                        </div>
                    </div>

                    {/* Guests */}
                    <div className="space-y-1.5">
                        <label htmlFor="reservation-guests" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">Số khách</label>
                        <input
                            id="reservation-guests"
                            type="number"
                            min={1}
                            className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200"
                            value={guests}
                            onChange={(e) => {
                                setGuests(Math.max(1, parseInt(e.target.value) || 1));
                                setGuestsTouched(true);
                            }}
                        />
                    </div>

                    {checkInDate && checkOutDate && !datesValid && (
                        <div className="rounded-xl p-3 text-sm bg-red-50 text-red-700 border border-red-200">
                            Ngày đi phải sau ngày đến ít nhất 1 đêm.
                        </div>
                    )}

                    {/* Availability Status */}
                    {!isEditMode && roomId && checkInDate && checkOutDate && (
                        <div className={`rounded-xl p-3 text-sm ${checkingAvail ? "bg-slate-50 text-slate-500" :
                            availability?.available ? "bg-emerald-50 text-emerald-700 border border-emerald-200" :
                                "bg-red-50 text-red-700 border border-red-200"
                            }`}>
                            {checkingAvail ? (
                                "Đang kiểm tra..."
                            ) : availability?.available ? (
                                <span>✅ Phòng available từ {checkInDate} đến {checkOutDate}</span>
                            ) : availability ? (
                                <div className="space-y-1">
                                    <div className="flex items-center gap-1.5 font-semibold">
                                        <AlertTriangle size={14} />
                                        Phòng đã có đặt phòng!
                                    </div>
                                    {availability.conflicts.slice(0, 3).map((c, i) => (
                                        <div key={i} className="text-xs opacity-80">
                                            📅 {c.date} — {c.guest_name || "Đã đặt"} ({c.status})
                                        </div>
                                    ))}
                                    {availability.max_nights != null && availability.max_nights > 0 && (
                                        <div className="text-xs mt-1 font-medium">
                                            💡 Tối đa {availability.max_nights} đêm (check-out trước {availability.conflicts[0]?.date})
                                        </div>
                                    )}
                                </div>
                            ) : null}
                        </div>
                    )}

                    <hr className="border-slate-100" />

                    {/* Guest Info */}
                    <div className="space-y-3">
                        <h3 className="text-xs font-semibold text-slate-500 uppercase tracking-wider flex items-center gap-1.5">
                            <User size={13} /> Thông tin khách
                        </h3>
                        <input
                            placeholder="Họ và tên *"
                            className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200"
                            value={guestName}
                            onChange={(e) => setGuestName(e.target.value)}
                        />
                        <div className="grid grid-cols-2 gap-3">
                            <div className="relative">
                                <Phone size={14} className="absolute left-3 top-1/2 -translate-y-1/2 text-slate-400" />
                                <input
                                    placeholder="Số điện thoại"
                                    className="w-full h-10 pl-9 pr-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200"
                                    value={guestPhone}
                                    onChange={(e) => setGuestPhone(e.target.value)}
                                />
                            </div>
                            <input
                                placeholder="Số CCCD (tùy chọn)"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200"
                                value={guestDoc}
                                onChange={(e) => setGuestDoc(e.target.value)}
                            />
                        </div>
                    </div>

                    <hr className="border-slate-100" />

                    {/* Source & Deposit */}
                    <div className="grid grid-cols-2 gap-3">
                        <div className="space-y-1.5">
                            <label className="text-xs font-semibold text-slate-500 uppercase tracking-wider">Nguồn</label>
                            <select
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200"
                                value={source}
                                onChange={(e) => setSource(e.target.value)}
                            >
                                <option value="phone">Điện thoại</option>
                                <option value="zalo">Zalo</option>
                                <option value="agoda">Agoda</option>
                                <option value="booking.com">Booking.com</option>
                                <option value="walk-in">Walk-in</option>
                                <option value="other">Khác</option>
                            </select>
                        </div>
                        <div className="space-y-1.5">
                            <label className="text-xs font-semibold text-slate-500 uppercase tracking-wider flex items-center gap-1">
                                <CreditCard size={12} /> Tiền cọc
                            </label>
                            <input
                                type="number"
                                placeholder="0"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200"
                                value={deposit}
                                onChange={(e) => setDeposit(e.target.value)}
                            />
                        </div>
                    </div>

                    {/* Notes */}
                    <div className="space-y-1.5">
                        <label className="text-xs font-semibold text-slate-500 uppercase tracking-wider">Ghi chú</label>
                        <textarea
                            placeholder="Ghi chú thêm..."
                            rows={2}
                            className="w-full px-3 py-2 rounded-xl border border-slate-200 bg-slate-50 text-sm resize-none focus:outline-none focus:ring-2 focus:ring-blue-200"
                            value={notes}
                            onChange={(e) => setNotes(e.target.value)}
                        />
                    </div>

                    {/* Price Estimate — con số do engine tính, không phải phép nhân ở đây */}
                    {roomId && datesValid && (
                        <div className="bg-blue-50 rounded-xl p-4 space-y-1">
                            {pricingError ? (
                                <div className="text-sm text-red-600">
                                    Không tính được giá — vui lòng thử lại.
                                </div>
                            ) : pricing && !preview ? (
                                <div className="text-sm text-slate-500">Đang tính giá...</div>
                            ) : preview ? (
                                <>
                                    {preview.breakdown.map((line, i) => (
                                        <div key={i} className="flex justify-between text-sm">
                                            <span className="text-slate-600">{line.label}</span>
                                            <span className="text-slate-700">{fmtNumber(line.amount)}₫</span>
                                        </div>
                                    ))}
                                    <div className="flex justify-between text-sm pt-1 border-t border-blue-100">
                                        <span className="text-slate-600">Tổng</span>
                                        <span className="font-bold text-slate-800">{fmtNumber(preview.total)}₫</span>
                                    </div>
                                    {deposit && parseFloat(deposit) > 0 && (
                                        <div className="flex justify-between text-sm">
                                            <span className="text-slate-600">Tiền cọc</span>
                                            <span className="font-semibold text-emerald-700">-{fmtNumber(parseFloat(deposit))}₫</span>
                                        </div>
                                    )}
                                </>
                            ) : null}
                        </div>
                    )}

                    {/* Invoice button (edit mode only) */}
                    {isEditMode && editBooking && (
                        <Button
                            variant="outline"
                            className="w-full h-10 rounded-xl gap-2 text-sm font-semibold cursor-pointer"
                            onClick={handleInvoice}
                            disabled={invoiceLoading}
                        >
                            <FileText className="w-4 h-4" />
                            {invoiceLoading ? "Đang tạo..." : "📄 Invoice"}
                        </Button>
                    )}

                    {/* Submit */}
                    <Button
                        className="w-full h-12 rounded-xl bg-blue-600 hover:bg-blue-700 text-white font-semibold text-sm cursor-pointer"
                        onClick={handleSubmit}
                        disabled={loading || (!isEditMode && !guestName) || !roomId || !datesValid || (availability !== null && !availability.available)}
                    >
                        {loading ? "Đang xử lý..." : isEditMode ? "💾 Lưu thay đổi" : "📅 Đặt phòng"}
                    </Button>
                </div>
            </SheetContent>

            {/* Invoice Dialog */}
            <InvoiceDialog
                open={invoiceOpen}
                onOpenChange={(nextOpen) => {
                    if (!nextOpen) closeInvoice();
                }}
                data={invoiceData}
            />
        </Sheet>
    );
}
