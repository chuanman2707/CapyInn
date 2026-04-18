import { useState } from "react";
import {
  ArrowLeft,
  Building2,
  CalendarDays,
  CalendarPlus,
  Check,
  Clipboard,
  CreditCard,
  FileText,
  LogOut,
} from "lucide-react";
import { toast } from "sonner";

import InvoiceDialog from "@/components/InvoiceDialog";
import InfoItem from "@/components/shared/InfoItem";
import ActionBtn from "@/components/shared/ActionBtn";
import PaymentBlock from "@/components/shared/PaymentBlock";
import RoomGuestsSection from "@/components/shared/RoomGuestsSection";
import Section from "@/components/shared/Section";
import StatusBadge from "@/components/shared/StatusBadge";
import SlideDrawer from "@/components/shared/SlideDrawer";
import { Button } from "@/components/ui/button";
import { useInvoiceDialog } from "@/hooks/useInvoiceDialog";
import { getRoomTypeLabel } from "@/lib/constants";
import { fmtDate, fmtDateShort, fmtMoney } from "@/lib/format";
import Modal from "@/components/ui/Modal";
import { useHotelStore } from "@/stores/useHotelStore";
import type { RoomWithBooking } from "@/types";

interface RoomDetailPanelProps {
  mode: "page" | "sheet";
  roomDetail?: RoomWithBooking | null;
  onBack?: () => void;
  onClose?: () => void;
}

