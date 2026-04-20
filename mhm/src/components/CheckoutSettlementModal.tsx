import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import InfoItem from "@/components/shared/InfoItem";
import Modal from "@/components/ui/Modal";
import { fmtMoney } from "@/lib/format";
import type {
    Booking,
    CheckoutSettlementMode,
    CheckoutSettlementPayload,
    CheckoutSettlementPreview,
} from "@/types";

interface CheckoutSettlementModalProps {
    open: boolean;
    roomId: string;
    booking: Booking;
    onClose: () => void;
    onConfirm: (payload: CheckoutSettlementPayload) => Promise<void> | void;
}

const MODE_OPTIONS: Array<{ value: CheckoutSettlementMode; label: string }> = [
    { value: "actual_nights", label: "Thực tế" },
    { value: "hourly", label: "Theo giờ" },
    { value: "booked_nights", label: "Đã đặt" },
];

export default function CheckoutSettlementModal({
    open,
    roomId,
    booking,
    onClose,
    onConfirm,
}: CheckoutSettlementModalProps) {
    const [settlementMode, setSettlementMode] = useState<CheckoutSettlementMode>("actual_nights");
    const [preview, setPreview] = useState<CheckoutSettlementPreview | null>(null);
    const [finalTotal, setFinalTotal] = useState(0);
    const [loadingPreview, setLoadingPreview] = useState(false);
    const [submitting, setSubmitting] = useState(false);
    const manualOverrideRef = useRef(false);

    useEffect(() => {
        if (!open) {
            setSettlementMode("actual_nights");
            setPreview(null);
            setFinalTotal(0);
            setLoadingPreview(false);
            setSubmitting(false);
            manualOverrideRef.current = false;
        }
    }, [open]);

    useEffect(() => {
        if (!open) {
            return;
        }

        let cancelled = false;
        manualOverrideRef.current = false;
        setLoadingPreview(true);

        invoke<CheckoutSettlementPreview>("preview_checkout_settlement", {
            req: {
                booking_id: booking.id,
                settlement_mode: settlementMode,
            },
        })
            .then((nextPreview) => {
                if (cancelled) {
                    return;
                }
                setPreview(nextPreview);
                if (!manualOverrideRef.current) {
                    setFinalTotal(nextPreview.recommended_total);
                }
            })
            .catch(() => {
                if (!cancelled) {
                    setPreview(null);
                }
            })
            .finally(() => {
                if (!cancelled) {
                    setLoadingPreview(false);
                }
            });

        return () => {
            cancelled = true;
        };
    }, [open, booking.id, settlementMode]);

    if (!open) {
        return null;
    }

    const overpaid = booking.paid_amount > finalTotal;
    const confirmDisabled =
        loadingPreview || submitting || preview === null || finalTotal < 0 || overpaid;

    const handleConfirm = async () => {
        if (confirmDisabled) {
            return;
        }

        setSubmitting(true);
        try {
            await onConfirm({ settlementMode, finalTotal });
        } finally {
            setSubmitting(false);
        }
    };

    return (
        <Modal title="Xác nhận Check-out">
            <div className="space-y-3 text-[13px]">
                <InfoItem label="Phòng" value={roomId} variant="block" />
                <InfoItem label="Đã trả" value={fmtMoney(booking.paid_amount)} variant="block" />
                <InfoItem label="Tổng đã đặt" value={fmtMoney(booking.total_price)} variant="block" />

                <div className="space-y-2">
                    <span className="text-[11px] text-slate-400 font-medium block">Cách tính</span>
                    <div className="grid grid-cols-3 gap-2">
                        {MODE_OPTIONS.map((option) => {
                            const active = settlementMode === option.value;
                            return (
                                <button
                                    key={option.value}
                                    type="button"
                                    onClick={() => setSettlementMode(option.value)}
                                    className={
                                        active
                                            ? "rounded-xl border border-blue-500 bg-blue-50 px-3 py-2 font-semibold text-blue-600 cursor-pointer"
                                            : "rounded-xl border border-slate-200 px-3 py-2 text-slate-600 cursor-pointer"
                                    }
                                >
                                    {option.label}
                                </button>
                            );
                        })}
                    </div>
                </div>

                <p className="rounded-xl bg-slate-50 px-3 py-2 text-slate-600 min-h-11">
                    {loadingPreview ? "Đang tính lại..." : preview?.explanation ?? ""}
                </p>

                <div>
                    <label
                        htmlFor="checkout-final-total"
                        className="text-[11px] text-slate-400 font-medium block mb-1"
                    >
                        Thanh toán cuối
                    </label>
                    <input
                        id="checkout-final-total"
                        type="number"
                        value={finalTotal}
                        onChange={(event) => {
                            manualOverrideRef.current = true;
                            setFinalTotal(Number(event.target.value));
                        }}
                        className="w-full bg-white border border-slate-200 rounded-lg px-3 py-2 text-slate-900 text-[13px] focus:outline-none focus:ring-2 focus:ring-blue-500/30 focus:border-blue-500"
                    />
                </div>

                {overpaid && (
                    <p className="text-[12px] font-medium text-red-600">
                        Booking đã overpaid. Hãy xử lý refund trước khi checkout.
                    </p>
                )}
            </div>

            <div className="flex gap-2.5 mt-5">
                <button
                    type="button"
                    onClick={onClose}
                    className="flex-1 py-2.5 bg-slate-100 hover:bg-slate-200 text-slate-700 rounded-xl text-[13px] font-medium cursor-pointer transition-colors"
                >
                    Hủy
                </button>
                <button
                    type="button"
                    onClick={handleConfirm}
                    disabled={confirmDisabled}
                    className="flex-1 py-2.5 bg-red-600 hover:bg-red-700 disabled:bg-slate-300 text-white rounded-xl text-[13px] font-semibold cursor-pointer transition-colors disabled:cursor-not-allowed"
                >
                    Xác nhận
                </button>
            </div>
        </Modal>
    );
}
