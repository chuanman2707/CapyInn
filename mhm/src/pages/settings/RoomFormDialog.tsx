import { X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { basePriceRole } from "@/lib/roomTypeRate";
import { useHotelStore } from "@/stores/useHotelStore";
import type { RoomTypeItem } from "@/types";

import type { RoomFormValues } from "./useRoomConfig";

interface RoomFormDialogProps {
  open: boolean;
  editingRoomId?: string;
  form: RoomFormValues;
  roomTypes: RoomTypeItem[];
  onChange: (next: RoomFormValues) => void;
  onClose: () => void;
  onSubmit: () => void;
}

export default function RoomFormDialog({
  open,
  editingRoomId,
  form,
  roomTypes,
  onChange,
  onClose,
  onSubmit,
}: RoomFormDialogProps) {
  const roomTypeRates = useHotelStore((state) => state.roomTypeRates);
  const role = basePriceRole(roomTypeRates, form.room_type);

  if (!open) return null;

  return (
    <div className="p-5 bg-slate-50 rounded-2xl space-y-4 border border-slate-200">
      <div className="flex items-center justify-between">
        <h4 className="font-bold text-sm">{editingRoomId ? `Sửa phòng: ${editingRoomId}` : "Thêm phòng mới"}</h4>
        <button onClick={onClose} className="text-brand-muted hover:text-brand-text cursor-pointer">
          <X size={18} />
        </button>
      </div>

      <div className="grid grid-cols-2 gap-3">
        <div>
          <Label>Mã phòng</Label>
          <Input
            value={form.id}
            onChange={(event) => onChange({ ...form, id: event.target.value })}
            placeholder="VD: 6A"
            className="mt-1.5"
            disabled={Boolean(editingRoomId)}
          />
        </div>
        <div>
          <Label>Tên phòng</Label>
          <Input
            value={form.name}
            onChange={(event) => onChange({ ...form, name: event.target.value })}
            placeholder="VD: Phòng 6A"
            className="mt-1.5"
          />
        </div>
        <div>
          <Label>Loại phòng</Label>
          <select
            value={form.room_type}
            onChange={(event) => onChange({ ...form, room_type: event.target.value })}
            className="mt-1.5 w-full bg-white border border-slate-200 rounded-xl px-3 py-2.5 text-sm font-medium outline-none"
          >
            <option value="">Chọn loại phòng...</option>
            {roomTypes.map((roomType) => (
              <option key={roomType.id} value={roomType.name}>
                {roomType.name}
              </option>
            ))}
          </select>
        </div>
        <div>
          <Label>Tầng</Label>
          <Input
            type="number"
            value={form.floor}
            onChange={(event) => onChange({ ...form, floor: Number(event.target.value) })}
            className="mt-1.5 w-24"
            min={1}
          />
        </div>
        <div className="flex items-end gap-3 pb-1">
          <label className="flex items-center gap-2 cursor-pointer">
            <input
              type="checkbox"
              checked={form.has_balcony}
              onChange={(event) => onChange({ ...form, has_balcony: event.target.checked })}
              className="w-4 h-4 accent-brand-primary"
            />
            <span className="text-sm font-medium">Ban công 🏞️</span>
          </label>
        </div>
        <div />
        <div>
          {/* Không gọi là "giá cơ bản": nhãn đó ngụ ý đây là giá khách trả, mà
              engine bỏ qua nó ngay khi loại phòng có bảng giá. Dòng chú thích
              dưới đây nói rõ số này đang có tác dụng gì. */}
          <Label>💰 Giá gốc của phòng (VNĐ)</Label>
          <Input
            type="number"
            value={form.base_price}
            onChange={(event) => onChange({ ...form, base_price: Number(event.target.value) })}
            className="mt-1.5"
          />
          <p className="mt-1.5 text-[11px] leading-snug text-brand-muted" data-testid="base-price-role">
            {role.kind === "ignored" && (
              <>
                Loại phòng này đã có bảng giá ({role.typeRateText}/đêm), nên số ở đây{" "}
                <strong>không</strong> ảnh hưởng tiền khách trả. Sửa giá ở mục Bảng giá.
              </>
            )}
            {role.kind === "derives-type-price" && (
              <>
                Loại phòng này chưa có bảng giá, nên giá gốc của phòng có mã nhỏ nhất
                trong loại sẽ thành giá của cả loại.
              </>
            )}
            {role.kind === "unknown" && <>Chưa đọc được bảng giá, chưa rõ số này có được dùng hay không.</>}
          </p>
        </div>
        <div>
          <Label>👥 Số khách tính giá base</Label>
          <Input
            type="number"
            value={form.max_guests}
            onChange={(event) => onChange({ ...form, max_guests: Number(event.target.value) })}
            className="mt-1.5 w-24"
            min={1}
          />
        </div>
        <div>
          <Label>➕ Phụ thu / người thêm (VNĐ)</Label>
          <Input
            type="number"
            value={form.extra_person_fee}
            onChange={(event) => onChange({ ...form, extra_person_fee: Number(event.target.value) })}
            className="mt-1.5"
          />
        </div>
      </div>

      <div className="flex gap-2">
        <Button onClick={onSubmit} className="bg-brand-primary text-white rounded-xl">
          {editingRoomId ? "Cập nhật" : "Tạo phòng"}
        </Button>
        <Button variant="outline" className="rounded-xl" onClick={onClose}>
          Hủy
        </Button>
      </div>
    </div>
  );
}
