import { useCallback, useEffect, useRef, useState } from "react";
import type { KeyboardEvent, PointerEvent } from "react";
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
 * Dùng được bằng bàn phím: giữ Enter/Space kích hoạt như giữ chuột trái, nhả
 * phím hoặc rời focus (blur) thì huỷ. Nếu `disabled` chuyển thành true giữa
 * lúc đang giữ, lượt giữ bị huỷ ngay lập tức.
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

    const cancelHold = useCallback(() => {
        clearTimer();
        setHolding(false);
    }, [clearTimer]);

    // disabled có thể chuyển thành true giữa lúc đang giữ (Task 10 truyền
    // disabled={busy}: một lệnh khác đang chạy thì nút này phải khoá ngay).
    // Không huỷ ở đây thì timer vẫn đếm ngầm và callback vẫn bắn ra sau khi nút
    // đã hiện disabled — người dùng thấy nút bị khoá nhưng hành động vẫn xảy ra.
    // cancelHold() cũng đưa holding về false nên thanh tiến trình tắt theo,
    // chứ không chỉ mỗi việc callback bị chặn.
    useEffect(() => {
        if (disabled) {
            cancelHold();
        }
    }, [disabled, cancelHold]);

    const startHold = () => {
        if (disabled || timerRef.current !== null) return;
        setHolding(true);
        timerRef.current = setTimeout(() => {
            timerRef.current = null;
            setHolding(false);
            onHoldComplete();
        }, holdMs);
    };

    const handlePointerDown = (event: PointerEvent<HTMLButtonElement>) => {
        // Chỉ tính cú nhấn chuột trái (hoặc cú chạm chính) — nhấn giữ chuột
        // phải hay chuột giữa không phải một cú bấm cố ý, không được kích hoạt.
        if (event.button !== 0) return;
        startHold();
    };

    const handleKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
        if (event.key !== "Enter" && event.key !== " ") return;
        if (event.key === " ") {
            // Chặn hành vi mặc định (cuộn trang) khi giữ phím cách.
            event.preventDefault();
        }
        // Giữ phím khiến hệ điều hành tự phát lại sự kiện keydown liên tục
        // (event.repeat === true). startHold() vốn đã tự chặn gọi chồng nhờ
        // kiểm tra timerRef, nhưng chặn sớm ở đây cho rõ ý định và để không
        // phụ thuộc ngầm vào chốt đó.
        if (event.repeat) return;
        startHold();
    };

    const handleKeyUp = (event: KeyboardEvent<HTMLButtonElement>) => {
        if (event.key !== "Enter" && event.key !== " ") return;
        cancelHold();
    };

    return (
        <button
            type="button"
            disabled={disabled}
            onPointerDown={handlePointerDown}
            onPointerUp={cancelHold}
            onPointerLeave={cancelHold}
            onPointerCancel={cancelHold}
            onKeyDown={handleKeyDown}
            onKeyUp={handleKeyUp}
            onBlur={cancelHold}
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
            {/* Thanh tiến trình phía trên chỉ là hình ảnh (aria-hidden) —
                trình đọc màn hình không thấy gì báo đang trong lúc giữ.
                Vùng aria-live này nói thay, chỉ khi đang giữ. */}
            <span role="status" aria-live="polite" className="sr-only">
                {holding ? "Đang giữ…" : ""}
            </span>
        </button>
    );
}
