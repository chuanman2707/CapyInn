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
    // M-a (rà cuối trước merge): điều kiện này CHỈ nói được "tổng của bạn
    // khác tổng engine" — tức MỌI lần giảm giá cố ý cũng bật cờ này y hệt
    // một kỳ có đêm giá khác nhau thật. Component chỉ có `engineTotal` (một
    // con số tổng), không có giá từng đêm, nên KHÔNG được suy ra nguyên nhân
    // "cuối tuần/lễ" — câu chữ ở dưới phải trung tính, chỉ nêu hai con số.
    const uneven =
        value != null && engineTotal != null && overrideTotal !== engineTotal;

    if (value == null && !editing) {
        // M-1 (review Task 17): chưa có engineTotal (đang tải, hoặc preview
        // vừa lỗi) thì không có gì để prefill — trước đây bấm vào lúc này
        // gọi prefillRate(null, nights) = 0 và âm thầm gửi giá 0₫/đêm lên
        // backend (backend từ chối bằng một toast khó hiểu thay vì một cảnh
        // báo rõ ràng ngay tại chỗ). Khoá nút lại thay vì để nó trông bấm được.
        const disabled = engineTotal == null;
        return (
            <button
                type="button"
                data-testid="rate-display"
                aria-label="Giá phòng — bấm để sửa giá tay"
                disabled={disabled}
                onClick={() => {
                    if (disabled) return;
                    setEditing(true);
                    onChange(prefillRate(engineTotal, nights));
                }}
                className={`text-base font-bold text-emerald-600 tabular-nums underline decoration-dotted underline-offset-4 ${disabled ? "cursor-not-allowed opacity-60" : "cursor-pointer"
                    }`}
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
                    aria-label="Giá tay mỗi đêm, đơn vị đồng"
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
                    Giá tay cho {nights} đêm là {fmtMoney(overrideTotal ?? 0)}, khác giá engine{" "}
                    {fmtMoney(engineTotal ?? 0)}.
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
