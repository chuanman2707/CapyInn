import { useState } from "react";
import { CalendarDays, Check, FileText, Pencil, X } from "lucide-react";

import InfoItem from "@/components/shared/InfoItem";
import Section from "@/components/shared/Section";
import { Button } from "@/components/ui/button";
import { isFullySettled } from "@/lib/bookingBalance";
import { fmtDateShort, fmtMoney } from "@/lib/format";
import type { Booking } from "@/types";

const MAX_RATE_PER_NIGHT = 100_000_000;

interface BookingSummaryProps {
    booking: Booking;
    onInvoice: () => void;
    invoiceLoading: boolean;
    onSaveRate?: (ratePerNight: number) => Promise<void>;
}

export default function BookingSummary({
    booking,
    onInvoice,
    invoiceLoading,
    onSaveRate,
}: BookingSummaryProps) {
    const [editingRate, setEditingRate] = useState(false);
    const [rateInput, setRateInput] = useState("");
    const [savingRate, setSavingRate] = useState(false);

    const paymentStatusClass = isFullySettled(booking.total_price, booking.paid_amount)
        ? "text-emerald-600"
        : "text-orange-600";

    // `total_price` là cột duy nhất được lưu — không có cột giá/đêm. Giá hiển
    // thị luôn suy ra bằng phép chia, làm tròn xuống VNĐ nguyên. Khi phụ thu
    // cuối tuần/lễ khiến các đêm không đồng giá, phép chia không hết — nhãn
    // phải nói rõ đây là số trung bình (TB), không phải giá cố định.
    const nights = booking.nights > 0 ? booking.nights : 1;
    const derivedRate = Math.floor(booking.total_price / nights);
    const isAverage = booking.total_price % nights !== 0;

    const parsedRate = Number(rateInput);
    const rateValid =
        Number.isInteger(parsedRate) && parsedRate > 0 && parsedRate <= MAX_RATE_PER_NIGHT;

    const openRateEditor = () => {
        setRateInput(String(derivedRate));
        setEditingRate(true);
    };

    const closeRateEditor = () => {
        setEditingRate(false);
        setRateInput("");
    };

    const submitRate = async () => {
        if (!onSaveRate || !rateValid || savingRate) return;
        setSavingRate(true);
        try {
            await onSaveRate(parsedRate);
            closeRateEditor();
        } finally {
            setSavingRate(false);
        }
    };

    return (
        <Section
            icon={CalendarDays}
            title="Booking hiện tại"
            className="bg-slate-50 rounded-2xl p-5 space-y-3"
        >
            <div className="grid grid-cols-2 gap-3">
                <InfoItem label="Check-in" value={fmtDateShort(booking.check_in_at)} />
                <InfoItem label="Checkout" value={fmtDateShort(booking.expected_checkout)} />
                <InfoItem label="Số đêm" value={booking.nights} />
                <InfoItem label="Tổng tiền" value={fmtMoney(booking.total_price)} />
                <InfoItem
                    label={isAverage ? "Giá/đêm (TB)" : "Giá/đêm"}
                    value={
                        <span className="inline-flex items-center gap-1.5">
                            {fmtMoney(derivedRate)}
                            {onSaveRate && !editingRate && (
                                <button
                                    type="button"
                                    aria-label="Sửa giá"
                                    onClick={openRateEditor}
                                    className="text-slate-400 hover:text-blue-600 cursor-pointer"
                                >
                                    <Pencil size={12} />
                                </button>
                            )}
                        </span>
                    }
                />
            </div>

            {editingRate && (
                <div className="space-y-2 rounded-xl border border-blue-200 bg-white p-3">
                    <label
                        htmlFor="booking-rate-input"
                        className="text-[11px] font-medium text-brand-muted"
                    >
                        Giá mỗi đêm
                    </label>
                    <input
                        id="booking-rate-input"
                        aria-label="Giá mỗi đêm"
                        inputMode="numeric"
                        value={rateInput}
                        onChange={(event) => setRateInput(event.target.value.replace(/\D/g, ""))}
                        onKeyDown={(event) => {
                            if (event.key === "Escape") closeRateEditor();
                        }}
                        className="w-full rounded-lg border border-slate-200 px-3 py-2 text-sm"
                    />
                    <p className="text-[12px] text-slate-500">
                        {rateValid
                            ? `${booking.nights} đêm × ${fmtMoney(parsedRate)} = ${fmtMoney(
                                  parsedRate * booking.nights,
                              )}`
                            : "Giá mỗi đêm không hợp lệ"}
                    </p>
                    <div className="flex gap-2">
                        <Button
                            size="sm"
                            aria-label="Lưu giá"
                            disabled={!rateValid || savingRate}
                            onClick={submitRate}
                            className="flex-1 gap-1.5 cursor-pointer"
                        >
                            <Check className="w-3.5 h-3.5" /> Lưu
                        </Button>
                        <Button
                            size="sm"
                            variant="outline"
                            aria-label="Huỷ giá"
                            onClick={closeRateEditor}
                            className="flex-1 gap-1.5 cursor-pointer"
                        >
                            <X className="w-3.5 h-3.5" /> Huỷ
                        </Button>
                    </div>
                </div>
            )}

            <div className="flex items-center justify-between pt-2 border-t border-slate-200">
                <span className="text-sm font-medium text-brand-muted">Đã thanh toán</span>
                <span className={"text-sm font-bold " + paymentStatusClass}>
                    {fmtMoney(booking.paid_amount)} / {fmtMoney(booking.total_price)}
                </span>
            </div>

            <Button
                variant="outline"
                className="w-full mt-3 gap-2 text-sm font-semibold cursor-pointer"
                onClick={onInvoice}
                disabled={invoiceLoading}
            >
                <FileText className="w-4 h-4" />
                {invoiceLoading ? "Đang tạo..." : "📄 Invoice"}
            </Button>
        </Section>
    );
}