export default function RoomDetailPanel({
  mode,
  roomDetail: roomDetailProp,
  onBack,
  onClose,
}: RoomDetailPanelProps) {
  const {
    checkOut,
    extendStay,
    getStayInfoText,
    setTab,
    setCheckinOpen,
    loading,
  } = useHotelStore();
  const [showCheckout, setShowCheckout] = useState(false);
  const [finalPaid, setFinalPaid] = useState(0);
  const [copied, setCopied] = useState(false);
  const { invoiceOpen, invoiceData, invoiceLoading, openInvoice, closeInvoice } = useInvoiceDialog();

  const resolvedRoomDetail = roomDetailProp ?? null;
  const isLoading = !roomDetailProp && loading;

  if (!resolvedRoomDetail || (mode === "page" && isLoading)) {
    return (
      <div className="flex items-center justify-center h-64 text-slate-400 text-sm">
        Đang tải...
      </div>
    );
  }

  const { room, booking, guests } = resolvedRoomDetail;

  const handleBack = () => {
    if (onBack) {
      onBack();
      return;
    }
    setTab("dashboard");
  };

  const handleClose = () => {
    onClose?.();
  };

  const handleCopyStayInfo = async () => {
    if (!booking) return;
    const text = await getStayInfoText(booking.id);
    await navigator.clipboard.writeText(text);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const handleCheckout = async () => {
    if (!booking) return;
    try {
      await checkOut(booking.id, finalPaid || undefined);
      setShowCheckout(false);
      toast.success("Check-out thành công!");
      if (mode === "sheet") {
        handleClose();
      }
    } catch (err) {
      toast.error("Lỗi check-out: " + err);
    }
  };

  const handleExtend = async () => {
    if (!booking) return;
    try {
      await extendStay(booking.id);
      toast.success("Đã gia hạn thêm 1 đêm!");
    } catch (err) {
      toast.error("Lỗi gia hạn: " + err);
    }
  };

  const handleInvoice = async () => {
    if (!booking) return;
    await openInvoice(booking.id);
  };

  const roomTypeLabel = getRoomTypeLabel(room.type);
  const outstandingAmount = booking ? booking.total_price - booking.paid_amount : 0;

  const guestSection = <RoomGuestsSection guests={guests} mode={mode} />;

  const pageContent = (
    <div className="space-y-5 animate-fade-up max-w-2xl">
      <button
        onClick={handleBack}
        className="flex items-center gap-1.5 text-[13px] text-slate-500 hover:text-blue-600 transition-colors cursor-pointer"
      >
        <ArrowLeft size={15} /> Quay lại
      </button>

      <div className="bg-white border border-slate-100 rounded-2xl p-5">
        <div className="flex items-center justify-between mb-5">
          <div className="flex items-center gap-3">
            <div className="w-10 h-10 rounded-xl bg-slate-50 flex items-center justify-center">
              <Building2 size={20} className="text-slate-400" />
            </div>
            <div>
              <h2 className="text-lg font-bold text-slate-900">{room.name}</h2>
              <p className="text-[12px] text-slate-400">
                {roomTypeLabel} · Tầng {room.floor} · {fmtMoney(room.base_price)}/đêm
              </p>
            </div>
          </div>
          <StatusBadge status={room.status} />
        </div>

        {room.status === "vacant" && (
          <button
            onClick={() => setCheckinOpen(true, room.id)}
            className="w-full py-3 bg-emerald-600 hover:bg-emerald-700 text-white rounded-xl font-semibold text-[13px] transition-colors cursor-pointer"
          >
            Check-in phòng này
          </button>
        )}

        {booking && (
          <div className="space-y-4 mt-1">
            {guestSection}

            <Section icon={CalendarDays} title="Thông tin lưu trú">
              <div className="grid grid-cols-2 gap-3 text-[12px]">
                <InfoItem label="Check-in" value={fmtDate(booking.check_in_at)} variant="block" />
                <InfoItem label="Check-out dự kiến" value={fmtDate(booking.expected_checkout)} variant="block" />
                <InfoItem label="Số đêm" value={String(booking.nights)} variant="block" />
                <InfoItem label="Nguồn" value={booking.source || "walk-in"} variant="block" />
              </div>
            </Section>

            <Section icon={CreditCard} title="Thanh toán">
              <div className="grid grid-cols-3 gap-3 text-center">
                <PaymentBlock label="Tổng tiền" value={fmtMoney(booking.total_price)} color="text-slate-900" />
                <PaymentBlock label="Đã trả" value={fmtMoney(booking.paid_amount)} color="text-emerald-600" />
                <PaymentBlock
                  label="Còn nợ"
                  value={fmtMoney(outstandingAmount)}
                  color={outstandingAmount > 0 ? "text-red-600" : "text-slate-400"}
                />
              </div>
            </Section>

            <div className="grid grid-cols-2 gap-2.5">
              <ActionBtn
                icon={copied ? Check : Clipboard}
                label={copied ? "Đã copy!" : "Copy lưu trú"}
                onClick={handleCopyStayInfo}
                variant="ghost"
              />
              <ActionBtn icon={CalendarPlus} label="Extend +1 đêm" onClick={handleExtend} variant="blue" />
              <Button
                variant="outline"
                className="rounded-xl gap-2 text-sm font-semibold cursor-pointer"
                onClick={handleInvoice}
                disabled={invoiceLoading}
              >
                <FileText className="w-4 h-4" />
                {invoiceLoading ? "Đang tạo..." : "📄 Invoice"}
              </Button>
              <button
                onClick={() => {
                  setFinalPaid(booking.total_price);
                  setShowCheckout(true);
                }}
                className="flex items-center justify-center gap-2 py-3 bg-red-600 hover:bg-red-700 text-white rounded-xl font-semibold text-[13px] transition-colors cursor-pointer"
              >
                <LogOut size={15} /> Check-out
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );

  const sheetContent = (
    <SlideDrawer open onClose={handleClose} title={room.name || `Room ${room.id}`} subtitle={`Tầng ${room.floor} • ${roomTypeLabel}`}>
      <div className="flex-1 overflow-y-auto p-6 space-y-6">
        <div className="flex items-center justify-between">
          <StatusBadge status={room.status} variant="badge" />
          <span className="text-lg font-bold text-brand-primary">{fmtMoney(room.base_price)}/đêm</span>
        </div>

        <div className="grid grid-cols-2 gap-3">
          <InfoItem label="Loại phòng" value={roomTypeLabel} />
          <InfoItem label="Ban công" value={room.has_balcony ? "Có" : "Không"} />
        </div>

        {booking ? (
          <Section icon={CalendarDays} title="Booking hiện tại" className="bg-slate-50 rounded-2xl p-5 space-y-3">
            <div className="grid grid-cols-2 gap-3">
              <InfoItem label="Check-in" value={fmtDateShort(booking.check_in_at)} />
              <InfoItem label="Checkout" value={fmtDateShort(booking.expected_checkout)} />
              <InfoItem label="Số đêm" value={booking.nights} />
              <InfoItem label="Tổng tiền" value={fmtMoney(booking.total_price)} />
            </div>
            <div className="flex items-center justify-between pt-2 border-t border-slate-200">
              <span className="text-sm font-medium text-brand-muted">Đã thanh toán</span>
              <span className={`text-sm font-bold ${booking.paid_amount >= booking.total_price ? "text-emerald-600" : "text-orange-600"}`}>
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
        ) : (
          <div className="bg-slate-50 rounded-2xl p-5 text-center text-brand-muted text-sm">
            Phòng hiện đang trống
          </div>
        )}

        {guestSection}
      </div>
    </SlideDrawer>
  );

  return (
    <>
      {mode === "page" ? pageContent : sheetContent}

      {showCheckout && booking && (
        <Modal title="Xác nhận Check-out">
          <div className="space-y-3 text-[13px]">
            <InfoItem label="Phòng" value={room.id} variant="block" />
            <InfoItem label="Tổng tiền" value={fmtMoney(booking.total_price)} variant="block" />
            <InfoItem label="Đã trả" value={fmtMoney(booking.paid_amount)} variant="block" />
            <div>
              <label className="text-[11px] text-slate-400 font-medium block mb-1">Thanh toán cuối</label>
              <input
                type="number"
                value={finalPaid}
                onChange={(event) => setFinalPaid(Number(event.target.value))}
                className="w-full bg-white border border-slate-200 rounded-lg px-3 py-2 text-slate-900 text-[13px] focus:outline-none focus:ring-2 focus:ring-blue-500/30 focus:border-blue-500"
              />
            </div>
          </div>
          <div className="flex gap-2.5 mt-5">
            <button
              onClick={() => setShowCheckout(false)}
              className="flex-1 py-2.5 bg-slate-100 hover:bg-slate-200 text-slate-700 rounded-xl text-[13px] font-medium cursor-pointer transition-colors"
            >
              Hủy
            </button>
            <button
              onClick={handleCheckout}
              className="flex-1 py-2.5 bg-red-600 hover:bg-red-700 text-white rounded-xl text-[13px] font-semibold cursor-pointer transition-colors"
            >
              Xác nhận
            </button>
          </div>
        </Modal>
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
