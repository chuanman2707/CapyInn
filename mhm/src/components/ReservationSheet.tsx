import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useHotelStore } from "../stores/useHotelStore";
import { CalendarDays, User, Phone, CreditCard, AlertTriangle, FileText } from "lucide-react";
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";
import { useAvailability } from "@/hooks/useAvailability";
import { useInvoiceDialog } from "@/hooks/useInvoiceDialog";
import { formatAppError } from "@/lib/appError";
import { createCorrelationId } from "@/lib/correlationId";
import { fmtNumber } from "@/lib/format";
import { createIdempotencyKey, invokeCommand } from "@/lib/invokeCommand";
import { toast } from "sonner";
import InvoiceDialog from "./InvoiceDialog";
import type { EditableBooking } from "@/types";

interface Props {
    open: boolean;
    onOpenChange: (v: boolean) => void;
    preSelectedRoomId?: string;
    editBooking?: EditableBooking;
}

export default function ReservationSheet({ open, onOpenChange, preSelectedRoomId, editBooking }: Props) {
    const { rooms, fetchRooms } = useHotelStore();
    const [roomId, setRoomId] = useState(preSelectedRoomId || "");
    const [guestName, setGuestName] = useState("");
    const [guestPhone, setGuestPhone] = useState("");
    const [guestDoc, setGuestDoc] = useState("");
    const [checkInDate, setCheckInDate] = useState("");
    const [checkOutDate, setCheckOutDate] = useState("");
    const [nights, setNights] = useState(1);
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
                setNights(editBooking.nights);
                setDeposit(editBooking.deposit_amount ? String(editBooking.deposit_amount) : "");
                setSource(editBooking.source || "phone");
            } else {
                // Set default check-in date to tomorrow
                const tomorrow = new Date();
                tomorrow.setDate(tomorrow.getDate() + 1);
                setCheckInDate(tomorrow.toISOString().split("T")[0]);
                updateCheckout(tomorrow.toISOString().split("T")[0], 1);
            }
        }
    }, [open, editBooking]);

    useEffect(() => {
        if (preSelectedRoomId) setRoomId(preSelectedRoomId);
    }, [preSelectedRoomId]);

    function updateCheckout(cin: string, n: number) {
        if (!cin) return;
        const d = new Date(cin);
        d.setDate(d.getDate() + n);
        setCheckOutDate(d.toISOString().split("T")[0]);
    }

    async function handleSubmit() {
        if (!roomId || !checkInDate || !checkOutDate) {
            toast.error("Vui lòng điền đầy đủ thông tin");
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
            if (isEditMode && editBooking) {
                await invoke("modify_reservation", {
                    req: {
                        booking_id: editBooking.id,
                        new_check_in_date: checkInDate,
                        new_check_out_date: checkOutDate,
                        new_nights: nights,
                    },
                });
                toast.success("Đã cập nhật đặt phòng!");
            } else {
                const correlationId = createCorrelationId();
                await invokeCommand("create_reservation", {
                    req: {
                        room_id: roomId,
                        guest_name: guestName,
                        guest_phone: guestPhone || null,
                        guest_doc_number: guestDoc || null,
                        check_in_date: checkInDate,
                        check_out_date: checkOutDate,
                        nights,
                        deposit_amount: deposit ? parseFloat(deposit) : null,
                        source,
                        notes: notes || null,
                    },
                    idempotencyKey: createIdempotencyKey("create_reservation"),
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
        setNights(1);
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
                        <label className="text-xs font-semibold text-slate-500 uppercase tracking-wider">Phòng</label>
                        <select
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
                            <label className="text-xs font-semibold text-slate-500 uppercase tracking-wider">Ngày đến</label>
                            <input
                                type="date"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200"
                                value={checkInDate}
                                min={new Date().toISOString().split("T")[0]}
                                onChange={(e) => {
                                    setCheckInDate(e.target.value);
                                    updateCheckout(e.target.value, nights);
                                }}
                            />
                        </div>
                        <div className="space-y-1.5">
                            <label className="text-xs font-semibold text-slate-500 uppercase tracking-wider">Ngày đi</label>
                            <input
                                type="date"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200"
                                value={checkOutDate}
                                readOnly
                            />
                        </div>
                    </div>

                    {/* Nights */}
                    <div className="space-y-1.5">
                        <label className="text-xs font-semibold text-slate-500 uppercase tracking-wider">Số đêm</label>
                        <input
                            type="number"
                            min={1}
                            max={90}
                            className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200"
                            value={nights}
                            onChange={(e) => {
                                const n = Math.max(1, parseInt(e.target.value) || 1);
                                setNights(n);
                                updateCheckout(checkInDate, n);
                            }}
                        />
                    </div>

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

                    {/* Price Estimate */}
                    {roomId && nights > 0 && (
                        <div className="bg-blue-50 rounded-xl p-4 space-y-1">
                            <div className="flex justify-between text-sm">
                                <span className="text-slate-600">Giá phòng × {nights} đêm</span>
                                <span className="font-bold text-slate-800">
                                    {fmtNumber((rooms.find((r) => r.id === roomId)?.base_price || 0) * nights)}₫
                                </span>
                            </div>
                            {deposit && parseFloat(deposit) > 0 && (
                                <div className="flex justify-between text-sm">
                                    <span className="text-slate-600">Tiền cọc</span>
                                    <span className="font-semibold text-emerald-700">-{fmtNumber(parseFloat(deposit))}₫</span>
                                </div>
                            )}
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
                        disabled={loading || (!isEditMode && !guestName) || !roomId || (availability !== null && !availability.available)}
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
