import { useEffect, type ReactNode } from "react";
import { X } from "lucide-react";

interface SlideDrawerProps {
    open: boolean;
    onClose: () => void;
    title?: string;
    subtitle?: string;
    width?: string;
    header?: ReactNode;
    children: ReactNode;
}

export default function SlideDrawer({
    open,
    onClose,
    title,
    subtitle,
    width = "w-[420px]",
    header,
    children,
}: SlideDrawerProps) {
    // Esc key support
    useEffect(() => {
        if (!open) return;
        const handleKey = (e: KeyboardEvent) => {
            if (e.key === "Escape") onClose();
        };
        document.addEventListener("keydown", handleKey);
        return () => document.removeEventListener("keydown", handleKey);
    }, [open, onClose]);

    if (!open) return null;

    return (
        <div className="fixed inset-0 z-50 flex justify-end">
            <div className="absolute inset-0 bg-black/10 backdrop-blur-xs" onClick={onClose} />

            <div className={`relative ${width} h-full bg-white shadow-xl animate-slide-in-right flex flex-col`}>
                {/* Header */}
                {header ?? (
                    <div className="flex items-center justify-between p-6 border-b border-slate-100">
                        <div>
                            {title && <h2 className="text-xl font-bold">{title}</h2>}
                            {subtitle && <p className="text-sm text-brand-muted">{subtitle}</p>}
                        </div>
                        <button
                            onClick={onClose}
                            className="w-8 h-8 rounded-lg bg-slate-100 flex items-center justify-center hover:bg-slate-200 transition-colors cursor-pointer"
                        >
                            <X size={16} />
                        </button>
                    </div>
                )}

                {/* Body */}
                <div className="flex-1 overflow-y-auto p-6">
                    {children}
                </div>
            </div>
        </div>
    );
}
