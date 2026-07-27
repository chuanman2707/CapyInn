import { CheckCircle2, XCircle, Pencil, FileText } from "lucide-react";

import { Button } from "@/components/ui/button";
import { fmtNumber } from "@/lib/format";
import type { BookingWithGuest } from "@/types";

interface BookingDetailPopupProps {
    booking: BookingWithGuest;
    mode: "reservation" | "readonly";
    onClose: () => void;
    onConfirm?: (bookingId: string) => void;
    onEdit?: (booking: BookingWithGuest) => void;
    onCancel?: (bookingId: string) => void;
    onViewInvoice?: (bookingId: string) => void;
}

function Row({ label, value }: { label: string; value: string }) {
    return (
        <div className="flex justify-between">
            <span>{label}</span>
            <span className="font-semibold">{value}</span>
        </div>
    );
}

export default function BookingDetailPopup({
    booking,
    mode,
    onClose,
    onConfirm,
    onEdit,
    onCancel,
    onViewInvoice,
}: BookingDetailPopupProps) {
    const isReadonly = mode === "readonly";
    const title = isReadonly
        ? `Đã trả — ${booking.guest_name}`
        : `Reservation — ${booking.guest_name}`;
    const checkInText = booking.scheduled_checkin || booking.check_in_at;
    const checkOutText = isReadonly
        ? booking.actual_checkout || booking.scheduled_checkout || booking.expected_checkout
        : booking.scheduled_checkout || booking.expected_checkout;

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30" onClick={onClose}>
            <div className="bg-white rounded-2xl shadow-2xl p-6 w-[380px] space-y-4" onClick={(e) => e.stopPropagation()}>
                <h3 className="font-bold text-lg text-slate-800">{title}</h3>
                <div className="space-y-2 text-sm text-slate-600">
                    <Row label="Phòng" value={booking.room_id} />
                    <Row label="Check-in" value={checkInText} />
                    <Row label={isReadonly ? "Trả phòng lúc" : "Check-out"} value={checkOutText} />
                    <Row label="Số đêm" value={String(booking.nights)} />
                    <div className="flex justify-between">
                        <span>Tổng tiền</span>
                        <span className="font-bold text-slate-800">{fmtNumber(booking.total_price)}₫</span>
                    </div>
                    {isReadonly ? (
                        <div className="flex justify-between">
                            <span>Đã thanh toán</span>
                            <span className="font-semibold text-emerald-600">{fmtNumber(booking.paid_amount)}₫</span>
                        </div>
                    ) : (
                        (booking.deposit_amount || 0) > 0 && (
                            <div className="flex justify-between">
                                <span>Đã cọc</span>
                                <span className="font-semibold text-emerald-600">{fmtNumber(booking.deposit_amount || 0)}₫</span>
                            </div>
                        )
                    )}
                    {booking.guest_phone && <Row label="SĐT" value={booking.guest_phone} />}
                </div>

                {isReadonly ? (
                    <div className="flex gap-2 pt-2">
                        <Button
                            className="flex-1 bg-slate-700 hover:bg-slate-800 text-white rounded-xl h-10 gap-1.5 cursor-pointer"
                            onClick={() => onViewInvoice?.(booking.id)}
                        >
                            <FileText size={14} /> Xem hóa đơn
                        </Button>
                        <Button
                            variant="outline"
                            className="flex-1 border-slate-200 text-slate-600 hover:bg-slate-50 rounded-xl h-10 cursor-pointer"
                            onClick={onClose}
                        >
                            Đóng
                        </Button>
                    </div>
                ) : (
                    <div className="flex gap-2 pt-2">
                        <Button
                            className="flex-1 bg-emerald-600 hover:bg-emerald-700 text-white rounded-xl h-10 gap-1.5 cursor-pointer"
                            onClick={() => onConfirm?.(booking.id)}
                        >
                            <CheckCircle2 size={14} /> Check-in
                        </Button>
                        <Button
                            variant="outline"
                            className="flex-1 border-blue-200 text-blue-600 hover:bg-blue-50 rounded-xl h-10 gap-1.5 cursor-pointer"
                            onClick={() => onEdit?.(booking)}
                        >
                            <Pencil size={14} /> Chỉnh sửa
                        </Button>
                        <Button
                            variant="outline"
                            className="flex-1 border-red-200 text-red-600 hover:bg-red-50 rounded-xl h-10 gap-1.5 cursor-pointer"
                            onClick={() => onCancel?.(booking.id)}
                        >
                            <XCircle size={14} /> Hủy
                        </Button>
                    </div>
                )}
            </div>
        </div>
    );
}
