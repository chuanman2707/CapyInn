import { BedDouble, CalendarClock, LogIn, Sparkles, User, Eye } from "lucide-react";
import { ROOM_STATUS_CARD_BG, ROOM_STATUS_TEXT, STATUS_DOT_COLORS, STATUS_LABELS, getRoomTypeLabel } from "@/lib/constants";
import { fmtMoney, fmtDateShort } from "@/lib/format";
import type { Room, BookingWithGuest } from "@/types";

interface UnifiedRoomCardProps {
    room: Room;
    booking?: BookingWithGuest | null;
    nextReservationDate?: string | null;
    onOpenDrawer: (roomId: string) => void;
    onQuickAction?: (roomId: string) => void;
    compact?: boolean;
}

const QUICK_ACTION_CONFIG: Record<string, { label: string; icon: typeof LogIn; className: string }> = {
    vacant: { label: "Check-in", icon: LogIn, className: "bg-emerald-500 hover:bg-emerald-600 text-white" },
    cleaning: { label: "Dọn phòng", icon: Sparkles, className: "bg-amber-500 hover:bg-amber-600 text-white" },
    occupied: { label: "Chi tiết", icon: Eye, className: "bg-blue-500 hover:bg-blue-600 text-white" },
    booked: { label: "Chi tiết", icon: Eye, className: "bg-purple-500 hover:bg-purple-600 text-white" },
};

export default function UnifiedRoomCard({
    room,
    booking,
    nextReservationDate,
    onOpenDrawer,
    onQuickAction,
    compact,
}: UnifiedRoomCardProps) {
    const qa = QUICK_ACTION_CONFIG[room.status] ?? QUICK_ACTION_CONFIG.vacant;
    const QaIcon = qa.icon;

    const summaryLine = (() => {
        switch (room.status) {
            case "occupied":
                if (booking) {
                    const pct = booking.total_price > 0
                        ? Math.round((booking.paid_amount / booking.total_price) * 100)
                        : 0;
                    return (
                        <div className="flex items-center gap-1.5 text-[11px] text-blue-600 font-medium truncate">
                            <User size={10} className="shrink-0" />
                            <span className="truncate">{booking.guest_name}</span>
                            <span className="text-blue-400">· {pct}%</span>
                        </div>
                    );
                }
                return null;

            case "cleaning":
                return (
                    <div className="flex items-center gap-1.5 text-[11px] text-amber-600 font-medium">
                        <Sparkles size={10} className="shrink-0 animate-pulse" />
                        <span>Cần dọn</span>
                    </div>
                );

            case "booked":
                if (booking) {
                    return (
                        <div className="flex items-center gap-1.5 text-[11px] text-purple-600 font-medium truncate">
                            <CalendarClock size={10} className="shrink-0" />
                            <span className="truncate">{booking.guest_name}</span>
                            <span className="text-purple-400">· {fmtDateShort(booking.scheduled_checkin || booking.check_in_at)}</span>
                        </div>
                    );
                }
                return null;

            case "vacant":
            default:
                return null;
        }
    })();

    return (
        <div
            className={`
        relative rounded-xl border border-transparent cursor-pointer text-left
        transition-all duration-200 ease-out group
        hover:shadow-soft active:scale-[0.98]
        ${ROOM_STATUS_CARD_BG[room.status]}
        ${compact ? "p-3" : "p-4"}
      `}
            onClick={() => onOpenDrawer(room.id)}
        >
            {/* Header: Room ID + Status Dot */}
            <div className="flex items-center justify-between mb-2">
                <span className={`${compact ? "text-sm" : "text-base"} font-bold text-text-primary`}>
                    {room.id}
                </span>
                <span className={`w-2 h-2 rounded-full ${STATUS_DOT_COLORS[room.status]} status-dot`} />
            </div>

            {/* Room type + Price */}
            <div className="flex items-center gap-1.5 mb-1.5">
                <BedDouble size={12} className="text-text-muted shrink-0" />
                <span className="text-[11px] text-text-secondary font-medium">
                    {getRoomTypeLabel(room.type)}
                </span>
            </div>

            <div className={`${compact ? "text-[12px]" : "text-[13px]"} font-semibold text-text-primary mb-1.5`}>
                {fmtMoney(room.base_price)}
            </div>

            {/* Status label */}
            <div className={`text-[10px] font-semibold uppercase tracking-wider mb-2 ${ROOM_STATUS_TEXT[room.status]}`}>
                {STATUS_LABELS[room.status]}
            </div>

            {/* Context line (state-dependent) */}
            {summaryLine}

            {/* Upcoming reservation badge (only for vacant) */}
            {nextReservationDate && room.status === "vacant" && (
                <div className="mt-1.5 flex items-center gap-1 text-[10px] text-blue-600 bg-blue-50 px-2 py-0.5 rounded-lg w-fit">
                    <CalendarClock size={10} />
                    <span className="font-semibold">Đặt {nextReservationDate}</span>
                </div>
            )}

            {/* Quick Action Button */}
            <button
                className={`
          mt-2.5 w-full flex items-center justify-center gap-1.5 py-1.5 rounded-lg
          text-[11px] font-semibold transition-all cursor-pointer
          opacity-0 group-hover:opacity-100
          ${qa.className}
        `}
                onClick={(e) => {
                    e.stopPropagation();
                    if (onQuickAction) {
                        onQuickAction(room.id);
                    } else {
                        onOpenDrawer(room.id);
                    }
                }}
            >
                <QaIcon size={12} />
                {qa.label}
            </button>
        </div>
    );
}
