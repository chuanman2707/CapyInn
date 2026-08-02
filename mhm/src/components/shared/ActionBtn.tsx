import type { ComponentType } from "react";

interface ActionBtnProps {
    icon: ComponentType<{ size?: number }>;
    label: string;
    onClick: () => void;
    variant: "ghost" | "blue";
    /** Lớp CSS phụ, ví dụ `col-span-2` cho nút đứng lẻ trong lưới 2 cột. */
    className?: string;
}

export default function ActionBtn({ icon: Icon, label, onClick, variant, className: extraClassName }: ActionBtnProps) {
    const className =
        variant === "blue"
            ? "bg-blue-50 text-blue-600 border border-blue-200 hover:bg-blue-100"
            : "bg-slate-100 text-slate-600 border border-slate-200 hover:bg-slate-200";

    return (
        <button
            onClick={onClick}
            className={`flex items-center justify-center gap-1.5 py-2.5 rounded-xl text-[12px] font-medium transition-colors cursor-pointer ${className} ${extraClassName ?? ""}`}
        >
            <Icon size={14} /> {label}
        </button>
    );
}
