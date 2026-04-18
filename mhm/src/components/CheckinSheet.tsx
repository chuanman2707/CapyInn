import { useState, useEffect, useMemo, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { useHotelStore } from "../stores/useHotelStore";
import { UserPlus, Trash2, Scan, CheckCircle2, AlertTriangle } from "lucide-react";
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";
import { FormField, FormFieldSelect } from "@/components/shared/FormField";
import { useAvailability } from "@/hooks/useAvailability";
import { getRoomTypeLabel } from "@/lib/constants";
import { fmtMoney } from "@/lib/format";
import { createDeferredCleanup } from "@/lib/deferredCleanup";
import { toast } from "sonner";
import type { CccdInfo, GuestInput, GuestSummary } from "@/types";

const emptyGuest = (): GuestInput => ({
    full_name: "",
    doc_number: "",
    phone: "",
    dob: "",
    gender: "Nam",
    nationality: "Việt Nam",
    address: "",
});

export default function CheckinSheet({ preSelectedRoomId }: { preSelectedRoomId?: string } = {}) {
    const { rooms, checkIn, fetchRooms, isCheckinOpen, setCheckinOpen } = useHotelStore();

    const [guests, setGuests] = useState<GuestInput[]>([emptyGuest()]);
    const [selectedRoom, setSelectedRoom] = useState("");
    const [nights, setNights] = useState(1);
    const [paidAmount, setPaidAmount] = useState(0);
    const [source, setSource] = useState("walk-in");
    const [notes, setNotes] = useState("");
    const [submitting, setSubmitting] = useState(false);
    const [ocrFlash, setOcrFlash] = useState(false);
    const [quickMode, setQuickMode] = useState(true);
    const [suggestions, setSuggestions] = useState<GuestSummary[]>([]);
    const [showSuggestions, setShowSuggestions] = useState(false);
    const formRef = useRef<HTMLDivElement>(null);

    // Auto-select room when pre-selected
    useEffect(() => {
        if (isCheckinOpen && preSelectedRoomId) {
            setSelectedRoom(preSelectedRoomId);
        }
    }, [isCheckinOpen, preSelectedRoomId]);

    // Listen for OCR results from background watcher
    useEffect(() => {
        const cleanupOcrResult = createDeferredCleanup(listen<CccdInfo>("ocr-result", (event) => {
            const cccd = event.payload;

            setGuests((prev) => {
                // If the first guest is still empty, fill it
                const firstEmpty = prev.findIndex(
                    (g) => !g.full_name && !g.doc_number
                );
                if (firstEmpty >= 0) {
                    return prev.map((g, i) =>
                        i === firstEmpty
                            ? {
                                full_name: cccd.full_name,
                                doc_number: cccd.doc_number,
                                phone: "",
                                dob: cccd.dob,
                                gender: cccd.gender,
                                nationality: cccd.nationality,
                                address: cccd.address,
                            }
                            : g
                    );
                }
                // Otherwise add as a new guest
                return [
                    ...prev,
                    {
                        full_name: cccd.full_name,
                        doc_number: cccd.doc_number,
                        phone: "",
                        dob: cccd.dob,
                        gender: cccd.gender,
                        nationality: cccd.nationality,
                        address: cccd.address,
                    },
                ];
            });

            // Open sheet if not open
            setCheckinOpen(true);

            // Flash animation + toast
            setOcrFlash(true);
            setTimeout(() => setOcrFlash(false), 1500);
            toast.success("Đã quét CCCD thành công", {
                description: cccd.full_name || "Thông tin đã được điền tự động",
                icon: <Scan size={16} />,
            });

            fetchRooms();
        }));

        const cleanupOcrError = createDeferredCleanup(listen<string>("ocr-error", (event) => {
            toast.error("Lỗi quét CCCD", {
                description: event.payload,
            });
        }));

        return () => {
            cleanupOcrResult();
            cleanupOcrError();
        };
    }, []);

    // Fetch rooms when sheet opens
    useEffect(() => {
        if (isCheckinOpen) fetchRooms();
    }, [isCheckinOpen]);

    const vacantRooms = rooms.filter((r) => r.status === "vacant");
    const selectedRoomData = rooms.find((r) => r.id === selectedRoom);
    const totalPrice = selectedRoomData
        ? selectedRoomData.base_price * nights
        : 0;

    const { fromDate, toDate } = useMemo(() => {
        const now = new Date();
        const next = new Date(now.getTime() + Math.max(nights, 0) * 86400000);

        return {
            fromDate: now.toISOString().split("T")[0],
            toDate: next.toISOString().split("T")[0],
        };
    }, [nights]);

    const { availability } = useAvailability({
        roomId: selectedRoom,
        fromDate,
        toDate,
        disabled: !selectedRoom || nights <= 0,
    });

    const availWarning = availability && !availability.available && availability.conflicts.length > 0
        ? `Phòng ${selectedRoom} đã có đặt phòng từ ${availability.conflicts[0].date} (${availability.conflicts[0].guest_name || "Đã đặt"}).`
        : null;
    const maxNights = availability?.available ? null : availability?.max_nights ?? null;

    const updateGuest = (idx: number, field: keyof GuestInput, val: string) => {
        setGuests((prev) =>
            prev.map((g, i) => (i === idx ? { ...g, [field]: val } : g))
        );
    };

    const addGuest = () => {
        setGuests((prev) => [...prev, emptyGuest()]);
    };

    const removeGuest = (idx: number) => {
        if (guests.length > 1) setGuests((prev) => prev.filter((_, i) => i !== idx));
    };

    const handleCheckin = async () => {
        const hasIdentifier = quickMode ? guests[0]?.phone : guests[0]?.doc_number;
        if (!selectedRoom || !guests[0]?.full_name || !hasIdentifier) return;
        setSubmitting(true);
        try {
            await checkIn(selectedRoom, guests, nights, paidAmount, source, notes);
            closeAll();
            toast.success("Check-in thành công!");
        } catch (err) {
            toast.error("Lỗi check-in: " + err);
        }
        setSubmitting(false);
    };

    const closeAll = () => {
        setGuests([emptyGuest()]);
        setSelectedRoom("");
        setNights(1);
        setPaidAmount(0);
        setSource("walk-in");
        setNotes("");
        setCheckinOpen(false);
    };

    const canSubmit =
        selectedRoom && guests[0]?.full_name && (quickMode ? guests[0]?.phone : guests[0]?.doc_number) && !submitting;

    // Auto-suggest returning guest by phone
    const handlePhoneChange = async (idx: number, phone: string) => {
        updateGuest(idx, "phone", phone);
        if (idx === 0 && phone.length >= 3) {
            try {
                const results = await invoke<GuestSummary[]>("search_guest_by_phone", { phone });
                setSuggestions(results);
                setShowSuggestions(results.length > 0);
            } catch {
                setSuggestions([]);
                setShowSuggestions(false);
            }
        } else {
            setShowSuggestions(false);
        }
    };

    const applySuggestion = (s: GuestSummary) => {
        setGuests((prev) =>
            prev.map((g, i) =>
                i === 0
                    ? { ...g, full_name: s.full_name, doc_number: s.doc_number, nationality: s.nationality || "Việt Nam" }
                    : g
            )
        );
        setShowSuggestions(false);
    };

    return (
        <Sheet
            open={isCheckinOpen}
            onOpenChange={(open) => {
                if (!open) closeAll();
            }}
        >
            <SheetContent className="w-[480px] sm:w-[540px] sm:max-w-[600px] border-l-0 shadow-[-10px_0_40px_rgba(0,0,0,0.1)] p-0 flex flex-col bg-slate-50 overflow-hidden">
                {/* Header */}
                <div className="bg-white border-b border-slate-100 p-6 z-10">
                    <SheetHeader className="text-left space-y-0">
                        <SheetTitle className="text-xl font-bold flex items-center gap-2">
                            <UserPlus className="text-brand-primary" size={22} />
                            Check-in Khách Mới
                        </SheetTitle>
                        <p className="text-sm text-brand-muted mt-1">
                            Điền thông tin khách hàng hoặc sử dụng máy scan để tự động điền
                        </p>
                    </SheetHeader>

                    {/* OCR Status indicator */}
                    <div
                        className={`mt-3 flex items-center gap-2 text-xs font-medium px-3 py-2 rounded-xl transition-all duration-500 ${ocrFlash
                            ? "bg-emerald-50 text-emerald-700 ring-2 ring-emerald-200"
                            : "bg-slate-50 text-brand-muted"
                            }`}
                    >
                        <span
                            className={`w-2 h-2 rounded-full ${ocrFlash ? "bg-emerald-500 animate-pulse" : "bg-green-500"
                                }`}
                        />
                        {ocrFlash ? (
                            <>
                                <CheckCircle2 size={14} />
                                Đã nhận kết quả scan!
                            </>
                        ) : (
                            "Scanner sẵn sàng — đặt CCCD lên máy scan để tự động điền"
                        )}
                    </div>
                </div>

                {/* Mode Toggle */}
                <div className="flex gap-1 bg-slate-100 rounded-xl p-1 mx-6 mt-3">
                    <button
                        onClick={() => setQuickMode(true)}
                        className={`flex-1 py-2 px-3 rounded-lg text-xs font-bold transition-all cursor-pointer ${quickMode ? "bg-white text-brand-primary shadow-sm" : "text-brand-muted hover:text-brand-text"
                            }`}
                    >
                        ⚡ Nhanh
                    </button>
                    <button
                        onClick={() => setQuickMode(false)}
                        className={`flex-1 py-2 px-3 rounded-lg text-xs font-bold transition-all cursor-pointer ${!quickMode ? "bg-white text-brand-primary shadow-sm" : "text-brand-muted hover:text-brand-text"
                            }`}
                    >
                        📋 Đầy đủ
                    </button>
                </div>

                {/* Form Content */}
                <div ref={formRef} className="flex-1 overflow-y-auto p-6 space-y-5">
                    {/* Guest Forms */}
                    {guests.map((g, idx) => (
                        <div
                            key={idx}
                            className={`bg-white rounded-2xl p-5 shadow-soft border relative overflow-hidden transition-all duration-500 ${ocrFlash && (g.full_name || g.doc_number)
                                ? "border-emerald-300 ring-2 ring-emerald-100"
                                : "border-slate-100"
                                }`}
                        >
                            {/* Left accent */}
                            <div className="absolute top-0 left-0 w-1 h-full bg-brand-primary rounded-l-2xl" />

                            <div className="flex items-center justify-between mb-4 pl-2">
                                <span className="text-sm font-bold text-brand-text">
                                    Khách {idx + 1} {idx === 0 && "(chính)"}
                                </span>
                                {idx > 0 && (
                                    <button
                                        onClick={() => removeGuest(idx)}
                                        className="text-red-500 hover:text-red-600 bg-red-50 p-1.5 rounded-lg transition-colors cursor-pointer"
                                    >
                                        <Trash2 size={14} />
                                    </button>
                                )}
                            </div>

                            <div className="grid grid-cols-2 gap-3 pl-2">
                                <FormField
                                    label="Họ và tên *"
                                    value={g.full_name}
                                    onChange={(v) => updateGuest(idx, "full_name", v)}
                                />
                                <div className="relative">
                                    <FormField
                                        label="Số điện thoại *"
                                        value={g.phone}
                                        onChange={(v) => handlePhoneChange(idx, v)}
                                    />
                                    {idx === 0 && showSuggestions && suggestions.length > 0 && (
                                        <div className="absolute z-50 top-full left-0 right-0 mt-1 bg-white border border-slate-200 rounded-xl shadow-float overflow-hidden">
                                            <p className="px-3 py-1.5 text-[10px] font-bold text-brand-muted uppercase bg-slate-50">Khách cũ</p>
                                            {suggestions.map((s) => (
                                                <button
                                                    key={s.id}
                                                    onClick={() => applySuggestion(s)}
                                                    className="w-full text-left px-3 py-2 hover:bg-blue-50 transition-colors text-sm cursor-pointer"
                                                >
                                                    <span className="font-semibold">{s.full_name}</span>
                                                    <span className="text-brand-muted ml-2">({s.total_stays} lần, CCCD: {s.doc_number})</span>
                                                </button>
                                            ))}
                                        </div>
                                    )}
                                </div>
                                {!quickMode && (
                                    <>
                                        <FormField
                                            label="Số CCCD"
                                            value={g.doc_number}
                                            onChange={(v) => updateGuest(idx, "doc_number", v)}
                                        />
                                        <FormField
                                            label="Ngày sinh"
                                            value={g.dob}
                                            onChange={(v) => updateGuest(idx, "dob", v)}
                                        />
                                        <FormFieldSelect
                                            label="Giới tính"
                                            value={g.gender}
                                            options={["Nam", "Nữ"]}
                                            onChange={(v) => updateGuest(idx, "gender", v)}
                                        />
                                        <FormField
                                            label="Quốc tịch"
                                            value={g.nationality}
                                            onChange={(v) => updateGuest(idx, "nationality", v)}
                                        />
                                        <FormField
                                            label="Địa chỉ"
                                            value={g.address}
                                            onChange={(v) => updateGuest(idx, "address", v)}
                                        />
                                    </>
                                )}
                            </div>
                        </div>
                    ))}

                    <button
                        onClick={addGuest}
                        className="flex items-center gap-1.5 text-sm text-brand-primary hover:text-brand-primary/80 font-semibold cursor-pointer transition-colors"
                    >
                        <UserPlus size={15} /> Thêm khách
                    </button>
                </div>

                {/* Footer: Booking Details + Submit */}
                <div className="bg-white p-6 border-t border-slate-100 mt-auto z-10 space-y-4">
                    <h3 className="font-bold text-brand-text text-sm">
                        Thông tin nhận phòng
                    </h3>

                    <div className="grid grid-cols-2 gap-3">
                        {/* Room selector */}
                        <div>
                            <label className="text-xs font-semibold text-brand-muted block mb-1.5 ml-1">
                                Phòng *
                            </label>
                            <select
                                value={selectedRoom}
                                onChange={(e) => setSelectedRoom(e.target.value)}
                                className="w-full bg-slate-50 border border-slate-100 focus:border-brand-primary/50 focus:ring-2 focus:ring-brand-primary/20 rounded-xl px-3 py-2.5 text-brand-text text-sm font-medium outline-none transition-all"
                            >
                                <option value="">Chọn phòng...</option>
                                {vacantRooms.map((r) => (
                                <option key={r.id} value={r.id}>
                                        {r.id} — {getRoomTypeLabel(r.type)} (
                                        {fmtMoney(r.base_price)})
                                    </option>
                                ))}
                            </select>
                        </div>

                        {/* Nights */}
                        <FormField
                            label="Số đêm"
                            value={String(nights)}
                            type="number"
                            onChange={(v) => setNights(Number(v) || 1)}
                        />

                        {/* Paid amount */}
                        <FormField
                            label="Trả trước"
                            value={String(paidAmount)}
                            type="number"
                            onChange={(v) => setPaidAmount(Number(v) || 0)}
                        />

                        {/* Source */}
                        <FormFieldSelect
                            label="Nguồn"
                            value={source}
                            options={["walk-in", "agoda", "booking.com", "phone"]}
                            onChange={setSource}
                        />
                    </div>

                    {/* Notes */}
                    <FormField label="Ghi chú" value={notes} onChange={setNotes} />

                    {/* Availability Warning */}
                    {availWarning && (
                        <div className="bg-amber-50 border border-amber-200 rounded-xl p-3 space-y-1">
                            <div className="flex items-center gap-1.5 text-amber-700 text-sm font-semibold">
                                <AlertTriangle size={14} />
                                Cảnh báo xung đột
                            </div>
                            <p className="text-xs text-amber-600">{availWarning}</p>
                            {maxNights != null && maxNights > 0 && (
                                <p className="text-xs text-amber-700 font-medium">
                                    💡 Tối đa {maxNights} đêm. <button className="underline cursor-pointer" onClick={() => setNights(maxNights!)}>Điều chỉnh</button>
                                </p>
                            )}
                        </div>
                    )}

                    {/* Total price */}
                    {selectedRoomData && (
                        <div className="bg-slate-50 rounded-xl p-3 flex justify-between items-center">
                            <span className="text-xs text-brand-muted font-medium">
                                Tổng tiền ({nights} đêm × {fmtMoney(selectedRoomData.base_price)})
                            </span>
                            <span className="text-base font-bold text-emerald-600 tabular-nums">
                                {fmtMoney(totalPrice)}
                            </span>
                        </div>
                    )}

                    {/* Submit */}
                    <Button
                        onClick={handleCheckin}
                        disabled={!canSubmit}
                        className="w-full h-12 bg-brand-primary hover:bg-brand-primary/90 text-white rounded-xl text-sm font-bold shadow-soft hover:shadow-float active:scale-[0.98] transition-all"
                    >
                        {submitting
                            ? "Đang xử lý..."
                            : `Hoàn tất Check-in cho ${guests.length} khách`}
                    </Button>
                </div>
            </SheetContent>
        </Sheet>
    );
}
