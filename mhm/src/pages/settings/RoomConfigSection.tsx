import { BedDouble, Pencil, Plus, Tag, Trash2, X } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { fmtMoney } from "@/lib/format";
import { basePriceIsUnused, nightlyRateDisplay } from "@/lib/roomTypeRate";
import { useHotelStore } from "@/stores/useHotelStore";

import RoomFormDialog from "./RoomFormDialog";
import useRoomConfig from "./useRoomConfig";

export default function RoomConfigSection() {
  const {
    rooms,
    roomTypes,
    newTypeName,
    setNewTypeName,
    showRoomForm,
    editingRoom,
    form,
    setForm,
    resetForm,
    handleAddType,
    handleDeleteType,
    openEdit,
    openAdd,
    handleSaveRoom,
    handleDeleteRoom,
  } = useRoomConfig();
  const roomTypeRates = useHotelStore((state) => state.roomTypeRates);

  return (
    <div className="space-y-8">
      <div className="space-y-4">
        <div>
          <h3 className="text-lg font-bold mb-1 flex items-center gap-2">
            <Tag size={18} className="text-brand-primary" />
            Loại phòng
          </h3>
          <p className="text-sm text-brand-muted">Tạo loại phòng trước, sau đó chọn khi thêm phòng</p>
        </div>
        <div className="flex flex-wrap gap-2">
          {roomTypes.map((roomType) => (
            <div key={roomType.id} className="flex items-center gap-1.5 px-3 py-1.5 bg-brand-primary/10 text-brand-primary rounded-full text-sm font-medium">
              {roomType.name}
              <button onClick={() => handleDeleteType(roomType.id)} className="hover:text-red-500 transition-colors cursor-pointer">
                <X size={14} />
              </button>
            </div>
          ))}
        </div>
        <div className="flex items-center gap-2">
          <Input
            value={newTypeName}
            onChange={(event) => setNewTypeName(event.target.value)}
            placeholder="Tên loại phòng mới..."
            className="w-64"
            onKeyDown={(event) => event.key === "Enter" && void handleAddType()}
          />
          <Button size="sm" className="bg-brand-primary text-white rounded-lg" onClick={() => void handleAddType()}>
            <Plus size={14} className="mr-1" /> Thêm loại
          </Button>
        </div>
      </div>

      <div className="space-y-4">
        <div className="flex items-center justify-between">
          <div>
            <h3 className="text-lg font-bold mb-1 flex items-center gap-2">
              <BedDouble size={18} className="text-brand-primary" />
              Danh sách phòng ({rooms.length})
            </h3>
            <p className="text-sm text-brand-muted">Quản lý phòng — thêm, sửa, xóa</p>
          </div>
          <Button className="bg-brand-primary text-white rounded-xl" onClick={openAdd}>
            <Plus size={14} className="mr-1" /> Thêm phòng
          </Button>
        </div>

        <RoomFormDialog
          open={showRoomForm}
          editingRoomId={editingRoom?.id}
          form={form}
          roomTypes={roomTypes}
          onChange={setForm}
          onClose={resetForm}
          onSubmit={() => void handleSaveRoom()}
        />

        <div className="space-y-2">
          {rooms.map((room) => {
            // Con số to nhất trên hàng phải là số khách trả, tức giá của loại
            // phòng. `room.base_price` từng đứng ở đây là dữ liệu engine bỏ qua
            // ngay khi loại phòng có bảng giá.
            const nightlyRate = nightlyRateDisplay(roomTypeRates, room.type);
            const basePriceUnused = basePriceIsUnused(roomTypeRates, room.type, room.base_price);

            return (
            <div key={room.id} className="flex items-center justify-between p-4 bg-slate-50 rounded-xl hover:bg-slate-100 transition-colors">
              <div className="flex items-center gap-4">
                <div className="w-10 h-10 rounded-lg bg-brand-primary/10 flex items-center justify-center font-bold text-brand-primary text-sm">
                  {room.id}
                </div>
                <div>
                  <p className="font-semibold text-sm">{room.name}</p>
                  <p className="text-xs text-brand-muted">
                    Tầng {room.floor} • {room.type} {room.has_balcony ? "• 🏞️" : ""} &nbsp;|&nbsp; 👥 {room.max_guests} người
                  </p>
                </div>
              </div>
              <div className="flex items-center gap-3">
                <div className="text-right">
                  <p className="text-sm font-bold text-brand-primary" data-testid="room-row-nightly-rate">
                    {nightlyRate.text}
                    {nightlyRate.unknown ? "" : "/đêm"}
                  </p>
                  {basePriceUnused && (
                    <p className="text-[10px] text-amber-600" data-testid="room-row-base-price-unused">
                      giá gốc {fmtMoney(room.base_price)} không được dùng
                    </p>
                  )}
                  {room.extra_person_fee > 0 && <p className="text-[10px] text-brand-muted">+{fmtMoney(room.extra_person_fee)}/người thêm</p>}
                </div>
                <button onClick={() => openEdit(room)} className="p-2 hover:bg-slate-200 rounded-lg transition-colors cursor-pointer">
                  <Pencil size={14} className="text-brand-muted" />
                </button>
                <button onClick={() => void handleDeleteRoom(room.id)} className="p-2 hover:bg-red-100 rounded-lg transition-colors cursor-pointer">
                  <Trash2 size={14} className="text-red-400" />
                </button>
              </div>
            </div>
            );
          })}

          {rooms.length === 0 && (
            <div className="text-center py-12 text-brand-muted">
              <BedDouble size={40} className="mx-auto mb-3 opacity-30" />
              <p className="text-sm">Chưa có phòng nào. Hãy tạo loại phòng trước, sau đó thêm phòng.</p>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
