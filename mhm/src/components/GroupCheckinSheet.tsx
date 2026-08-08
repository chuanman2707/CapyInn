import { useState, useEffect, useMemo } from "react";
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
import RateOverrideField from "@/components/shared/RateOverrideField";
import { formatAppError } from "@/lib/appError";
import { fmtMoney } from "@/lib/format";
import { addDays, addDaysIso, localDateIso, localRfc3339 } from "@/lib/timelineSelection";
import { useLocalDay } from "@/hooks/useLocalDay";
import { sumRoomPricesWithOverrides, useRoomPrices } from "@/hooks/useRoomPrices";
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
    // Ngày địa phương. `toISOString()` trả ngày UTC, mà trước 07:00 giờ Việt Nam
    // vẫn còn là hôm qua — đủ để mặc định sai ngày nhận phòng và để
    // `isReservation` phán nhầm chế độ ngay trong ca đêm.
    const [checkInDate, setCheckInDate] = useState(() => localDateIso(new Date()));

    // Qua nửa đêm là `todayStr` đổi theo, nên một sheet mở xuyên ca đêm không
    // còn phân loại một đoàn khách vãng lai thành đặt trước (hoặc ngược lại) —
    // hai nhánh đó gửi hai dạng ngày khác nhau xuống backend.
    const todayStr = useLocalDay();
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

    // Giá tay theo từng phòng, đơn vị mỗi đêm. Khoá là room_id — khớp
    // `GroupCheckinRequest.rate_override_per_room` phía backend.
    const [rateOverrides, setRateOverrides] = useState<Record<string, number>>({});
    // Phòng vừa được lễ tân sửa/mở giá gần nhất — nguồn cho "Áp cho tất cả
    // phòng". KHÔNG dùng `Object.keys(rateOverrides)[0]`: thứ tự khoá object
    // là thứ tự CHÈN vào map, không phải thứ tự lễ tân thao tác — sửa phòng
    // thứ hai sau khi đã mở (không gõ) phòng thứ nhất sẽ áp nhầm giá của
    // phòng thứ nhất.
    const [lastEditedRoomId, setLastEditedRoomId] = useState<string | null>(null);
    // Tăng lên mỗi lần sheet đóng (xem effect `[isGroupCheckinOpen]` bên
    // dưới) để ép remount `RateOverrideField` — reset `rateOverrides` (data)
    // không đủ, vì trạng thái "đang sửa" nằm nội bộ component con (Task 16,
    // không sửa lại) và chỉ mất đi khi remount. Cùng cách CheckinSheet.tsx /
    // ReservationSheet.tsx đã làm ở Task 17 (commit fc76e43).
    const [priceFormSession, setPriceFormSession] = useState(0);

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
            // Ngày địa phương, cùng lý do đã ghi ở chỗ khởi tạo state phía trên:
            // `toISOString()` trả ngày UTC, mà trước 07:00 giờ Việt Nam vẫn là
            // hôm qua — reset form trong ca đêm sẽ đặt sai ngày nhận phòng.
            setCheckInDate(localDateIso(new Date()));
            setSelectedRooms([]);
            setMasterRoomId("");
            setAssignMode("auto");
            setGuestsPerRoom({});
            setPaidAmount(0);
            setNotes("");
            // Giá tay của đoàn trước không được rò sang đoàn kế tiếp (bài học
            // Task 17: sheet không unmount, chỉ đổi prop `open`).
            setRateOverrides({});
            setLastEditedRoomId(null);
            setPriceFormSession((s) => s + 1);
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
            // Cùng lý do với `toggleRoom`: auto-assign thay HẲN danh sách
            // phòng, nên override của một phòng không còn trong danh sách mới
            // phải bị dọn theo — `group_checkin_tx` từ chối cả giao dịch nếu
            // map còn khoá không nằm trong `room_ids`.
            const idSet = new Set(ids);
            setRateOverrides((current) => {
                const entries = Object.entries(current).filter(([roomId]) => idSet.has(roomId));
                return entries.length === Object.keys(current).length
                    ? current
                    : Object.fromEntries(entries);
            });
            setLastEditedRoomId((current) => (current != null && !idSet.has(current) ? null : current));
            toast.success(`Đã chọn tự động ${ids.length} phòng`);
        } catch (err) {
            toast.error(formatAppError(err));
        }
    };

    const toggleRoom = (id: string) => {
        const wasSelected = selectedRooms.includes(id);
        setSelectedRooms((prev) =>
            prev.includes(id) ? prev.filter((r) => r !== id) : [...prev, id]
        );
        if (wasSelected) {
            // `group_checkin_tx` từ chối cả giao dịch nếu map override còn
            // khoá không nằm trong `room_ids` — dọn ngay tại đường bỏ chọn,
            // đừng đợi tới lúc submit mới phát hiện.
            setRateOverrides((current) => {
                if (!(id in current)) return current;
                const next = { ...current };
                delete next[id];
                return next;
            });
            setLastEditedRoomId((current) => (current === id ? null : current));
        }
    };

    const setRoomRate = (roomId: string, rate: number | null) => {
        setRateOverrides((current) => {
            const next = { ...current };
            if (rate == null) {
                delete next[roomId];
            } else {
                next[roomId] = rate;
            }
            return next;
        });
        // Ghi nhận "vừa sửa" cả khi bấm mở ô giá (prefill) lẫn khi thật sự gõ
        // số mới — cả hai đều là một lượt tương tác với ô giá của phòng này,
        // và đúng là cái "Áp cho tất cả phòng" nên coi là nguồn.
        setLastEditedRoomId(rate == null ? null : roomId);
    };

    // Đoàn hầu như luôn chốt một giá chung; không có nút này thì đoàn 8 phòng
    // phải gõ 8 lần.
    const applyRateToAllRooms = (rate: number) => {
        setRateOverrides(Object.fromEntries(selectedRooms.map((roomId) => [roomId, rate])));
    };

    // Nguồn giá cho nút "Áp cho tất cả phòng": ưu tiên phòng vừa sửa gần
    // nhất; nếu phòng đó đã bị bỏ chọn hoặc chưa từng có (mở sheet lại chẳng
    // hạn) thì rơi về phòng đầu tiên trong `selectedRooms` (giữ đúng THỨ TỰ
    // CHỌN của lễ tân) đang có override.
    const overrideSourceRoomId =
        lastEditedRoomId != null && rateOverrides[lastEditedRoomId] != null
            ? lastEditedRoomId
            : selectedRooms.find((id) => rateOverrides[id] != null) ?? null;

    // `group_lifecycle.rs` tính tiền cho đặt trước bằng ngày trần, còn khách vãng
    // lai bằng `Local::now()` và `now + nights`. Bản xem trước phải hỏi đúng cặp
    // mốc mà nhánh tương ứng sẽ dùng, nếu không con số báo cho khách là của một
    // kỳ nghỉ khác.
    const { quoteCheckIn, quoteCheckOut } = useMemo(() => {
        if (isReservation) {
            return {
                quoteCheckIn: checkInDate,
                quoteCheckOut: addDaysIso(checkInDate, Math.max(nights, 0)),
            };
        }
        const now = new Date();
        return {
            quoteCheckIn: localRfc3339(now),
            quoteCheckOut: localRfc3339(addDays(now, Math.max(nights, 0))),
        };
    }, [checkInDate, isReservation, nights]);

    const {
        byRoomId: priceByRoom,
        loading: priceLoading,
        failed: priceFailed,
    } = useRoomPrices({
        roomIds: selectedRooms,
        checkIn: quoteCheckIn,
        checkOut: quoteCheckOut,
        // `group_lifecycle.rs` charges with `None`, so the group is not billed an
        // extra-person fee. Quoting one would show more than the desk collects.
        guests: null,
        disabled: !isGroupCheckinOpen,
    });

    // `null` khi còn phòng chưa có giá VÀ chưa được sửa giá tay — thà không
    // hiện tổng còn hơn hiện một tổng thiếu mất một phòng. Phòng có override
    // dùng `override × nights`, không cần đợi giá engine của chính nó.
    const totalPrice = sumRoomPricesWithOverrides(selectedRooms, priceByRoom, rateOverrides, nights);

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
                rate_override_per_room: rateOverrides,
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
                                            {/* Không in `base_price`: giá gắn với loại
                                                phòng, nên con số theo từng phòng đặt
                                                cạnh tổng đã báo giá là số không ai thu. */}
                                            <p className="text-xs text-brand-muted">{room.type} • T{room.floor}</p>
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
                                        <div key={id} className="flex justify-between items-start gap-3 text-sm">
                                            <span className="pt-1.5">{room?.name || id}</span>
                                            <RateOverrideField
                                                // Ghép `priceFormSession`: `rateOverrides` (data) reset
                                                // ở effect đóng sheet không đủ để RateOverrideField quay
                                                // lại nút hiển thị — trạng thái "đang sửa" nằm nội bộ
                                                // component con, chỉ mất đi khi remount (Task 17).
                                                key={`${id}:${priceFormSession}`}
                                                engineTotal={priceByRoom[id]?.total ?? null}
                                                nights={nights}
                                                value={rateOverrides[id] ?? null}
                                                onChange={(rate) => setRoomRate(id, rate)}
                                            />
                                        </div>
                                    );
                                })}
                                {overrideSourceRoomId != null && selectedRooms.length > 1 && (
                                    <button
                                        type="button"
                                        onClick={() =>
                                            applyRateToAllRooms(rateOverrides[overrideSourceRoomId])
                                        }
                                        className="text-xs text-indigo-600 underline cursor-pointer"
                                    >
                                        Áp cho tất cả phòng
                                    </button>
                                )}
                                <hr className="border-slate-200" />
                                <div className="flex justify-between text-base font-bold">
                                    <span>Tổng cộng</span>
                                    {priceFailed ? (
                                        <span
                                            data-testid="group-price-error"
                                            className="text-amber-600 text-sm"
                                        >
                                            Chưa tính được giá
                                        </span>
                                    ) : (
                                        <span
                                            data-testid="group-price-total"
                                            className="text-brand-primary tabular-nums"
                                        >
                                            {priceLoading || totalPrice == null ? "…" : fmtMoney(totalPrice)}
                                        </span>
                                    )}
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
