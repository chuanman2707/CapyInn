import { Activity, ArrowRight, Clock3, DoorOpen, UserRound } from "lucide-react";

import InfoItem from "@/components/shared/InfoItem";
import Section from "@/components/shared/Section";
import SlideDrawer from "@/components/shared/SlideDrawer";
import { Button } from "@/components/ui/button";
import type { ActivityItem } from "@/types";

interface ActivityDetailDrawerProps {
    open: boolean;
    activity: ActivityItem | null;
    onClose: () => void;
    onOpenRoom?: (roomId: string) => void;
}

const KIND_LABELS: Record<NonNullable<ActivityItem["kind"]>, string> = {
    check_in: "Check-in",
    check_out: "Check-out",
    housekeeping: "Housekeeping",
};

function getKindLabel(kind?: ActivityItem["kind"]) {
    return kind ? KIND_LABELS[kind] : "Hoạt động";
}

function formatOccurredAt(value?: string) {
    if (!value) return "—";

    const parsed = new Date(value);
    if (Number.isNaN(parsed.getTime())) {
        return value;
    }

    return parsed.toLocaleString("vi-VN", {
        hour: "2-digit",
        minute: "2-digit",
        day: "2-digit",
        month: "2-digit",
        year: "numeric",
    });
}

export default function ActivityDetailDrawer({
    open,
    activity,
    onClose,
    onOpenRoom,
}: ActivityDetailDrawerProps) {
    if (!open || !activity) return null;

    return (
        <SlideDrawer
            open={open}
            onClose={onClose}
            width="w-[420px]"
            title="Chi tiết hoạt động"
            subtitle={getKindLabel(activity.kind)}
        >
            <div className="flex-1 overflow-y-auto p-6 space-y-4">
                <div className="rounded-2xl border border-slate-100 bg-slate-50 p-4">
                    <div className="flex items-start gap-3">
                        <div className={`w-10 h-10 rounded-xl flex items-center justify-center shrink-0 text-base ${activity.color || "bg-slate-100"}`}>
                            {activity.icon}
                        </div>
                        <div className="min-w-0">
                            <p className="text-base font-semibold text-brand-text leading-snug">{activity.text}</p>
                            <p className="text-sm text-brand-muted mt-1">{activity.status_label || getKindLabel(activity.kind)}</p>
                        </div>
                    </div>
                </div>

                <Section icon={Activity} title="Tổng quan">
                    <div className="grid grid-cols-2 gap-4">
                        <InfoItem label="Loại" value={getKindLabel(activity.kind)} />
                        <InfoItem label="Trạng thái" value={activity.status_label || "Cập nhật"} />
                        <InfoItem label="Giờ hiển thị" value={activity.time || "—"} />
                        <InfoItem label="Ghi nhận lúc" value={formatOccurredAt(activity.occurred_at)} />
                    </div>
                </Section>

                <Section icon={UserRound} title="Liên quan">
                    <div className="grid grid-cols-2 gap-4">
                        <InfoItem label="Khách" value={activity.guest_name || "—"} />
                        <InfoItem label="Phòng" value={activity.room_id || "—"} />
                    </div>
                </Section>

                {activity.room_id ? (
                    <Section icon={DoorOpen} title="Điều hướng">
                        <Button
                            type="button"
                            variant="outline"
                            className="w-full justify-between cursor-pointer"
                            onClick={() => onOpenRoom?.(activity.room_id!)}
                        >
                            Mở phòng {activity.room_id}
                            <ArrowRight />
                        </Button>
                    </Section>
                ) : null}

                <div className="flex items-center gap-2 text-xs text-brand-muted">
                    <Clock3 size={14} />
                    Dữ liệu lấy từ activity feed của dashboard overview.
                </div>
            </div>
        </SlideDrawer>
    );
}
