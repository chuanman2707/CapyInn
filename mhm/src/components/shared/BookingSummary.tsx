import { CalendarDays, FileText } from "lucide-react";

import InfoItem from "@/components/shared/InfoItem";
import Section from "@/components/shared/Section";
import { Button } from "@/components/ui/button";
import { isFullySettled } from "@/lib/bookingBalance";
import { fmtDateShort, fmtMoney } from "@/lib/format";
import type { Booking } from "@/types";

interface BookingSummaryProps {
    booking: Booking;
    onInvoice: () => void;
    invoiceLoading: boolean;
}

export default function BookingSummary({
    booking,
    onInvoice,
    invoiceLoading,
}: BookingSummaryProps) {
    const paymentStatusClass = isFullySettled(booking.total_price, booking.paid_amount)
        ? "text-emerald-600"
        : "text-orange-600";

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
            </div>
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
