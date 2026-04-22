import { useState, useEffect } from "react";
import { useHotelStore } from "../stores/useHotelStore";
import {
    Sheet,
    SheetContent,
    SheetHeader,
    SheetTitle,
} from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { FormField, FormFieldSelect } from "@/components/shared/FormField";
import { formatAppError } from "@/lib/appError";
import { fmtMoney } from "@/lib/format";
import { toast } from "sonner";
import { Sparkles, Hand, Check, ChevronLeft, ChevronRight, Users, Star, CalendarClock } from "lucide-react";
import type { CheckInGuestInput, RoomAssignment } from "@/types";

const STEPS = ["Thông tin đoàn", "Chọn phòng", "Thông tin khách", "Xác nhận"];

export default function GroupCheckinSheet() {
    const {
        isGroupCheckinOpen,
        setGroupCheckinOpen,
        rooms,
        groupCheckIn,
        autoAssignRooms,
        loading,
    } = useHotelStore();

    const [step, setStep] = useState(0);

    // Step 1: Group info
    const [groupName, setGroupName] = useState("");
    const [organizerName, setOrganizerName] = useState("");
    const [organizerPhone, setOrganizerPhone] = useState("");
    const [roomCount, setRoomCount] = useState(3);
    const [roomType, setRoomType] = useState("all");
    const [nights, setNights] = useState(1);
    const [source, setSource] = useState("walk-in");
    const [checkInDate, setCheckInDate] = useState(() => {
        const d = new Date();
        return d.toISOString().split("T")[0];
    });

    const todayStr = new Date().toISOString().split("T")[0];
    const isReservation = checkInDate > todayStr;

    // Step 2: Room selection
    const [selectedRooms, setSelectedRooms] = useState<string[]>([]);
    const [masterRoomId, setMasterRoomId] = useState<string>("");
    const [assignMode, setAssignMode] = useState<"auto" | "manual">("auto");

    // Step 3: Guest info
    const [guestsPerRoom, setGuestsPerRoom] = useState<Record<string, CheckInGuestInput[]>>({});

    // Step 4: Payment
    const [paidAmount, setPaidAmount] = useState(0);
    const [notes, setNotes] = useState("");

    const vacantRooms = rooms.filter((r) => r.status === "vacant");

    useEffect(() => {
        if (!isGroupCheckinOpen) {
            setStep(0);
            setGroupName("");
            setOrganizerName("");
            setOrganizerPhone("");
            setRoomCount(3);
            setRoomType("all");
            setNights(1);
            setSource("walk-in");
            setCheckInDate(new Date().toISOString().split("T")[0]);
            setSelectedRooms([]);
            setMasterRoomId("");
            setAssignMode("auto");
            setGuestsPerRoom({});
            setPaidAmount(0);
            setNotes("");
        }
    }, [isGroupCheckinOpen]);

    const handleAutoAssign = async () => {
        try {
            const result = await autoAssignRooms(
                roomCount,
                roomType === "all" ? undefined : roomType
            );
            const ids = result.assignments.map((a: RoomAssignment) => a.room.id);
            setSelectedRooms(ids);
            if (ids.length > 0) setMasterRoomId(ids[0]);
            toast.success(`Đã chọn tự động ${ids.length} phòng`);
        } catch (err) {
            toast.error(formatAppError(err));
        }
    };

    const toggleRoom = (id: string) => {
        setSelectedRooms((prev) =>
            prev.includes(id) ? prev.filter((r) => r !== id) : [...prev, id]
        );
    };

    const totalPrice = selectedRooms.reduce((sum, id) => {
        const room = rooms.find((r) => r.id === id);
        return sum + (room ? room.base_price * nights : 0);
    }, 0);

    const handleSubmit = async () => {
        try {
            await groupCheckIn({
                group_name: groupName,
                organizer_name: organizerName,
                organizer_phone: organizerPhone || undefined,
                check_in_date: isReservation ? checkInDate : undefined,
                room_ids: selectedRooms,
                master_room_id: masterRoomId,
                guests_per_room: guestsPerRoom,
                nights,
                source,
                notes: notes || undefined,
                paid_amount: paidAmount || undefined,
            });
            if (isReservation) {
                toast.success(`📅 Đã đặt phòng đoàn "${groupName}" cho ngày ${checkInDate} — ${selectedRooms.length} phòng`);
            } else {
                toast.success(`✅ Group check-in "${groupName}" — ${selectedRooms.length} phòng`);
            }
            setGroupCheckinOpen(false);
        } catch (err) {
            toast.error(formatAppError(err));
        }
    };

    const updateGuestField = (roomId: string, guestIdx: number, field: keyof CheckInGuestInput, val: string) => {
        setGuestsPerRoom((prev) => {
            const roomGuests = [...(prev[roomId] || [])];
            if (!roomGuests[guestIdx]) {
                roomGuests[guestIdx] = { full_name: "", doc_number: "" };
            }
            // eslint-disable-next-line @typescript-eslint/no-explicit-any
            (roomGuests[guestIdx] as any)[field] = val;
            return { ...prev, [roomId]: roomGuests };
        });
    };

    const canNext = () => {
        switch (step) {
            case 0: return groupName.trim() && organizerName.trim() && roomCount > 0 && nights > 0;
            case 1: return selectedRooms.length > 0 && masterRoomId;
            case 2: return true; // Guest info optional
            case 3: return true;
            default: return false;
        }
    };

    return (
        <Sheet open={isGroupCheckinOpen} onOpenChange={setGroupCheckinOpen}>
            <SheetContent
                side="right"
                className="w-[640px] sm:max-w-[640px] overflow-y-auto p-0"
            >
                <SheetHeader className="px-6 pt-6 pb-4 border-b border-slate-100">
                    <SheetTitle className="text-xl font-bold">
                        <Users className="inline mr-2 text-brand-primary" size={22} />
                        {isReservation ? "Group Reservation" : "Group Check-in"}
                        {isReservation && (
                            <span className="ml-2 text-xs bg-amber-100 text-amber-700 px-2 py-0.5 rounded-full font-bold uppercase align-middle">
                                <CalendarClock size={12} className="inline mr-1" />Đặt trước
                            </span>
                        )}
                    </SheetTitle>
                    {/* Step indicator */}
                    <div className="flex gap-2 mt-3">
                        {STEPS.map((s, i) => (
                            <div
                                key={s}
                                className={`flex-1 h-1.5 rounded-full transition-colors ${i <= step ? "bg-brand-primary" : "bg-slate-100"
                                    }`}
                            />
                        ))}
                    </div>
                    <p className="text-sm text-brand-muted mt-1">
                        Bước {step + 1}/{STEPS.length}: {STEPS[step]}
                    </p>
                </SheetHeader>

                <div className="px-6 py-5 space-y-5">
                    {/* Step 1: Group Info */}
                    {step === 0 && (
                        <>
                            <FormField label="Tên đoàn *" value={groupName} onChange={setGroupName} />
                            <FormField label="Trưởng đoàn *" value={organizerName} onChange={setOrganizerName} />
                            <FormField label="SĐT trưởng đoàn" value={organizerPhone} onChange={setOrganizerPhone} />
                            <div className="grid grid-cols-2 gap-4">
                                <div>
                                    <Label className="text-xs font-semibold text-brand-muted uppercase mb-1.5 block">Số phòng cần</Label>
                                    <Input type="number" value={roomCount} onChange={(e) => setRoomCount(+e.target.value)} min={1} max={30} />
                                </div>
                                <FormFieldSelect
                                    label="Loại phòng"
                                    value={roomType}
                                    onChange={setRoomType}
                                    options={["all", "standard", "deluxe"]}
                                />
                            </div>
                            <div className="grid grid-cols-2 gap-4">
                                <div>
                                    <Label className="text-xs font-semibold text-brand-muted uppercase mb-1.5 block">Ngày nhận phòng *</Label>
                                    <Input
                                        type="date"
                                        value={checkInDate}
                                        onChange={(e) => setCheckInDate(e.target.value)}
                                        min={todayStr}
                                    />
                                </div>
                                <div>
                                    <Label className="text-xs font-semibold text-brand-muted uppercase mb-1.5 block">Số đêm *</Label>
                                    <Input type="number" value={nights} onChange={(e) => setNights(+e.target.value)} min={1} />
                                </div>
                            </div>
                        </>
                    )}

                    {/* Step 2: Room Selection */}
                    {step === 1 && (
                        <>
                            <div className="flex gap-3 mb-4">
                                <Button
                                    variant={assignMode === "auto" ? "default" : "outline"}
                                    onClick={() => setAssignMode("auto")}
                                    className="flex-1 rounded-xl"
                                >
                                    <Sparkles size={16} className="mr-2" /> Auto-assign
                                </Button>
                                <Button
                                    variant={assignMode === "manual" ? "default" : "outline"}
                                    onClick={() => setAssignMode("manual")}
                                    className="flex-1 rounded-xl"
                                >
                                    <Hand size={16} className="mr-2" /> Chọn tay
                                </Button>
                            </div>

                            {assignMode === "auto" && (
                                <div className="space-y-3">
                                    <Button onClick={handleAutoAssign} className="w-full rounded-xl bg-brand-primary text-white">
                                        <Sparkles size={16} className="mr-2" /> Tự động chọn {roomCount} phòng
                                    </Button>
                                    {selectedRooms.length > 0 && (
                                        <p className="text-sm text-emerald-600 font-medium">
                                            ✅ Đã chọn: {selectedRooms.join(", ")}
                                        </p>
                                    )}
                                </div>
                            )}

                            {assignMode === "manual" && (
                                <div className="grid grid-cols-3 gap-2 max-h-[300px] overflow-y-auto">
                                    {vacantRooms.map((room) => (
                                        <button
                                            key={room.id}
                                            onClick={() => toggleRoom(room.id)}
                                            className={`p-3 rounded-xl border-2 text-left transition-all cursor-pointer ${selectedRooms.includes(room.id)
                                                ? "border-brand-primary bg-brand-primary/5"
                                                : "border-slate-100 hover:border-slate-200"
                                                }`}
                                        >
                                            <p className="font-bold text-sm">{room.name}</p>
                                            <p className="text-xs text-brand-muted">{room.type} • T{room.floor}</p>
                                            <p className="text-xs font-semibold text-brand-primary mt-1">{fmtMoney(room.base_price)}</p>
                                            {selectedRooms.includes(room.id) && (
                                                <Check size={14} className="text-brand-primary mt-1" />
                                            )}
                                        </button>
                                    ))}
                                </div>
                            )}

                            {selectedRooms.length > 0 && (
                                <div className="mt-4 p-4 bg-slate-50 rounded-xl">
                                    <Label className="text-xs font-semibold text-brand-muted uppercase mb-2 block">
                                        Phòng đại diện (Master Room)
                                    </Label>
                                    <div className="flex flex-wrap gap-2">
                                        {selectedRooms.map((id) => {
                                            const room = rooms.find((r) => r.id === id);
                                            return (
                                                <button
                                                    key={id}
                                                    onClick={() => setMasterRoomId(id)}
                                                    className={`px-3 py-1.5 rounded-lg text-sm font-medium transition-all cursor-pointer ${masterRoomId === id
                                                        ? "bg-brand-primary text-white"
                                                        : "bg-white border border-slate-200 text-brand-text hover:border-brand-primary"
                                                        }`}
                                                >
                                                    {masterRoomId === id && <Star size={12} className="inline mr-1" />}
                                                    {room?.name || id}
                                                </button>
                                            );
                                        })}
                                    </div>
                                </div>
                            )}
                        </>
                    )}

                    {/* Step 3: Guest Info */}
                    {step === 2 && (
                        <div className="space-y-4">
                            <p className="text-sm text-brand-muted">
                                Nhập thông tin khách cho mỗi phòng. Phòng đại diện ({masterRoomId}) bắt buộc có khách chính.
                            </p>
                            {selectedRooms.map((roomId) => {
                                const room = rooms.find((r) => r.id === roomId);
                                const isMaster = roomId === masterRoomId;
                                const guest = guestsPerRoom[roomId]?.[0] || { full_name: "", doc_number: "" };
                                return (
                                    <div
                                        key={roomId}
                                        className={`p-4 rounded-xl border ${isMaster ? "border-brand-primary bg-brand-primary/5" : "border-slate-100"
                                            }`}
                                    >
                                        <div className="flex items-center gap-2 mb-3">
                                            <span className="font-bold text-sm">{room?.name || roomId}</span>
                                            {isMaster && (
                                                <span className="text-[10px] bg-brand-primary text-white px-2 py-0.5 rounded-full font-bold uppercase">
                                                    Master
                                                </span>
                                            )}
                                        </div>
                                        <div className="grid grid-cols-2 gap-3">
                                            <div>
                                                <Label className="text-xs text-brand-muted mb-1 block">Họ tên {isMaster ? "*" : ""}</Label>
                                                <Input
                                                    value={guest.full_name}
                                                    onChange={(e) => updateGuestField(roomId, 0, "full_name", e.target.value)}
                                                    placeholder="Nguyễn Văn A"
                                                    className="text-sm"
                                                />
                                            </div>
                                            <div>
                                                <Label className="text-xs text-brand-muted mb-1 block">Số CCCD {isMaster ? "*" : ""}</Label>
                                                <Input
                                                    value={guest.doc_number}
                                                    onChange={(e) => updateGuestField(roomId, 0, "doc_number", e.target.value)}
                                                    placeholder="001234567890"
                                                    className="text-sm"
                                                />
                                            </div>
                                        </div>
                                    </div>
                                );
                            })}
                        </div>
                    )}

                    {/* Step 4: Summary & Payment */}
                    {step === 3 && (
                        <div className="space-y-4">
                            <div className="bg-slate-50 rounded-xl p-4 space-y-2">
                                <div className="flex justify-between text-sm">
                                    <span className="text-brand-muted">Đoàn</span>
                                    <span className="font-bold">{groupName}</span>
                                </div>
                                <div className="flex justify-between text-sm">
                                    <span className="text-brand-muted">Trưởng đoàn</span>
                                    <span className="font-medium">{organizerName}</span>
                                </div>
                                <div className="flex justify-between text-sm">
                                    <span className="text-brand-muted">Số phòng</span>
                                    <span className="font-medium">{selectedRooms.length} phòng × {nights} đêm</span>
                                </div>
                                <div className="flex justify-between text-sm">
                                    <span className="text-brand-muted">Phòng đại diện</span>
                                    <span className="font-medium">{rooms.find((r) => r.id === masterRoomId)?.name || masterRoomId}</span>
                                </div>
                                {isReservation && (
                                    <div className="flex justify-between text-sm">
                                        <span className="text-brand-muted">Ngày nhận phòng</span>
                                        <span className="font-medium text-amber-600">{checkInDate}</span>
                                    </div>
                                )}
                                <hr className="border-slate-200" />
                                {selectedRooms.map((id) => {
                                    const room = rooms.find((r) => r.id === id);
                                    return (
                                        <div key={id} className="flex justify-between text-sm">
                                            <span>{room?.name || id}</span>
                                            <span className="font-medium">{fmtMoney((room?.base_price || 0) * nights)}</span>
                                        </div>
                                    );
                                })}
                                <hr className="border-slate-200" />
                                <div className="flex justify-between text-base font-bold">
                                    <span>Tổng cộng</span>
                                    <span className="text-brand-primary">{fmtMoney(totalPrice)}</span>
                                </div>
                            </div>

                            <div>
                                <Label className="text-xs font-semibold text-brand-muted uppercase mb-1.5 block">Trả trước</Label>
                                <Input
                                    type="number"
                                    value={paidAmount}
                                    onChange={(e) => setPaidAmount(+e.target.value)}
                                    placeholder="0"
                                />
                            </div>

                            <div>
                                <Label className="text-xs font-semibold text-brand-muted uppercase mb-1.5 block">Ghi chú</Label>
                                <Input value={notes} onChange={(e) => setNotes(e.target.value)} placeholder="Ghi chú thêm..." />
                            </div>
                        </div>
                    )}
                </div>

                {/* Bottom navigation */}
                <div className="px-6 py-4 border-t border-slate-100 flex justify-between">
                    <Button
                        variant="outline"
                        onClick={() => (step > 0 ? setStep(step - 1) : setGroupCheckinOpen(false))}
                        className="rounded-xl"
                    >
                        <ChevronLeft size={16} className="mr-1" />
                        {step > 0 ? "Quay lại" : "Đóng"}
                    </Button>

                    {step < 3 ? (
                        <Button
                            onClick={() => setStep(step + 1)}
                            disabled={!canNext()}
                            className="rounded-xl bg-brand-primary text-white"
                        >
                            Tiếp theo <ChevronRight size={16} className="ml-1" />
                        </Button>
                    ) : (
                        <Button
                            onClick={handleSubmit}
                            disabled={loading}
                            className="rounded-xl bg-emerald-500 hover:bg-emerald-600 text-white px-8"
                        >
                            {loading ? "Đang xử lý..." : isReservation ? "📅 Hoàn tất Reservation" : "✅ Hoàn tất Group Check-in"}
                        </Button>
                    )}
                </div>
            </SheetContent>
        </Sheet>
    );
}
