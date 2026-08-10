import { useState, useEffect } from "react";
import { Calendar, CreditCard, Globe } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { invoke } from "@tauri-apps/api/core";
import SlideDrawer from "@/components/shared/SlideDrawer";
import StatCard from "@/components/shared/StatCard";
import EmptyState from "@/components/shared/EmptyState";
import { fmtDateShort, fmtMoney } from "@/lib/format";
import type { BookingStatus } from "@/types";

interface BookingWithRoom {
    booking_id: string;
    room_id: string;
    check_in_at: string;
    expected_checkout: string;
    total_price: number;
    status: BookingStatus;
}

interface GuestHistoryResponse {
    guest: { id: string; full_name: string; doc_number: string; nationality: string | null; date_of_birth: string | null; gender: string | null };
    bookings: BookingWithRoom[];
}

// Nhãn tiếng Việt cho từng status lượt lưu trú. Bản trước chỉ có hai giá trị
// nhị phân, tiếng Anh, suy từ `status === "active"`: mọi status khác (kể cả
// "booked" chưa hề diễn ra, hay "cancelled" đã hủy) đều bị gộp vào
// "Completed" — vừa sai ngôn ngữ vừa sai sự thật. Backend đã lọc "voided"
// khỏi lịch sử khách (8 đường đọc SQL), nhưng hàm này vẫn nhận diện nó tường
// minh thay vì rơi vào default — cùng lý do phòng thủ theo chiều sâu như
// Reservations.tsx.
/** Không bao giờ được gọi nếu switch bên dưới xét đủ mọi nhánh của BookingStatus. */
function assertUnreachableStatus(status: never): never {
    throw new Error(`Thiếu nhãn tiếng Việt cho status: ${String(status)}`);
}

export function guestBookingStatusLabel(status: BookingStatus): string {
    switch (status) {
        case "active":
            return "Đang ở";
        case "checked_out":
            return "Đã trả phòng";
        case "booked":
            return "Đặt trước";
        case "cancelled":
            return "Đã hủy";
        case "no_show":
            return "Không đến";
        case "voided":
            return "Đã xóa";
        default:
            return assertUnreachableStatus(status);
    }
}

export default function GuestProfileSheet({ guestId, onClose }: { guestId: string; onClose: () => void }) {
    const [data, setData] = useState<GuestHistoryResponse | null>(null);

    useEffect(() => {
        invoke<GuestHistoryResponse>("get_guest_history", { guestId })
            .then(setData)
            .catch(console.error);
    }, [guestId]);

    if (!data) {
        return (
            <SlideDrawer open onClose={onClose} width="w-[440px]">
                <div className="flex-1 flex items-center justify-center">
                    <span className="text-brand-muted">Đang tải...</span>
                </div>
            </SlideDrawer>
        );
    }

    const totalSpent = data.bookings.reduce((sum, b) => sum + b.total_price, 0);
    const totalNights = data.bookings.reduce((sum, b) => {
        const ci = new Date(b.check_in_at);
        const co = new Date(b.expected_checkout);
        return sum + Math.max(1, Math.ceil((co.getTime() - ci.getTime()) / (1000 * 60 * 60 * 24)));
    }, 0);

    return (
        <SlideDrawer open onClose={onClose} width="w-[440px]" header={
            <div className="p-6 border-b border-slate-100">
                <div className="flex items-start justify-between">
                    <div>
                        <h2 className="text-xl font-bold">{data.guest.full_name}</h2>
                        <p className="text-sm text-brand-muted font-mono mt-1">{data.guest.doc_number}</p>
                    </div>
                    <button onClick={onClose} className="w-8 h-8 rounded-lg bg-slate-100 flex items-center justify-center hover:bg-slate-200 transition-colors cursor-pointer">
                        <span className="text-sm">✕</span>
                    </button>
                </div>

                {/* Info Tags */}
                <div className="flex items-center gap-2 mt-4">
                    {data.guest.nationality && (
                        <Badge className="bg-blue-50 text-blue-700 border-blue-200 rounded-lg text-xs px-2 py-0.5">
                            <Globe size={12} className="mr-1" />{data.guest.nationality}
                        </Badge>
                    )}
                    {data.guest.gender && (
                        <Badge className="bg-slate-50 text-slate-600 border-slate-200 rounded-lg text-xs px-2 py-0.5">{data.guest.gender}</Badge>
                    )}
                    {data.bookings.length >= 5 && (
                        <Badge className="bg-amber-50 text-amber-700 border-amber-200 rounded-lg text-xs px-2 py-0.5">⭐ VIP</Badge>
                    )}
                </div>
            </div>
        }>
            {/* Metrics */}
            <div className="grid grid-cols-3 gap-4 p-6 border-b border-slate-100">
                <StatCard icon={Calendar} label="Lần lưu trú" value={String(data.bookings.length)} color="blue" layout="centered" />
                <StatCard icon={CreditCard} label="Tổng chi tiêu" value={fmtMoney(totalSpent)} color="emerald" layout="centered" />
                <StatCard icon={Calendar} label="Tổng đêm" value={String(totalNights)} color="amber" layout="centered" />
            </div>

            {/* Stay History Timeline */}
            <div className="flex-1 overflow-y-auto p-6">
                <h3 className="font-bold text-sm mb-4">Lịch sử lưu trú</h3>
                {data.bookings.length > 0 ? (
                    <div className="space-y-0">
                        {data.bookings.map((b, i) => {
                            const isActive = b.status === "active";
                            return (
                                <div key={b.booking_id} className="flex gap-3">
                                    {/* Timeline line */}
                                    <div className="flex flex-col items-center">
                                        <div className={`w-3 h-3 rounded-full shrink-0 ${isActive ? "bg-brand-primary ring-4 ring-brand-primary/20" : "bg-slate-300"}`} />
                                        {i < data.bookings.length - 1 && <div className="w-0.5 h-full bg-slate-200 my-1" />}
                                    </div>

                                    {/* Content */}
                                    <div className="pb-5 flex-1">
                                        <div className="flex items-center justify-between">
                                            <span className="font-semibold text-sm">Room {b.room_id}</span>
                                            <Badge className={`text-[10px] px-2 py-0 rounded-md border-0 ${isActive ? "bg-blue-50 text-blue-700" : "bg-slate-100 text-slate-500"}`}>
                                                {guestBookingStatusLabel(b.status)}
                                            </Badge>
                                        </div>
                                        <p className="text-xs text-brand-muted mt-1">
                                            {fmtDateShort(b.check_in_at)} → {fmtDateShort(b.expected_checkout)}
                                        </p>
                                        <p className="text-xs font-semibold mt-1">{fmtMoney(b.total_price)}</p>
                                    </div>
                                </div>
                            );
                        })}
                    </div>
                ) : (
                    <EmptyState message="Chưa có lịch sử lưu trú" />
                )}
            </div>
        </SlideDrawer>
    );
}

