import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
    CalendarDays,
    CalendarPlus,
    Check,
    CheckCircle2,
    Clipboard,
    FileText,
    LogOut,
    Play,
    Sparkles,
} from "lucide-react";
import { toast } from "sonner";

import InvoiceDialog from "@/components/InvoiceDialog";
import CheckoutSettlementModal from "@/components/CheckoutSettlementModal";
import InfoItem from "@/components/shared/InfoItem";
import ActionBtn from "@/components/shared/ActionBtn";
import RoomGuestsSection from "@/components/shared/RoomGuestsSection";
import Section from "@/components/shared/Section";
import StatusBadge from "@/components/shared/StatusBadge";
import SlideDrawer from "@/components/shared/SlideDrawer";
import { Button } from "@/components/ui/button";
import { useInvoiceDialog } from "@/hooks/useInvoiceDialog";
import { formatAppError } from "@/lib/appError";
import { getRoomTypeLabel } from "@/lib/constants";
import { fmtDateShort, fmtMoney } from "@/lib/format";
import { useHotelStore } from "@/stores/useHotelStore";
import type { CheckoutSettlementPayload, RoomWithBooking, HousekeepingTask } from "@/types";

interface RoomDrawerProps {
    open: boolean;
    onClose: () => void;
    roomId: string | null;
}

