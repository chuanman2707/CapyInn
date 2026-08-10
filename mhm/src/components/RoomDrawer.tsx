import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowRightLeft, Check, Clipboard, LogOut } from "lucide-react";
import { toast } from "sonner";

import InvoiceDialog from "@/components/InvoiceDialog";
import CheckoutSettlementModal from "@/components/CheckoutSettlementModal";
import VoidBookingDialog from "@/components/VoidBookingDialog";
import BookingSummary from "@/components/shared/BookingSummary";
import InfoItem from "@/components/shared/InfoItem";
import ActionBtn from "@/components/shared/ActionBtn";
import NightsStepper from "@/components/shared/NightsStepper";
import RoomGuestsSection from "@/components/shared/RoomGuestsSection";
import StatusBadge from "@/components/shared/StatusBadge";
import SlideDrawer from "@/components/shared/SlideDrawer";
import { useInvoiceDialog } from "@/hooks/useInvoiceDialog";
import { formatAppError } from "@/lib/appError";
import { getRoomTypeLabel } from "@/lib/constants";
import { fmtMoney } from "@/lib/format";
import { nightlyRateDisplay } from "@/lib/roomTypeRate";
import { useAuthStore } from "@/stores/useAuthStore";
import { useHotelStore } from "@/stores/useHotelStore";
import type { CheckoutSettlementPayload, RoomWithBooking } from "@/types";

interface RoomDrawerProps {
    open: boolean;
    onClose: () => void;
    roomId: string | null;
}

