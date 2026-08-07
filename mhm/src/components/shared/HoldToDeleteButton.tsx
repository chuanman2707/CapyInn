import { useCallback, useEffect, useRef, useState } from "react";
import { Trash2 } from "lucide-react";

interface HoldToDeleteButtonProps {
    label: string;
    /** Thời gian phải giữ, tính bằng mili giây. Mặc định 2 giây. */
    holdMs?: number;
    disabled?: boolean;
    onHoldComplete: () => void;
}

/**
 * Nút đỏ phải nhấn giữ mới kích hoạt. Nhả tay sớm thì huỷ im lặng — người dùng
 * đổi ý không đáng bị hiện một thông báo lỗi.
 *
 * Component này không biết gì về booking: nó chỉ biết "giữ đủ lâu thì gọi
 * callback".
 */
export default function HoldToDeleteButton({
    label,
    holdMs = 2000,
    disabled = false,
    onHoldComplete,
}: HoldToDeleteButtonProps) {
    const [holding, setHolding] = useState(false);
    const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

    const clearTimer = useCallback(() => {
        if (timerRef.current !== null) {
            clearTimeout(timerRef.current);
            timerRef.current = null;
        }
    }, []);

    // Rời màn hình giữa lúc đang giữ thì timer vẫn chạy và sẽ gọi callback trên
    // một component đã unmount.
    useEffect(() => clearTimer, [clearTimer]);

    const startHold = () => {
        if (disabled || timerRef.current !== null) return;
        setHolding(true);
        timerRef.current = setTimeout(() => {
            timerRef.current = null;
            setHolding(false);
            onHoldComplete();
        }, holdMs);
    };

    const cancelHold = () => {
        clearTimer();
        setHolding(false);
    };

    return (
        <button
            type="button"
            disabled={disabled}
            onPointerDown={startHold}
            onPointerUp={cancelHold}
            onPointerLeave={cancelHold}
            onPointerCancel={cancelHold}
            className={`relative w-full overflow-hidden rounded-xl h-11 font-semibold text-white
                transition-colors select-none touch-none
                ${disabled ? "bg-slate-300 cursor-not-allowed" : "bg-red-600 hover:bg-red-700 cursor-pointer"}`}
        >
            <span
                aria-hidden
                data-testid="hold-progress"
                className="absolute inset-y-0 left-0 bg-red-900/40 transition-[width] ease-linear"
                style={{
                    width: holding ? "100%" : "0%",
                    transitionDuration: holding ? `${holdMs}ms` : "0ms",
                }}
            />
            <span className="relative flex items-center justify-center gap-1.5">
                <Trash2 size={16} /> {label}
            </span>
        </button>
    );
}
