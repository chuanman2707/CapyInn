import { useState, useRef, useEffect } from "react";
import { useAuthStore } from "@/stores/useAuthStore";
import { Delete, Lock } from "lucide-react";
import AppLogo from "@/components/AppLogo";
import { formatAppError } from "@/lib/appError";

export default function LoginScreen() {
    const { login, loading, error, clearError } = useAuthStore();
    const [pin, setPin] = useState("");
    const [shake, setShake] = useState(false);
    const containerRef = useRef<HTMLDivElement>(null);

    // Auto-submit when 4 digits entered
    useEffect(() => {
        if (pin.length === 4) {
            handleSubmit();
        }
    }, [pin]);

    const handleSubmit = async () => {
        if (pin.length !== 4) return;
        clearError();
        const success = await login(pin);
        if (!success) {
            setShake(true);
            setTimeout(() => {
                setShake(false);
                setPin("");
            }, 600);
        }
    };

    const handleDigit = (d: string) => {
        if (pin.length < 4) {
            setPin((prev) => prev + d);
        }
    };

    const handleBackspace = () => {
        setPin((prev) => prev.slice(0, -1));
        clearError();
    };

    const digits = ["1", "2", "3", "4", "5", "6", "7", "8", "9", "", "0", "⌫"];

    return (
        <div className="h-screen w-screen flex items-center justify-center bg-gradient-to-br from-slate-50 to-slate-100 select-none">
            <div
                ref={containerRef}
                className={`flex flex-col items-center gap-8 ${shake ? "animate-shake" : ""}`}
            >
                {/* Logo */}
                <div className="flex flex-col items-center gap-4">
                    <AppLogo className="h-20 w-20 drop-shadow-sm" />
                    <p className="text-sm text-brand-muted flex items-center gap-1.5">
                        <Lock size={14} />
                        Nhập mã PIN để đăng nhập
                    </p>
                </div>

                {/* PIN Dots */}
                <div className="flex gap-4">
                    {[0, 1, 2, 3].map((i) => (
                        <div
                            key={i}
                            className={`w-4 h-4 rounded-full transition-all duration-200 ${i < pin.length
                                ? "bg-brand-primary scale-110"
                                : "bg-slate-200"
                                }`}
                        />
                    ))}
                </div>

                {/* Error message */}
                {error && (
                    <p className="text-sm text-red-500 font-medium -mt-4 animate-fade-up">
                        {formatAppError(error)}
                    </p>
                )}

                {/* Number pad */}
                <div className="grid grid-cols-3 gap-3 w-[280px]">
                    {digits.map((d, i) => {
                        if (d === "") return <div key={i} />;
                        if (d === "⌫") {
                            return (
                                <button
                                    key={i}
                                    onClick={handleBackspace}
                                    disabled={loading}
                                    aria-label="Xóa"
                                    className="h-16 rounded-2xl bg-white border border-slate-100 text-brand-muted hover:bg-slate-50 active:scale-95 transition-all flex items-center justify-center cursor-pointer shadow-sm"
                                >
                                    <Delete size={22} />
                                </button>
                            );
                        }
                        return (
                            <button
                                key={i}
                                onClick={() => handleDigit(d)}
                                disabled={loading || pin.length >= 4}
                                className="h-16 rounded-2xl bg-white border border-slate-100 text-xl font-semibold text-brand-text hover:bg-slate-50 active:scale-95 transition-all cursor-pointer shadow-sm"
                            >
                                {d}
                            </button>
                        );
                    })}
                </div>

                {/* Loading indicator */}
                {loading && (
                    <div className="flex items-center gap-2 text-sm text-brand-muted">
                        <div className="w-4 h-4 border-2 border-brand-primary border-t-transparent rounded-full animate-spin" />
                        Đang xác thực...
                    </div>
                )}

                {/* Hint */}
                <p className="text-xs text-brand-muted/50 mt-4">
                    Liên hệ quản lý nếu quên mã PIN
                </p>
            </div>

            <style>{`
        @keyframes shake {
          0%, 100% { transform: translateX(0); }
          10%, 30%, 50%, 70%, 90% { transform: translateX(-8px); }
          20%, 40%, 60%, 80% { transform: translateX(8px); }
        }
        .animate-shake { animation: shake 0.5s ease-in-out; }
      `}</style>
        </div>
    );
}
