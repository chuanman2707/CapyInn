import { CheckCircle2, XCircle, Pencil, FileText, ArrowRightLeft } from "lucide-react";

import { Button } from "@/components/ui/button";
import { fmtDate, fmtDateShort, fmtNumber } from "@/lib/format";
import type { BookingWithGuest } from "@/types";

interface BookingDetailPopupProps {
    booking: BookingWithGuest;
    mode: "reservation" | "readonly";
    onClose: () => void;
    onConfirm?: (bookingId: string) => void;
    onEdit?: (booking: BookingWithGuest) => void;
    onCancel?: (bookingId: string) => void;
    onViewInvoice?: (bookingId: string) => void;
    onRoomChange?: (bookingId: string) => void;
    invoiceLoading?: boolean;
}

/**
 * Ngày trơn `YYYY-MM-DD` được ghép bằng cách tách chuỗi thay vì dựng `Date` —
 * `new Date("YYYY-MM-DD")` bị hiểu là nửa đêm UTC nên có thể lùi một ngày ở
 * các múi giờ phía tây UTC. Định dạng đầu ra khớp với fmtDateShort (dd/MM/yyyy).
 */
function fmtDateOnlyLocal(value: string): string {
    const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
    if (!match) return fmtDateShort(value);
    const [, year, month, day] = match;
    return `${day}/${month}/${year}`;
}

/**
 * Backend trả về hai dạng: ngày trơn `YYYY-MM-DD` (lịch đặt trước) và timestamp
 * RFC3339 (check-in / trả phòng thực tế). Ngày trơn phải đi qua fmtDateOnlyLocal —
 * fmtDate sẽ hiểu nó là nửa đêm UTC và in thêm giờ ảo, thậm chí lùi một ngày ở
 * các múi giờ phía tây UTC.
 */
function fmtCheckpoint(value: string): string {
    if (!value) return value;
    return value.length <= 10 ? fmtDateOnlyLocal(value) : fmtDate(value);
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
    onRoomChange,
    invoiceLoading,
}: BookingDetailPopupProps) {
    const isReadonly = mode === "readonly";
    const title = isReadonly
        ? `Đã trả — ${booking.guest_name}`
        : `Reservation — ${booking.guest_name}`;
    const checkInText = isReadonly
        ? booking.check_in_at ?? booking.scheduled_checkin
        : booking.scheduled_checkin || booking.check_in_at;
    const checkOutText = isReadonly
        ? booking.actual_checkout || booking.scheduled_checkout || booking.expected_checkout
        : booking.scheduled_checkout || booking.expected_checkout;

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30" onClick={onClose}>
            <div className="bg-white rounded-2xl shadow-2xl p-6 w-[380px] space-y-4" onClick={(e) => e.stopPropagation()}>
                <h3 className="font-bold text-lg text-slate-800">{title}</h3>
                <div className="space-y-2 text-sm text-slate-600">
                    <Row label="Phòng" value={booking.room_id} />
                    <Row label="Check-in" value={fmtCheckpoint(checkInText)} />
                    <Row label={isReadonly ? "Trả phòng lúc" : "Check-out"} value={fmtCheckpoint(checkOutText)} />
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
                            disabled={invoiceLoading}
                        >
                            <FileText size={14} /> {invoiceLoading ? "Đang tạo..." : "Xem hóa đơn"}
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
                    <div className="space-y-2 pt-2">
                        <div className="flex gap-2">
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
                        {/* Chỉ hiện khi còn đổi được phòng — không cho với booking đã trả. */}
                        {(booking.status === "active" || booking.status === "booked") && (
                            <Button
                                variant="outline"
                                className="w-full border-indigo-200 text-indigo-600 hover:bg-indigo-50 rounded-xl h-10 gap-1.5 cursor-pointer"
                                onClick={() => onRoomChange?.(booking.id)}
                            >
                                <ArrowRightLeft size={14} /> Chuyển phòng
                            </Button>
                        )}
                    </div>
                )}
            </div>
        </div>
    );
}