export default function RoomDrawer({ open, onClose, roomId }: RoomDrawerProps) {
    const {
        checkOut,
        extendStay,
        getStayInfoText,
        setCheckinOpen,
        fetchRooms,
        updateHousekeeping,
    } = useHotelStore();

    const [roomDetail, setRoomDetail] = useState<RoomWithBooking | null>(null);
    const [housekeepingTask, setHousekeepingTask] = useState<HousekeepingTask | null>(null);
    const [showCheckout, setShowCheckout] = useState(false);
    const [copied, setCopied] = useState(false);
    const [fetching, setFetching] = useState(false);
    const { invoiceOpen, invoiceData, invoiceLoading, openInvoice, closeInvoice } = useInvoiceDialog();

    useEffect(() => {
        if (!open || !roomId) {
            setRoomDetail(null);
            setHousekeepingTask(null);
            return;
        }

        setFetching(true);
        Promise.all([
            invoke<RoomWithBooking>("get_room_detail", { roomId }),
            invoke<HousekeepingTask[]>("get_housekeeping_tasks").then(
                (tasks) => tasks.find((t) => t.room_id === roomId && t.status !== "clean") ?? null
            ),
        ])
            .then(([detail, task]) => {
                setRoomDetail(detail);
                setHousekeepingTask(task);
            })
            .catch(console.error)
            .finally(() => setFetching(false));
    }, [open, roomId]);

    if (!open) return null;

    const handleClose = () => {
        setShowCheckout(false);
        onClose();
    };

    if (fetching || !roomDetail) {
        return (
            <SlideDrawer open onClose={handleClose}>
                <div className="flex-1 flex items-center justify-center">
                    <div className="text-sm text-slate-400">Đang tải...</div>
                </div>
            </SlideDrawer>
        );
    }

    const { room, booking, guests } = roomDetail;
    const roomTypeLabel = getRoomTypeLabel(room.type);

    const handleCopyStayInfo = async () => {
        if (!booking) return;
        const text = await getStayInfoText(booking.id);
        await navigator.clipboard.writeText(text);
        setCopied(true);
        setTimeout(() => setCopied(false), 2000);
    };

    const handleCheckoutConfirm = async ({
        settlementMode,
        finalTotal,
    }: CheckoutSettlementPayload) => {
        if (!booking) return;
        try {
            await checkOut(booking.id, settlementMode, finalTotal);
            setShowCheckout(false);
            toast.success("Check-out thành công!");
            handleClose();
        } catch (err) {
            toast.error("Lỗi check-out: " + formatAppError(err));
        }
    };

    const handleExtend = async () => {
        if (!booking) return;
        try {
            await extendStay(booking.id);
            const detail = await invoke<RoomWithBooking>("get_room_detail", { roomId: room.id });
            setRoomDetail(detail);
            toast.success("Đã gia hạn thêm 1 đêm!");
        } catch (err) {
            toast.error("Lỗi gia hạn: " + err);
        }
    };

    const handleInvoice = async () => {
        if (!booking) return;
        await openInvoice(booking.id);
    };

    const handleHousekeepingUpdate = async (newStatus: string) => {
        if (!housekeepingTask) return;
        try {
            await updateHousekeeping(housekeepingTask.id, newStatus);
            toast.success(newStatus === "cleaning" ? "Đang dọn phòng..." : "Dọn phòng hoàn tất! ✨");
            const [detail, tasks] = await Promise.all([
                invoke<RoomWithBooking>("get_room_detail", { roomId: room.id }),
                invoke<HousekeepingTask[]>("get_housekeeping_tasks"),
            ]);
            setRoomDetail(detail);
            setHousekeepingTask(tasks.find((t) => t.room_id === room.id && t.status !== "clean") ?? null);
            await fetchRooms();
        } catch (err) {
            toast.error("Lỗi cập nhật: " + err);
        }
    };

    const fmtTime = (iso: string) => {
        try {
            return new Date(iso).toLocaleString("vi-VN", {
                hour: "2-digit",
                minute: "2-digit",
                day: "2-digit",
                month: "2-digit",
            });
        } catch {
            return iso;
        }
    };

    // ── Content Sections ───────────────────────────────

    const guestSection = <RoomGuestsSection guests={guests} mode="sheet" />;

    const paymentStatusClass =
        booking && booking.paid_amount >= booking.total_price
            ? "text-emerald-600"
            : "text-orange-600";

    const bookingSection = booking ? (
        <Section icon={CalendarDays} title="Booking hiện tại" className="bg-slate-50 rounded-2xl p-5 space-y-3">
            <div className="grid grid-cols-2 gap-3">
                <InfoItem label="Check-in" value={fmtDateShort(booking.check_in_at)} />
                <InfoItem label="Checkout" value={fmtDateShort(booking.expected_checkout)} />
                <InfoItem label="Số đêm" value={booking.nights} />
                <InfoItem label="Tổng tiền" value={fmtMoney(booking.total_price)} />
            </div>
            <div className="flex items-center justify-between pt-2 border-t border-slate-200">
                <span className="text-sm font-medium text-brand-muted">Đã thanh toán</span>
                <span className={"text-sm font-bold " + paymentStatusClass}>
                    {fmtMoney(booking.paid_amount)} / {fmtMoney(booking.total_price)}
                </span>
            </div>
            <Button
                variant="outline"
                className="w-full mt-3 gap-2 text-sm font-semibold cursor-pointer"
                onClick={handleInvoice}
                disabled={invoiceLoading}
            >
                <FileText className="w-4 h-4" />
                {invoiceLoading ? "Đang tạo..." : "📄 Invoice"}
            </Button>
        </Section>
    ) : null;

    const getHkBadgeClass = () => {
        if (!housekeepingTask) return "";
        if (housekeepingTask.status === "needs_cleaning") return "text-amber-600 bg-amber-100";
        if (housekeepingTask.status === "cleaning") return "text-blue-600 bg-blue-100";
        return "text-emerald-600 bg-emerald-100";
    };

    const getHkLabel = () => {
        if (!housekeepingTask) return "";
        if (housekeepingTask.status === "needs_cleaning") return "Cần dọn";
        if (housekeepingTask.status === "cleaning") return "Đang dọn";
        return "Sạch";
    };

    const housekeepingSection =
        room.status === "cleaning" || housekeepingTask ? (
            <Section icon={Sparkles} title="Dọn phòng" className="bg-amber-50/50 rounded-2xl p-5 space-y-3">
                {housekeepingTask ? (
                    <>
                        <div className="flex items-center justify-between">
                            <span className="text-sm font-medium text-slate-600">Trạng thái</span>
                            <span className={"inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-[11px] font-semibold " + getHkBadgeClass()}>
                                {getHkLabel()}
                            </span>
                        </div>
                        <div className="flex items-center justify-between text-xs text-slate-500">
                            <span>Thời gian tạo</span>
                            <span>{fmtTime(housekeepingTask.triggered_at)}</span>
                        </div>
                        <div className="flex gap-2 pt-1">
                            {housekeepingTask.status === "needs_cleaning" && (
                                <Button
                                    className="flex-1 bg-blue-500 hover:bg-blue-600 text-white rounded-xl gap-1.5 cursor-pointer"
                                    onClick={() => handleHousekeepingUpdate("cleaning")}
                                >
                                    <Play size={14} /> Bắt đầu dọn
                                </Button>
                            )}
                            {housekeepingTask.status === "cleaning" && (
                                <Button
                                    className="flex-1 bg-emerald-500 hover:bg-emerald-600 text-white rounded-xl gap-1.5 cursor-pointer"
                                    onClick={() => handleHousekeepingUpdate("clean")}
                                >
                                    <CheckCircle2 size={14} /> Dọn xong
                                </Button>
                            )}
                        </div>
                    </>
                ) : (
                    <p className="text-sm text-amber-600">Phòng cần dọn</p>
                )}
            </Section>
        ) : null;

    const actionsSection = booking ? (
        <div className="space-y-2">
            <div className="grid grid-cols-2 gap-2">
                <ActionBtn
                    icon={copied ? Check : Clipboard}
                    label={copied ? "Đã copy!" : "Copy lưu trú"}
                    onClick={handleCopyStayInfo}
                    variant="ghost"
                />
                <ActionBtn icon={CalendarPlus} label="Extend +1 đêm" onClick={handleExtend} variant="blue" />
            </div>
            <button
                onClick={() => setShowCheckout(true)}
                className="w-full flex items-center justify-center gap-2 py-3 bg-red-600 hover:bg-red-700 text-white rounded-xl font-semibold text-[13px] transition-colors cursor-pointer"
            >
                <LogOut size={15} /> Check-out
            </button>
        </div>
    ) : null;

    const vacantSection =
        room.status === "vacant" ? (
            <button
                onClick={() => {
                    setCheckinOpen(true, room.id);
                    handleClose();
                }}
                className="w-full py-3 bg-emerald-600 hover:bg-emerald-700 text-white rounded-xl font-semibold text-[13px] transition-colors cursor-pointer"
            >
                Check-in phòng này
            </button>
        ) : null;

    const roomTitle = room.name || "Room " + room.id;

    return (
        <>
            <SlideDrawer open onClose={handleClose} title={roomTitle} subtitle={"Tầng " + room.floor + " • " + roomTypeLabel}>
                {/* Body */}
                <div className="flex-1 overflow-y-auto p-6 space-y-5">
                    {/* Status + Price row */}
                    <div className="flex items-center justify-between">
                        <StatusBadge status={room.status} variant="badge" />
                        <span className="text-lg font-bold text-brand-primary">{fmtMoney(room.base_price)}/đêm</span>
                    </div>

                    {/* Room info */}
                    <div className="grid grid-cols-2 gap-3">
                        <InfoItem label="Loại phòng" value={roomTypeLabel} />
                        <InfoItem label="Ban công" value={room.has_balcony ? "Có" : "Không"} />
                    </div>

                    {/* State-dependent sections */}
                    {vacantSection}
                    {bookingSection}
                    {guestSection}
                    {housekeepingSection}
                    {actionsSection}
                </div>
            </SlideDrawer>

            {booking && (
                <CheckoutSettlementModal
                    open={showCheckout}
                    roomId={room.id}
                    booking={booking}
                    onClose={() => setShowCheckout(false)}
                    onConfirm={handleCheckoutConfirm}
                />
            )}

            <InvoiceDialog
                open={invoiceOpen}
                onOpenChange={(nextOpen) => {
                    if (!nextOpen) closeInvoice();
                }}
                data={invoiceData}
            />
        </>
    );
}
