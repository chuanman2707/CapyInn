import { useEffect, useState } from "react";
import { useHotelStore } from "../stores/useHotelStore";
import { History } from "lucide-react";
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";
import { usePricePreview } from "@/hooks/usePricePreview";
import { formatAppError } from "@/lib/appError";
import { createCorrelationId } from "@/lib/correlationId";
import { invokeWriteCommand } from "@/lib/invokeCommand";
import { getRoomTypeLabel } from "@/lib/constants";
import { fmtNumber } from "@/lib/format";
import { toast } from "sonner";

export interface BackfillPrefill {
    roomId: string;
    checkInDate: string;
    checkOutDate: string;
    stillStaying: boolean;
}

interface Props {
    open: boolean;
    onOpenChange: (v: boolean) => void;
    prefill?: BackfillPrefill;
}

// Ngày dạng "YYYY-MM-DD" (không giờ) được JS phân giải theo UTC — dùng nguyên
// dạng đó (không thêm "T00:00:00") để tránh lệch ngày ở múi giờ dương như
// Asia/Ho_Chi_Minh khi quy đổi qua toISOString(). Cùng cách làm với
// ReservationSheet.tsx.
function nightsBetween(checkIn: string, checkOut: string): number {
    if (!checkIn || !checkOut) return 0;
    const ms = new Date(checkOut).getTime() - new Date(checkIn).getTime();
    if (Number.isNaN(ms)) return 0;
    return Math.round(ms / 86_400_000);
}

