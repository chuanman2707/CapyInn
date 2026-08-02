import { useEffect, useState } from "react";
import { ArrowRightLeft } from "lucide-react";
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";
import { useHotelStore } from "@/stores/useHotelStore";
import { formatAppError } from "@/lib/appError";
import { getRoomTypeLabel } from "@/lib/constants";
import { fmtDateShort, fmtMoney, fmtNumber } from "@/lib/format";
import { toast } from "sonner";
import type { RoomChangeOptions } from "@/types";

/**
 * Sheet dùng chung cho mọi lối vào chuyển phòng (Task 9). Một booking có thể
 * đã ở vài đêm trước khi chuyển — những đêm đó KHÔNG được gán lại cho phòng
 * mới, chỉ những đêm từ hôm nay trở đi mới chuyển. Đây là điều dễ hiểu lầm
 * nhất của tính năng này nên phải nói thẳng ra, không chỉ ngụ ý qua con số
 * "còn X đêm".
 */
export function RoomChangeSheet() {
  const isOpen = useHotelStore((s) => s.isRoomChangeOpen);
  const bookingId = useHotelStore((s) => s.roomChangeBookingId);
  const setRoomChangeOpen = useHotelStore((s) => s.setRoomChangeOpen);
  const fetchRoomChangeOptions = useHotelStore((s) => s.fetchRoomChangeOptions);
  const changeRoom = useHotelStore((s) => s.changeRoom);

  const [options, setOptions] = useState<RoomChangeOptions | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [selectedRoomId, setSelectedRoomId] = useState<string | null>(null);
  const [keepPrice, setKeepPrice] = useState(false);
  const [reason, setReason] = useState("");
  const [saving, setSaving] = useState(false);

  useEffect(() => {
    if (!isOpen || !bookingId) return;
    let cancelled = false;
    setOptions(null);
    setLoadError(null);
    setSelectedRoomId(null);
    setKeepPrice(false);
    setReason("");
    fetchRoomChangeOptions(bookingId)
      .then((result) => {
        if (!cancelled) setOptions(result);
      })
      .catch((err) => {
        if (!cancelled) setLoadError(formatAppError(err));
      });
    return () => {
      cancelled = true;
    };
  }, [isOpen, bookingId, fetchRoomChangeOptions]);

  const selected = options?.rooms.find((r) => r.roomId === selectedRoomId) ?? null;
  const difference = selected ? (keepPrice ? 0 : selected.priceDifference) : 0;

  function closeAndReset(open: boolean) {
    setRoomChangeOpen(open, bookingId);
  }

  async function handleConfirm() {
    if (!options || !selected || saving) return;
    setSaving(true);
    try {
      await changeRoom(options.bookingId, selected.roomId, keepPrice, reason.trim() || undefined);
      toast.success(`Đã chuyển sang ${selected.name}`);
      closeAndReset(false);
    } catch (err) {
      // Phòng vừa được chọn có thể đã bị người khác lấy mất trong lúc sheet
      // đang mở — báo lỗi rồi nạp lại danh sách để lễ tân chọn tiếp, thay vì
      // để lại một lựa chọn đã chết trên màn hình.
      toast.error(formatAppError(err));
      setSelectedRoomId(null);
      if (bookingId) {
        try {
          const refreshed = await fetchRoomChangeOptions(bookingId);
          setOptions(refreshed);
        } catch (refreshErr) {
          setLoadError(formatAppError(refreshErr));
        }
      }
    } finally {
      setSaving(false);
    }
  }

  return (
    <Sheet open={isOpen} onOpenChange={closeAndReset}>
      <SheetContent side="right" className="w-[480px] sm:w-[520px] overflow-y-auto p-0">
        <SheetHeader className="p-6 pb-4 border-b border-slate-100">
          <SheetTitle className="flex items-center gap-2 text-lg">
            <ArrowRightLeft size={20} className="text-brand-primary" />
            Chuyển phòng
          </SheetTitle>
          {options && (
            <p className="text-sm text-slate-500">
              Đang ở <span className="font-semibold text-slate-700">{options.currentRoomName}</span>
            </p>
          )}
        </SheetHeader>

        <div className="p-6 space-y-5">
          {loadError && (
            <div className="rounded-xl p-3 text-sm bg-red-50 text-red-700 border border-red-200">
              {loadError}
            </div>
          )}

          {!options && !loadError && (
            <p className="text-sm text-slate-400 py-6 text-center">Đang tải...</p>
          )}

          {options && (
            <>
              <div className="rounded-xl border border-slate-200 bg-slate-50 p-3 space-y-1.5">
                <p className="text-sm text-slate-700">
                  Còn <span className="font-semibold">{options.nightsRemaining} đêm</span> sẽ chuyển:{" "}
                  {fmtDateShort(options.fromDate)} → {fmtDateShort(options.toDate)}
                </p>
                {options.nightsStayed > 0 && (
                  <p className="text-sm font-medium text-amber-700 bg-amber-50 border border-amber-200 rounded-lg px-2.5 py-1.5">
                    {options.nightsStayed} đêm trước vẫn tính {options.currentRoomName} — chỉ những
                    đêm còn lại mới chuyển sang phòng mới.
                  </p>
                )}
              </div>

              <div className="space-y-2">
                <h3 className="text-xs font-semibold text-slate-500 uppercase tracking-wider">
                  Chọn phòng mới
                </h3>

                {options.rooms.length === 0 ? (
                  <div className="rounded-xl p-3 text-sm bg-red-50 text-red-700 border border-red-200">
                    Không phòng nào trống suốt {options.nightsRemaining} đêm còn lại.
                  </div>
                ) : (
                  <ul className="space-y-2">
                    {options.rooms.map((room) => {
                      const isSelected = room.roomId === selectedRoomId;
                      return (
                        <li key={room.roomId}>
                          <button
                            type="button"
                            onClick={() => setSelectedRoomId(room.roomId)}
                            aria-pressed={isSelected}
                            className={`w-full text-left rounded-xl border p-3 transition-colors cursor-pointer ${
                              isSelected
                                ? "border-brand-primary bg-brand-primary/5 ring-2 ring-brand-primary/20"
                                : "border-slate-200 bg-white hover:border-slate-300"
                            }`}
                          >
                            <span className="font-semibold text-sm text-slate-800">{room.name}</span>
                            <p className="text-xs text-slate-500 mt-0.5">
                              {getRoomTypeLabel(room.roomType)} · Tầng {room.floor} ·{" "}
                              {fmtMoney(room.basePrice)}/đêm · tối đa {room.maxGuests} khách
                            </p>
                          </button>
                        </li>
                      );
                    })}
                  </ul>
                )}
              </div>

              {selected && (
                <div className="space-y-3 rounded-xl border border-slate-200 p-3">
                  <h3 className="text-xs font-semibold text-slate-500 uppercase tracking-wider">
                    Cách tính tiền
                  </h3>
                  <label className="flex items-center gap-2 text-sm text-slate-700 cursor-pointer">
                    <input
                      type="radio"
                      name="room-change-pricing"
                      checked={!keepPrice}
                      onChange={() => setKeepPrice(false)}
                    />
                    Tính lại theo phòng mới
                  </label>
                  <label className="flex items-center gap-2 text-sm text-slate-700 cursor-pointer">
                    <input
                      type="radio"
                      name="room-change-pricing"
                      aria-label="Giữ nguyên giá cũ"
                      checked={keepPrice}
                      onChange={() => setKeepPrice(true)}
                    />
                    Giữ nguyên giá cũ{" "}
                    <span className="text-xs text-slate-400">(lỗi của khách sạn hoặc nâng hạng miễn phí)</span>
                  </label>
                  <p className="text-sm font-semibold text-slate-800">
                    {difference === 0
                      ? "Không đổi tiền"
                      : `${difference > 0 ? "+" : "−"}${fmtNumber(Math.abs(difference))}đ so với phòng cũ`}
                  </p>
                </div>
              )}

              <div className="space-y-1.5">
                <label htmlFor="room-change-reason" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">
                  Lý do (không bắt buộc)
                </label>
                <input
                  id="room-change-reason"
                  className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-brand-primary/20"
                  value={reason}
                  onChange={(e) => setReason(e.target.value)}
                  placeholder="Vd: khách phàn nàn tiếng ồn, nâng hạng miễn phí..."
                />
              </div>

              <Button
                className="w-full h-12 rounded-xl bg-brand-primary hover:bg-brand-primary/90 text-white font-semibold text-sm cursor-pointer disabled:opacity-50 disabled:cursor-not-allowed"
                onClick={handleConfirm}
                disabled={!selected || saving}
              >
                {saving
                  ? "Đang xử lý..."
                  : selected
                    ? `Chuyển sang ${selected.name}${
                        difference !== 0
                          ? `, ${difference > 0 ? "cộng" : "trừ"} ${fmtNumber(Math.abs(difference))}đ`
                          : ", không đổi tiền"
                      }`
                    : "Chọn một phòng để chuyển"}
              </Button>
            </>
          )}
        </div>
      </SheetContent>
    </Sheet>
  );
}
