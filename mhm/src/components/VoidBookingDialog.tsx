import { useEffect, useState } from "react";
import { toast } from "sonner";

import HoldToDeleteButton from "@/components/shared/HoldToDeleteButton";
import { Button } from "@/components/ui/button";
import { formatAppError } from "@/lib/appError";
import { fmtMoney } from "@/lib/format";
import { invokeCommand } from "@/lib/invokeCommand";
import { useHotelStore } from "@/stores/useHotelStore";
import type { VoidBookingPreview } from "@/types";

const REASONS = ["Bấm nhầm", "Nhập trùng", "Khách không đến", "Khác"];

function fmtDateOnly(value: string): string {
    const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
    if (!match) return value;
    const [, year, month, day] = match;
    return `${day}/${month}/${year}`;
}

interface VoidBookingDialogProps {
    bookingId: string;
    onClose: () => void;
    onVoided: () => void;
}

/**
 * Hộp xác nhận hai bước: mở ra để ĐỌC hậu quả, giữ nút để THỰC HIỆN.
 *
 * Mọi con số ở đây do `preview_void_booking` trả về. Không tính lại ở frontend:
 * tính hai nơi là cách chắc chắn nhất để hộp thoại hứa một số mà backend gỡ đi
 * một số khác.
 */
export default function VoidBookingDialog({ bookingId, onClose, onVoided }: VoidBookingDialogProps) {
    const voidBooking = useHotelStore((state) => state.voidBooking);
    const [preview, setPreview] = useState<VoidBookingPreview | null>(null);
    const [loadError, setLoadError] = useState<string | null>(null);
    const [reason, setReason] = useState<string>(REASONS[0]);
    const [busy, setBusy] = useState(false);

    useEffect(() => {
        let cancelled = false;
        invokeCommand<VoidBookingPreview>("preview_void_booking", { bookingId })
            .then((result) => {
                if (!cancelled) setPreview(result);
            })
            .catch((err) => {
                if (!cancelled) setLoadError(formatAppError(err));
            });
        return () => {
            cancelled = true;
        };
    }, [bookingId]);

    const handleVoid = async () => {
        if (busy) return;
        setBusy(true);
        try {
            await voidBooking(bookingId, reason);
            toast.success("Đã xóa lượt nhập sai");
            onVoided();
        } catch (err) {
            toast.error("Lỗi xóa lượt: " + formatAppError(err));
        } finally {
            setBusy(false);
        }
    };

    // `void_booking_tx` (services/booking/void_lifecycle.rs) chỉ UPDATE bảng
    // rooms cho status active/checked_out — nhánh booked là `_ => {}`, không
    // đụng gì tới phòng. `room_was_reused` cũng chỉ được backend tính cho
    // checked_out (luôn false với booked/active), nên không thể chỉ nhìn
    // `!room_was_reused` mà kết luận phòng "sẽ về trống": với một lượt mới đặt
    // trước, câu đó bịa ra một hiệu ứng backend không hề làm.
    const roomWillBecomeVacant =
        preview !== null &&
        (preview.previous_status === "active" || preview.previous_status === "checked_out") &&
        !preview.room_was_reused;

    return (
        <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/40" onClick={onClose}>
            <div
                className="bg-white rounded-2xl shadow-2xl p-6 w-[400px] space-y-4"
                onClick={(event) => event.stopPropagation()}
            >
                {loadError ? (
                    <p className="text-sm text-red-600">{loadError}</p>
                ) : !preview ? (
                    <p className="text-sm text-slate-500">Đang tải…</p>
                ) : (
                    <>
                        <h3 className="font-bold text-lg text-slate-800">
                            Xóa lượt ở của {preview.guest_name}?
                        </h3>

                        <ul className="space-y-1.5 text-sm text-slate-600">
                            {/* void_booking_tx từ chối thẳng mọi booking có group_id
                                ("Lượt này thuộc đoàn — chưa hỗ trợ xóa từng phòng"),
                                không điều kiện — giữ đủ 2 giây rồi mới báo lỗi là một
                                trải nghiệm tệ khi preview đã biết trước kết quả. */}
                            {preview.is_group_booking && (
                                <li data-testid="void-group-booking-warning" className="text-red-700">
                                    ⚠️ Lượt này thuộc một đoàn — chưa hỗ trợ xóa từng phòng
                                    trong đoàn ở đây
                                </li>
                            )}
                            {roomWillBecomeVacant && (
                                <li data-testid="void-room-vacant-note">
                                    Phòng <strong>{preview.room_id}</strong> → về trạng thái{" "}
                                    <strong>Trống</strong>
                                </li>
                            )}
                            {preview.revenue_impact > 0 && (
                                <li data-testid="void-revenue-impact">
                                    Gỡ <strong>{fmtMoney(preview.revenue_impact)}</strong> khỏi doanh
                                    thu ngày <strong>{fmtDateOnly(preview.revenue_date)}</strong>
                                    {preview.previous_status === "active" &&
                                        ` (đã ghi nhận ${preview.nights_recognized}/${preview.nights_total} đêm)`}
                                </li>
                            )}
                            {preview.deposit_amount > 0 && (
                                <li data-testid="void-deposit-note">
                                    Tiền cọc <strong>{fmtMoney(preview.deposit_amount)}</strong> vẫn
                                    nằm trong sổ thu — xóa lượt không hoàn cọc, cần xử lý riêng với khách
                                </li>
                            )}
                            {preview.is_audited && (
                                <li data-testid="void-audited-warning" className="text-amber-700">
                                    ⚠️ Ngày này đã chốt kiểm toán đêm — số liệu ngày{" "}
                                    {fmtDateOnly(preview.revenue_date)} sẽ thay đổi
                                </li>
                            )}
                            {preview.room_was_reused && (
                                <li data-testid="void-room-reused-note" className="text-slate-500">
                                    ℹ️ Phòng {preview.room_id} hiện đang có khách khác — trạng thái
                                    phòng giữ nguyên
                                </li>
                            )}
                        </ul>

                        <label className="block text-sm text-slate-600">
                            Lý do
                            <select
                                className="mt-1 w-full rounded-lg border border-slate-200 px-2 h-9 text-sm"
                                value={reason}
                                onChange={(event) => setReason(event.target.value)}
                            >
                                {REASONS.map((item) => (
                                    <option key={item} value={item}>
                                        {item}
                                    </option>
                                ))}
                            </select>
                        </label>

                        <div className="space-y-2">
                            <HoldToDeleteButton
                                label={
                                    busy
                                        ? "Đang xóa…"
                                        : preview.is_group_booking
                                          ? "Không thể xóa — thuộc đoàn"
                                          : "Giữ 2 giây để xóa"
                                }
                                disabled={busy || preview.is_group_booking}
                                onHoldComplete={handleVoid}
                            />
                            <Button
                                variant="outline"
                                className="w-full rounded-xl h-10 cursor-pointer"
                                onClick={onClose}
                            >
                                Thôi
                            </Button>
                        </div>
                    </>
                )}
            </div>
        </div>
    );
}
