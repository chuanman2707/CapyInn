import { CalendarMinus, CalendarPlus } from "lucide-react";

interface NightsStepperProps {
    canShorten: boolean;
    shortenDisabledReason: string;
    busy: boolean;
    onShorten: () => void;
    onExtend: () => void;
}

const baseClass =
    "flex flex-1 items-center justify-center gap-1.5 py-2.5 text-[12px] font-medium transition-colors cursor-pointer bg-blue-50 text-blue-600 hover:bg-blue-100 disabled:cursor-not-allowed disabled:opacity-50 disabled:hover:bg-blue-50";

export default function NightsStepper({
    canShorten,
    shortenDisabledReason,
    busy,
    onShorten,
    onExtend,
}: NightsStepperProps) {
    const shortenDisabled = busy || !canShorten;

    return (
        <div className="flex overflow-hidden rounded-xl border border-blue-200">
            <button
                type="button"
                onClick={onShorten}
                disabled={shortenDisabled}
                title={canShorten ? undefined : shortenDisabledReason}
                className={baseClass + " border-r border-blue-200"}
            >
                <CalendarMinus size={14} /> −1 đêm
            </button>
            <button
                type="button"
                onClick={onExtend}
                disabled={busy}
                className={baseClass}
            >
                <CalendarPlus size={14} /> +1 đêm
            </button>
        </div>
    );
}