export default function RoomDrawer({ open, onClose, roomId }: RoomDrawerProps) {
    const {
        checkOut,
        extendStay,
        shortenStay,
        getStayInfoText,
        setCheckinOpen,
        setRoomChangeOpen,
        fetchRooms,
        roomTypeRates,
        setBookingRate,
        updateBookingNotes,
    } = useHotelStore();

    const [roomDetail, setRoomDetail] = useState<RoomWithBooking | null>(null);
    const [showCheckout, setShowCheckout] = useState(false);
    const [copied, setCopied] = useState(false);
    const [fetching, setFetching] = useState(false);
    const [nightsBusy, setNightsBusy] = useState(false);
    const [voidOpen, setVoidOpen] = useState(false);
    const { invoiceOpen, invoiceData, invoiceLoading, openInvoice, closeInvoice } = useInvoiceDialog();
    // Xóa (khác trả phòng bình thường): lượt check-in này lẽ ra không nên tồn
    // tại — bấm nhầm phòng, ghi trùng khách. Chỉ chủ khách sạn được làm, và
    // backend (void_booking_tx) tự chặn lại độc lập với UI này; trạng thái nút
    // ở đây chỉ là gợi ý cho người dùng, không phải lớp bảo vệ.
    const isAdmin = useAuthStore((state) => state.isAdmin());

    useEffect(() => {
        if (!open || !roomId) {
            setRoomDetail(null);
            return;
        }

        setFetching(true);
        invoke<RoomWithBooking>("get_room_detail", { roomId })
            .then(setRoomDetail)
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

    const { room, booking, guests, group_id } = roomDetail;
    const roomTypeLabel = getRoomTypeLabel(room.type);
    // Giá loại phòng từ engine, không phải `room.base_price`.
    const nightlyRate = nightlyRateDisplay(roomTypeRates, room.type);

    // So sánh phải cắt về đầu ngày ở cả hai vế. Backend chặn khi
    // `checkout − 1 ngày < hôm nay`, tức tương đương `checkout ≤ hôm nay`;
    // nên nút chỉ mở khi checkout thực sự sau hôm nay. So thẳng hai đối
    // tượng Date chưa cắt giờ sẽ sai: checkout hôm nay lúc 12:00 vẫn lớn
    // hơn 00:00 hôm nay, nút sẽ mở trong khi backend từ chối.
    const checkoutDay = booking ? new Date(booking.expected_checkout) : null;
    checkoutDay?.setHours(0, 0, 0, 0);
    const todayStart = new Date();
    todayStart.setHours(0, 0, 0, 0);

    // Đảo ngược quyết định trước đó: rút đêm từng được phép đưa tổng tiền
    // xuống dưới số khách đã trả. Backend (shorten_stay_tx) giờ từ chối thẳng
    // ca đó — khoá luôn nút ở đây thay vì để người dùng bấm rồi nhận lỗi.
    // Công thức tiền hoàn 1 đêm phải khớp CHÍNH XÁC công thức backend dùng
    // (chia nguyên, làm tròn xuống): `current_total / current_nights`.
    const nightCredit = booking ? Math.floor(booking.total_price / booking.nights) : 0;
    const totalAfterShorten = booking ? booking.total_price - nightCredit : 0;
    const wouldUnderpayAfterShorten = Boolean(
        booking && totalAfterShorten < booking.paid_amount,
    );

    const canShorten = Boolean(
        booking &&
            booking.nights > 1 &&
            checkoutDay !== null &&
            checkoutDay.getTime() > todayStart.getTime() &&
            !wouldUnderpayAfterShorten,
    );
    // Thứ tự ưu tiên khi nhiều điều kiện cùng chặn: cấu trúc (số đêm tối
    // thiểu, đêm cuối đã tới) đi trước lý do tiền — hai cái đầu là bất khả
    // thi tuyệt đối bất kể tiền nong, nên tooltip nên nói lý do "gốc" hơn.
    const shortenDisabledReason =
        booking && booking.nights <= 1
            ? "Lưu trú tối thiểu 1 đêm"
            : checkoutDay !== null && checkoutDay.getTime() <= todayStart.getTime()
                ? "Đêm cuối là hôm nay — dùng Check-out"
                : `Khách đã thanh toán ${fmtMoney(booking?.paid_amount ?? 0)}, cao hơn tổng tiền sau khi rút đêm (${fmtMoney(totalAfterShorten)}) — cần xử lý hoàn tiền trước`;

    const refreshRoomDetail = async () => {
        const detail = await invoke<RoomWithBooking>("get_room_detail", { roomId: room.id });
        setRoomDetail(detail);
    };

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
        if (!booking || nightsBusy) return;
        setNightsBusy(true);
        try {
            await extendStay(booking.id);
            await refreshRoomDetail();
            toast.success("Đã gia hạn thêm 1 đêm!");
        } catch (err) {
            toast.error("Lỗi gia hạn: " + formatAppError(err));
        } finally {
            setNightsBusy(false);
        }
    };

    const handleShorten = async () => {
        if (!booking || nightsBusy) return;
        setNightsBusy(true);
        try {
            await shortenStay(booking.id);
            // Gọi invoke trực tiếp thay vì dùng lại refreshRoomDetail(): hàm đó
            // set state rồi trả về void, còn toast bên dưới cần chính con số
            // vừa refetch (đêm còn lại, tổng tiền mới). State setter không đọc
            // lại được trong cùng tick, nên nếu tái dùng refreshRoomDetail(),
            // toast sẽ hiện số liệu cũ trước khi rút đêm — sai mà không lỗi rõ.
            const detail = await invoke<RoomWithBooking>("get_room_detail", {
                roomId: room.id,
            });
            setRoomDetail(detail);
            const nights = detail.booking?.nights ?? 0;
            const total = detail.booking?.total_price ?? 0;
            toast.success(`Đã rút 1 đêm — còn ${nights} đêm, ${fmtMoney(total)}`);
        } catch (err) {
            toast.error("Lỗi rút đêm: " + formatAppError(err));
        } finally {
            setNightsBusy(false);
        }
    };

    const handleInvoice = async () => {
        if (!booking) return;
        await openInvoice(booking.id);
    };

    const handleSaveRate = async (ratePerNight: number) => {
        if (!booking) return;
        try {
            await setBookingRate(booking.id, ratePerNight);
            await refreshRoomDetail();
            toast.success("Đã cập nhật giá phòng!");
        } catch (err) {
            toast.error("Lỗi sửa giá: " + formatAppError(err));
            throw err;
        }
    };

    const handleSaveNotes = async (notes: string) => {
        if (!booking) return;
        try {
            await updateBookingNotes(booking.id, notes);
            await refreshRoomDetail();
            toast.success("Đã lưu ghi chú!");
        } catch (err) {
            toast.error("Lỗi lưu ghi chú: " + formatAppError(err));
            throw err;
        }
    };

    // ── Content Sections ───────────────────────────────

    const guestSection = <RoomGuestsSection guests={guests} mode="sheet" />;

    const bookingSection = booking ? (
        <BookingSummary
            booking={booking}
            onInvoice={handleInvoice}
            invoiceLoading={invoiceLoading}
            onSaveRate={handleSaveRate}
            onSaveNotes={handleSaveNotes}
        />
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
                {/* Nút "Extend +1 đêm" cũ đã thành cặp −1/+1 trong
                    NightsStepper; nó vẫn chiếm đúng một ô lưới như trước. */}
                <NightsStepper
                    canShorten={canShorten}
                    shortenDisabledReason={shortenDisabledReason}
                    busy={nightsBusy}
                    onShorten={handleShorten}
                    onExtend={handleExtend}
                />
                {/* 3 nút trong lưới 2 cột — nút cuối trải hết chiều ngang thay
                    vì bị bỏ mồ côi một cột khi số nút lẻ. */}
                <ActionBtn
                    icon={ArrowRightLeft}
                    label="Chuyển phòng"
                    onClick={() => {
                        if (!booking) return;
                        setRoomChangeOpen(true, booking.id);
                        // Bàn giao hẳn cho RoomChangeSheet rồi đóng, giống hệt
                        // nút check-in bên dưới. `roomDetail` là state cục bộ
                        // và effect nạp nó chỉ phụ thuộc [open, roomId] — cả
                        // hai đều không đổi khi khách chuyển phòng, còn
                        // listener "db-updated" toàn cục chỉ làm mới
                        // rooms/stats của store. Để drawer mở là nó vẫn khoe
                        // phòng cũ, tổng tiền trước khi cộng chênh, và một nút
                        // Check-out mở modal ghi "Phòng 101" cho khách đã sang
                        // 202 — sai ngay lúc xác nhận tiền.
                        handleClose();
                    }}
                    variant="ghost"
                    className="col-span-2"
                />
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

    // Chỉ hiện với một lượt ĐANG Ở thật sự — phòng trống (booking null) hay đã
    // trả (status khác active) thì không có gì để xóa qua đường này. Gate
    // hiển thị này là thứ BookingDetailPopup (Task 11) không có (nó luôn hiện
    // nút cho cả reservation lẫn đã trả) — giữ nguyên, không rút gọn theo
    // Task 11 khi mirror logic khoá bên dưới.
    //
    // Khoá cho lượt thuộc đoàn: mirror đúng BookingDetailPopup (Task 11).
    // Trước bản sửa này `get_room_detail` không trả `group_id` nên nút luôn
    // hiện bật cho tới khi bấm mới biết bị chặn ở preview — hai lối vào của
    // cùng một hành động phá hoại cư xử khác nhau. Giờ `RoomWithBooking.group_id`
    // (thêm ở mhm/src-tauri/src/queries/booking/room_queries.rs — KHÔNG thêm
    // vào `Booking` dùng chung nơi khác) mang dữ liệu đó lên tới đây. An toàn
    // vẫn không đổi — vẫn chỉ là gợi ý sớm: VoidBookingDialog tự phát hiện
    // `is_group_booking` qua preview và void_booking_tx từ chối đoàn vô điều
    // kiện ở backend, độc lập với mọi trạng thái UI.
    const isGroupBooking = Boolean(group_id);
    const voidDisabled = !isAdmin || isGroupBooking;
    const voidHint = isGroupBooking
        ? "Lượt này thuộc đoàn — chưa hỗ trợ xóa"
        : !isAdmin
          ? "Chỉ chủ khách sạn xóa được"
          : null;

    const voidSection =
        booking && booking.status === "active" ? (
            <div className="pt-2 border-t border-slate-100 space-y-1">
                <button
                    type="button"
                    disabled={voidDisabled}
                    onClick={() => setVoidOpen(true)}
                    className={`w-full text-sm rounded-lg h-9 border transition-colors ${
                        voidDisabled
                            ? "border-slate-200 text-slate-300 cursor-not-allowed"
                            : "border-red-200 text-red-600 hover:bg-red-50 cursor-pointer"
                    }`}
                >
                    Xóa lượt này
                </button>
                {voidHint && <p className="text-[11px] text-slate-400 text-center">{voidHint}</p>}
            </div>
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
                        <span
                            className="text-lg font-bold text-brand-primary"
                            data-testid="room-drawer-nightly-rate"
                            title={nightlyRate.derived ? "Suy ra từ giá phòng, chưa có bảng giá cho loại này" : undefined}
                        >
                            {nightlyRate.text}
                            {nightlyRate.unknown ? "" : "/đêm"}
                        </span>
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
                    {actionsSection}
                    {voidSection}
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

            {voidOpen && booking && (
                <VoidBookingDialog
                    bookingId={booking.id}
                    onClose={() => setVoidOpen(false)}
                    onVoided={async () => {
                        setVoidOpen(false);
                        // Ngăn kéo đóng thẳng sau khi xóa (khác popup chi tiết
                        // booking): nó đứng trên lưới phòng, và phòng vừa xóa
                        // đã về trạng thái trống — làm mới `rooms` để lưới
                        // không còn treo khách đã bị xóa lượt. `voidBooking`
                        // (gọi bên trong VoidBookingDialog) tự làm mới rồi,
                        // gọi lại ở đây cho chắc — hơn là ngầm dựa vào
                        // side-effect của một action store khác.
                        await fetchRooms();
                        handleClose();
                    }}
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
