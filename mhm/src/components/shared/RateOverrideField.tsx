import { useState } from "react";

import { fmtMoney } from "@/lib/format";
import type { MoneyVnd } from "@/lib/money";

interface RateOverrideFieldProps {
    /** Tổng tiền engine tính. `null` khi chưa tính xong hoặc tính lỗi. */
    engineTotal: MoneyVnd | null;
    nights: number;
    /** Giá mỗi đêm đang override. `null` = đang dùng giá engine. */
    value: number | null;
    onChange: (ratePerNight: number | null) => void;
}

/** Làm tròn XUỐNG bội số 1.000₫ — làm tròn lên sẽ báo giá cao hơn engine. */
function prefillRate(engineTotal: MoneyVnd | null, nights: number): number {
    if (!engineTotal || nights <= 0) return 0;
    return Math.floor(engineTotal / nights / 1000) * 1000;
}

/**
 * Ô giá bấm-để-sửa, dùng chung cho cả ba màn tạo booking.
 *
 * Đơn vị là GIÁ MỖI ĐÊM, khớp `set_booking_rate` và cột `rate_overridden_at` ở
 * backend — nhờ vậy gia hạn thêm đêm sau này vẫn áp đúng giá.
 */
export default function RateOverrideField({
    engineTotal,
    nights,
    value,
    onChange,
}: RateOverrideFieldProps) {
    const [editing, setEditing] = useState(false);

    const overrideTotal = value != null ? value * nights : null;
    // Kỳ có ngày lễ/cuối tuần thì các đêm giá khác nhau; đặt một mức cho cả kỳ
    // sẽ làm mất phần phụ thu. Nói thẳng ra thay vì để lễ tân tự phát hiện.
    const uneven =
        value != null && engineTotal != null && overrideTotal !== engineTotal;

    if (value == null && !editing) {
        return (
            <button
                type="button"
                data-testid="rate-display"
                onClick={() => {
                    setEditing(true);
                    onChange(prefillRate(engineTotal, nights));
                }}
                className="text-base font-bold text-emerald-600 tabular-nums underline decoration-dotted underline-offset-4 cursor-pointer"
            >
                {engineTotal == null ? "…" : fmtMoney(engineTotal)}
            </button>
        );
    }

    return (
        <div className="space-y-1 text-right">
            <div className="flex items-center justify-end gap-1">
                <input
                    data-testid="rate-input"
                    type="number"
                    inputMode="numeric"
                    min={0}
                    value={value ?? prefillRate(engineTotal, nights)}
                    onChange={(event) => {
                        const parsed = Number(event.target.value);
                        onChange(Number.isFinite(parsed) ? Math.trunc(parsed) : 0);
                    }}
                    className="w-32 rounded-lg border border-emerald-300 px-2 h-9 text-right text-sm tabular-nums"
                />
                <span className="text-xs text-brand-muted">₫/đêm</span>
            </div>

            <p data-testid="rate-override-total" className="text-sm font-bold text-emerald-600 tabular-nums">
                {nights} đêm × {fmtMoney(value ?? 0)} = {fmtMoney(overrideTotal ?? 0)}
            </p>

            {uneven && (
                <p data-testid="rate-uneven-warning" className="text-[11px] text-amber-600">
                    Kỳ này có đêm giá khác nhau (cuối tuần/lễ). Đặt giá tay sẽ áp một mức cho cả{" "}
                    {nights} đêm: {fmtMoney(overrideTotal ?? 0)} thay vì {fmtMoney(engineTotal ?? 0)}.
                </p>
            )}

            <button
                type="button"
                onClick={() => {
                    setEditing(false);
                    onChange(null);
                }}
                className="text-[11px] text-slate-500 underline cursor-pointer"
            >
                Về giá gốc
            </button>
        </div>
    );
}