export default function BackfillSheet({ open, onOpenChange, prefill }: Props) {
    const { rooms, fetchRooms } = useHotelStore();
    const [roomId, setRoomId] = useState("");
    const [guestName, setGuestName] = useState("");
    const [guestPhone, setGuestPhone] = useState("");
    const [guestDoc, setGuestDoc] = useState("");
    const [checkInDate, setCheckInDate] = useState("");
    const [checkOutDate, setCheckOutDate] = useState("");
    const [stillStaying, setStillStaying] = useState(false);
    const [total, setTotal] = useState(0);
    const [totalDirty, setTotalDirty] = useState(false);
    const [paid, setPaid] = useState(0);
    const [paidDirty, setPaidDirty] = useState(false);
    const [source, setSource] = useState("walk-in");
    const [notes, setNotes] = useState("");
    const [submitting, setSubmitting] = useState(false);

    // Không phụ thuộc vào *reference* của `prefill` — cha (Reservations.tsx)
    // dựng object này inline mỗi lần render, nên phụ thuộc vào các trường
    // nguyên thuỷ bên trong mới tránh được việc effect chạy lại (và xoá dữ
    // liệu người dùng vừa gõ) chỉ vì cha re-render vì lý do khác. Cùng bài
    // học đã áp dụng cho prefillDates ở ReservationSheet.tsx.
    useEffect(() => {
        if (!open) return;
        fetchRooms();
        setRoomId(prefill?.roomId ?? "");
        setCheckInDate(prefill?.checkInDate ?? "");
        setCheckOutDate(prefill?.checkOutDate ?? "");
        setStillStaying(prefill?.stillStaying ?? false);
        setGuestName("");
        setGuestPhone("");
        setGuestDoc("");
        setTotal(0);
        setTotalDirty(false);
        setPaid(0);
        setPaidDirty(false);
        setSource("walk-in");
        setNotes("");
        setSubmitting(false);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [open, prefill?.roomId, prefill?.checkInDate, prefill?.checkOutDate, prefill?.stillStaying]);

    const nights = nightsBetween(checkInDate, checkOutDate);
    const datesValid = nights > 0;

    // Form ghi bù chỉ có một khách chính (không có ô "số khách" như
    // ReservationSheet) — engine giá được gọi với guests: 1 cho đúng số
    // người thực sự ở, không phải một hằng số bịa ra.
    const { preview, loading: pricingLoading, error: pricingError } = usePricePreview({
        roomId,
        checkIn: checkInDate,
        checkOut: checkOutDate,
        guests: 1,
        debounceMs: 200,
    });

    // Gợi ý theo bảng giá cho tới khi chủ sửa tay ô Tiền phòng.
    useEffect(() => {
        if (!totalDirty && preview) setTotal(preview.total);
    }, [preview, totalDirty]);

    // Khách đã trả phòng: mặc định đã thu đủ, cho tới khi chủ sửa tay.
    // Khách còn ở: không giả định đã thu đủ (khách chưa trả phòng).
    useEffect(() => {
        if (!paidDirty && !stillStaying) setPaid(total);
    }, [total, paidDirty, stillStaying]);

    // Khách còn ở chỉ ghi bù được vào phòng đang trống — backend enforce lại
    // rule này, đây chỉ là phản ánh lên danh sách chọn cho khỏi chọn nhầm.
    const selectableRooms = stillStaying ? rooms.filter((r) => r.status === "vacant") : rooms;

    const paidTooHigh = paid > total;
    const paidNegative = paid < 0;
    const canSubmit =
        !!roomId &&
        guestName.trim().length > 0 &&
        !!checkInDate &&
        !!checkOutDate &&
        datesValid &&
        !paidTooHigh &&
        !paidNegative &&
        !submitting;

    async function handleSubmit() {
        if (!canSubmit) return;
        setSubmitting(true);
        try {
            const correlationId = createCorrelationId();
            await invokeWriteCommand(
                "backfill_stay",
                {
                    req: {
                        room_id: roomId,
                        guests: [
                            {
                                full_name: guestName,
                                doc_number: guestDoc,
                                phone: guestPhone,
                                dob: "",
                                gender: "Nam",
                                nationality: "Việt Nam",
                                address: "",
                            },
                        ],
                        check_in_date: checkInDate,
                        check_out_date: stillStaying ? null : checkOutDate,
                        expected_checkout_date: stillStaying ? checkOutDate : null,
                        total_price: total,
                        paid_amount: paid,
                        source,
                        notes: notes || null,
                    },
                },
                { correlationId },
            );
            toast.success(stillStaying ? "Đã ghi bù khách đang ở!" : "Đã ghi bù khách đã trả phòng!");
            onOpenChange(false);
            fetchRooms();
        } catch (e) {
            toast.error(formatAppError(e));
        }
        setSubmitting(false);
    }

    return (
        <Sheet open={open} onOpenChange={onOpenChange}>
            <SheetContent side="right" className="w-[480px] sm:w-[520px] overflow-y-auto p-0">
                <SheetHeader className="p-6 pb-4 border-b border-slate-100">
                    <SheetTitle className="flex items-center gap-2 text-lg">
                        <History size={20} className="text-amber-600" />
                        Ghi bù sổ khách
                    </SheetTitle>
                    <p className="text-sm text-slate-500">
                        Ghi lại khách đã ở mà quên nhập — khách sẽ vào danh sách khai báo tạm trú.
                    </p>
                </SheetHeader>

                <div className="p-6 space-y-5">
                    <div className="space-y-1.5">
                        <label htmlFor="backfill-room" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">
                            Phòng
                        </label>
                        <select
                            id="backfill-room"
                            className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-amber-200"
                            value={roomId}
                            onChange={(e) => setRoomId(e.target.value)}
                        >
                            <option value="">— Chọn phòng —</option>
                            {selectableRooms.map((r) => (
                                <option key={r.id} value={r.id}>
                                    {r.name} ({getRoomTypeLabel(r.type)})
                                </option>
                            ))}
                        </select>
                    </div>

                    <label className="flex items-center gap-2 text-sm font-medium text-slate-700 cursor-pointer">
                        <input
                            type="checkbox"
                            checked={stillStaying}
                            onChange={(e) => setStillStaying(e.target.checked)}
                        />
                        Khách còn ở (chưa trả phòng)
                    </label>

                    <div className="grid grid-cols-2 gap-3">
                        <div className="space-y-1.5">
                            <label htmlFor="backfill-checkin" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">
                                Ngày vào
                            </label>
                            <input
                                id="backfill-checkin"
                                type="date"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-amber-200"
                                value={checkInDate}
                                onChange={(e) => setCheckInDate(e.target.value)}
                            />
                        </div>
                        <div className="space-y-1.5">
                            <label htmlFor="backfill-checkout" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">
                                {stillStaying ? "Ngày ra dự kiến" : "Ngày ra"}
                            </label>
                            <input
                                id="backfill-checkout"
                                type="date"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-amber-200"
                                value={checkOutDate}
                                onChange={(e) => setCheckOutDate(e.target.value)}
                            />
                        </div>
                    </div>

                    {checkInDate && checkOutDate && !datesValid && (
                        <div className="rounded-xl p-3 text-sm bg-red-50 text-red-700 border border-red-200">
                            Ngày ra phải sau ngày vào ít nhất 1 đêm.
                        </div>
                    )}

                    <div className="space-y-3">
                        <h3 className="text-xs font-semibold text-slate-500 uppercase tracking-wider">Thông tin khách</h3>
                        <input
                            placeholder="Họ và tên *"
                            className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-amber-200"
                            value={guestName}
                            onChange={(e) => setGuestName(e.target.value)}
                        />
                        <div className="grid grid-cols-2 gap-3">
                            <input
                                placeholder="Số điện thoại"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-amber-200"
                                value={guestPhone}
                                onChange={(e) => setGuestPhone(e.target.value)}
                            />
                            <input
                                placeholder="Số CCCD (tùy chọn)"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-amber-200"
                                value={guestDoc}
                                onChange={(e) => setGuestDoc(e.target.value)}
                            />
                        </div>
                    </div>

                    <div className="grid grid-cols-2 gap-3">
                        <div className="space-y-1.5">
                            <label htmlFor="backfill-total" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">
                                Tiền phòng ({nights} đêm)
                            </label>
                            <input
                                id="backfill-total"
                                type="number"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-amber-200"
                                value={total}
                                onChange={(e) => {
                                    setTotal(Number(e.target.value) || 0);
                                    setTotalDirty(true);
                                }}
                            />
                            {pricingLoading && !preview && (
                                <p className="text-xs text-slate-400">Đang tính giá gợi ý...</p>
                            )}
                            {pricingError && (
                                <p className="text-xs text-red-500">Không tính được giá gợi ý — có thể sửa tay.</p>
                            )}
                        </div>
                        <div className="space-y-1.5">
                            <label htmlFor="backfill-paid" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">
                                Đã thu
                            </label>
                            <input
                                id="backfill-paid"
                                type="number"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-amber-200"
                                value={paid}
                                onChange={(e) => {
                                    setPaid(Number(e.target.value) || 0);
                                    setPaidDirty(true);
                                }}
                            />
                        </div>
                    </div>
                    {(paidTooHigh || paidNegative) && (
                        <p className="text-xs text-red-600 font-medium">
                            {paidTooHigh
                                ? `Đã thu (${fmtNumber(paid)}₫) không được vượt quá tiền phòng (${fmtNumber(total)}₫).`
                                : "Đã thu không được nhỏ hơn 0."}
                        </p>
                    )}

                    <div className="grid grid-cols-2 gap-3">
                        <div className="space-y-1.5">
                            <label htmlFor="backfill-source" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">
                                Nguồn
                            </label>
                            <select
                                id="backfill-source"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-amber-200"
                                value={source}
                                onChange={(e) => setSource(e.target.value)}
                            >
                                <option value="walk-in">Walk-in</option>
                                <option value="phone">Điện thoại</option>
                                <option value="zalo">Zalo</option>
                                <option value="agoda">Agoda</option>
                                <option value="booking.com">Booking.com</option>
                                <option value="other">Khác</option>
                            </select>
                        </div>
                        <div className="space-y-1.5">
                            <label htmlFor="backfill-notes" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">
                                Ghi chú
                            </label>
                            <input
                                id="backfill-notes"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-amber-200"
                                value={notes}
                                onChange={(e) => setNotes(e.target.value)}
                            />
                        </div>
                    </div>

                    <Button
                        className="w-full h-12 rounded-xl bg-amber-600 hover:bg-amber-700 text-white font-semibold text-sm cursor-pointer"
                        onClick={handleSubmit}
                        disabled={!canSubmit}
                    >
                        {submitting ? "Đang xử lý..." : "Ghi bù vào sổ"}
                    </Button>
                </div>
            </SheetContent>
        </Sheet>
    );
}
