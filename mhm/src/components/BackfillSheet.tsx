import { useEffect, useState } from "react";
import { useHotelStore } from "../stores/useHotelStore";
import { History } from "lucide-react";
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";
import { FormField, FormFieldSelect } from "@/components/shared/FormField";
import { usePricePreview } from "@/hooks/usePricePreview";
import { formatAppError } from "@/lib/appError";
import { createCorrelationId } from "@/lib/correlationId";
import { invokeWriteCommand } from "@/lib/invokeCommand";
import { getRoomTypeLabel } from "@/lib/constants";
import { bookingBalance } from "@/lib/bookingBalance";
import { fmtNumber } from "@/lib/format";
import { addDaysIso, localDateIso, nightsBetween } from "@/lib/timelineSelection";
import { toast } from "sonner";

export interface BackfillPrefill {
    roomId: string;
    checkInDate: string;
    checkOutDate: string;
    stillStaying: boolean;
}

interface Props {
    open: boolean;
    onOpenChange: (v: boolean) => void;
    prefill?: BackfillPrefill;
}

export default function BackfillSheet({ open, onOpenChange, prefill }: Props) {
    const { rooms, fetchRooms } = useHotelStore();
    const [roomId, setRoomId] = useState("");
    const [guestName, setGuestName] = useState("");
    const [guestPhone, setGuestPhone] = useState("");
    const [guestDoc, setGuestDoc] = useState("");
    // Feed thẳng vào danh sách khai báo tạm trú nộp công an — không được bịa,
    // nên có input thật (giá trị dưới đây chỉ là gợi ý ban đầu trong ô, chủ
    // thấy và có thể sửa; những gì gửi đi là những gì chủ thực sự nhập).
    const [guestDob, setGuestDob] = useState("");
    const [guestGender, setGuestGender] = useState("Nam");
    const [guestNationality, setGuestNationality] = useState("Việt Nam");
    const [guestAddress, setGuestAddress] = useState("");
    const [checkInDate, setCheckInDate] = useState("");
    const [checkOutDate, setCheckOutDate] = useState("");
    const [stillStaying, setStillStaying] = useState(false);
    // Nhớ ngày ra *trước khi* tick "Khách còn ở", để tắt lại trả nó về đúng
    // ngày chủ đã kéo — thay vì chốt cứng về hôm nay. `null` nghĩa là không
    // còn giá trị đáng tin để phục hồi (sheet mở sẵn ở chế độ còn ở, hoặc chủ
    // đã tự sửa ô ngày ra trong lúc toggle đang bật) — khi đó rơi về nhánh
    // chốt-hôm-nay cũ như trước.
    const [preToggleCheckOutDate, setPreToggleCheckOutDate] = useState<string | null>(null);
    const [total, setTotal] = useState(0);
    const [totalDirty, setTotalDirty] = useState(false);
    const [paid, setPaid] = useState(0);
    const [paidDirty, setPaidDirty] = useState(false);
    const [source, setSource] = useState("walk-in");
    const [notes, setNotes] = useState("");
    const [submitting, setSubmitting] = useState(false);

    // Không phụ thuộc vào *reference* của `prefill` — cha (Reservations.tsx)
    // dựng object này inline mỗi lần render, nên phụ thuộc vào các trường
    // nguyên thuỷ bên trong mới tránh được việc effect chạy lại (và xoá dữ
    // liệu người dùng vừa gõ) chỉ vì cha re-render vì lý do khác. Cùng bài
    // học đã áp dụng cho prefillDates ở ReservationSheet.tsx.
    useEffect(() => {
        if (!open) return;
        fetchRooms();
        setRoomId(prefill?.roomId ?? "");
        setCheckInDate(prefill?.checkInDate ?? "");
        setCheckOutDate(prefill?.checkOutDate ?? "");
        setStillStaying(prefill?.stillStaying ?? false);
        setPreToggleCheckOutDate(null);
        setGuestName("");
        setGuestPhone("");
        setGuestDoc("");
        setGuestDob("");
        setGuestGender("Nam");
        setGuestNationality("Việt Nam");
        setGuestAddress("");
        setTotal(0);
        setTotalDirty(false);
        setPaid(0);
        setPaidDirty(false);
        setSource("walk-in");
        setNotes("");
        setSubmitting(false);
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [open, prefill?.roomId, prefill?.checkInDate, prefill?.checkOutDate, prefill?.stillStaying]);

    const nights = nightsBetween(checkInDate, checkOutDate);
    const datesValid = nights > 0;

    // Form ghi bù chỉ có một khách chính (không có ô "số khách" như
    // ReservationSheet) — engine giá được gọi với guests: 1 cho đúng số
    // người thực sự ở, không phải một hằng số bịa ra.
    const { preview, loading: pricingLoading, error: pricingError } = usePricePreview({
        roomId,
        checkIn: checkInDate,
        checkOut: checkOutDate,
        guests: 1,
        debounceMs: 200,
    });

    // Gợi ý theo bảng giá cho tới khi chủ sửa tay ô Tiền phòng.
    useEffect(() => {
        if (!totalDirty && preview) setTotal(preview.total);
    }, [preview, totalDirty]);

    // Khách đã trả phòng: mặc định đã thu đủ, cho tới khi chủ sửa tay.
    // Khách còn ở: mặc định 0 — khách chưa trả phòng nên không giả định đã
    // thu đủ. Guard cũ là `!paidDirty && !stillStaying`: đúng chiều untick
    // (còn ở → đã trả) nhưng bỏ sót chiều tick (đã trả → còn ở) vì điều kiện
    // hoá false ngay khi stillStaying thành true, nên "Đã thu" bị kẹt ở giá
    // trị total cũ và gửi đi paid_amount sai cho một khách chưa trả phòng.
    useEffect(() => {
        if (!paidDirty) setPaid(stillStaying ? 0 : total);
    }, [total, paidDirty, stillStaying]);

    // Khách còn ở chỉ ghi bù được vào phòng đang trống — backend enforce lại
    // rule này, đây chỉ là phản ánh lên danh sách chọn cho khỏi chọn nhầm.
    const selectableRooms = stillStaying ? rooms.filter((r) => r.status === "vacant") : rooms;

    // Bật "Khách còn ở" có thể lọc mất phòng đang chọn (không còn vacant) —
    // <select> khi đó không có option khớp và hiện trống, nhưng state vẫn giữ
    // (và vẫn gửi đi) roomId đó nếu không xoá ở đây. Trước đây deps chỉ có
    // [stillStaying, rooms], thiếu roomId — nếu sheet mở lại với prefill mới
    // (roomId khác) trong khi stillStaying đã sẵn true và rooms không đổi
    // reference, effect không chạy lại và không dọn roomId cũ. Depend thẳng
    // vào roomId và selectableRooms (danh sách đã lọc, đúng những gì effect
    // dùng) để effect luôn thấy đúng giá trị mới nhất. Không lặp vô hạn: khi
    // effect tự setRoomId(""), lần chạy kế tiếp roomId rỗng nên `if (roomId
    // && ...)` chặn ngay ở điều kiện đầu — no-op. Khi roomId đang hợp lệ
    // (có trong selectableRooms), `!selectableRooms.some(...)` là false nên
    // cũng no-op — dù selectableRooms là mảng mới mỗi lần stillStaying=true
    // khiến effect chạy lại nhiều hơn cần thiết, nó không bao giờ gọi
    // setRoomId khi không cần, nên không thể lặp.
    useEffect(() => {
        // `rooms.length > 0`: các sheet này gọi fetchRooms() lúc mở, nên có một
        // khoảnh khắc danh sách còn rỗng. Không có vế này, một phòng hợp lệ
        // truyền vào bị xoá ngay trước khi rooms kịp về, và effect nạp
        // preSelected không chạy lại để đặt lại nó.
        if (rooms.length > 0 && roomId && !selectableRooms.some((r) => r.id === roomId)) {
            setRoomId("");
        }
    }, [rooms, roomId, selectableRooms]);

    // Backend vẫn validate lại — đây chỉ là rào chắn trong form cho khỏi chọn
    // nhầm một ngày rõ ràng vô lý (ghi bù mà ngày vào ở tương lai, hoặc ngày
    // ra "đã trả phòng" lại nằm sau hôm nay).
    const todayIso = localDateIso(new Date());

    // Cùng vị từ với `CheckoutSettlementModal` và hai màn hình hiện số dư: thu
    // nhiều hơn tiền phòng là "thu quá". Chỗ này từng viết lại `paid > total`
    // bằng tay — cùng một câu hỏi, phát biểu lần thứ tư.
    const paidTooHigh = bookingBalance(total, paid).kind === "overpaid";
    const paidNegative = paid < 0;
    const canSubmit =
        !!roomId &&
        guestName.trim().length > 0 &&
        !!checkInDate &&
        !!checkOutDate &&
        datesValid &&
        // Bảng giá hỏng thì `total` đứng nguyên ở 0 và không gì chặn một lượt
        // lưu trú 0₫ đi vào sổ — vừa sai sổ sách, vừa kéo lệch doanh thu.
        total > 0 &&
        !paidTooHigh &&
        !paidNegative &&
        !submitting;

    async function handleSubmit() {
        if (!canSubmit) return;
        setSubmitting(true);
        try {
            const correlationId = createCorrelationId();
            await invokeWriteCommand(
                "backfill_stay",
                {
                    req: {
                        room_id: roomId,
                        guests: [
                            {
                                full_name: guestName,
                                doc_number: guestDoc,
                                phone: guestPhone,
                                dob: guestDob,
                                gender: guestGender,
                                nationality: guestNationality,
                                address: guestAddress,
                            },
                        ],
                        check_in_date: checkInDate,
                        check_out_date: stillStaying ? null : checkOutDate,
                        expected_checkout_date: stillStaying ? checkOutDate : null,
                        total_price: total,
                        paid_amount: paid,
                        source,
                        notes: notes || null,
                    },
                },
                { correlationId },
            );
            toast.success(stillStaying ? "Đã ghi bù khách đang ở!" : "Đã ghi bù khách đã trả phòng!");
            onOpenChange(false);
            fetchRooms();
        } catch (e) {
            toast.error(formatAppError(e));
        }
        setSubmitting(false);
    }

    return (
        <Sheet open={open} onOpenChange={onOpenChange}>
            <SheetContent side="right" className="w-[480px] sm:w-[520px] overflow-y-auto p-0">
                <SheetHeader className="p-6 pb-4 border-b border-slate-100">
                    <SheetTitle className="flex items-center gap-2 text-lg">
                        <History size={20} className="text-amber-600" />
                        Ghi bù sổ khách
                    </SheetTitle>
                    <p className="text-sm text-slate-500">
                        Ghi lại khách đã ở mà quên nhập — khách sẽ vào danh sách khai báo tạm trú.
                    </p>
                </SheetHeader>

                <div className="p-6 space-y-5">
                    <div className="space-y-1.5">
                        <label htmlFor="backfill-room" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">
                            Phòng
                        </label>
                        <select
                            id="backfill-room"
                            className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-amber-200"
                            value={roomId}
                            onChange={(e) => setRoomId(e.target.value)}
                        >
                            <option value="">— Chọn phòng —</option>
                            {selectableRooms.map((r) => (
                                <option key={r.id} value={r.id}>
                                    {r.name} ({getRoomTypeLabel(r.type)})
                                </option>
                            ))}
                        </select>
                    </div>

                    <label className="flex items-center gap-2 text-sm font-medium text-slate-700 cursor-pointer">
                        <input
                            type="checkbox"
                            checked={stillStaying}
                            onChange={(e) => {
                                const nextStillStaying = e.target.checked;
                                setStillStaying(nextStillStaying);
                                if (nextStillStaying) {
                                    // Nhớ lại ngày ra hiện tại (ngày chủ thực sự đã kéo/nhập)
                                    // trước khi ghi đè nó, để tắt toggle sau này biết phục hồi
                                    // đúng giá trị thay vì chốt cứng về hôm nay.
                                    setPreToggleCheckOutDate(checkOutDate || null);
                                    // §4: ngày ra dự kiến lấy theo cuối vùng kéo khi vùng đó
                                    // chạm tương lai, còn không thì mặc định NGÀY MAI. Đổi nhãn
                                    // và min/max thôi là chưa đủ: `datesValid` chỉ soi nights > 0
                                    // nên nút gửi vẫn sáng với một ngày ra trong quá khứ, và
                                    // backend từ chối bằng "Ngày ra dự kiến phải sau hôm nay."
                                    // `min` không cứu được — không có <form> bao quanh và nút
                                    // không phải submit, đúng lý do đã áp cho step="1" ở ô tiền.
                                    // Ngày tương lai chủ đã chọn thì giữ nguyên, không đè.
                                    if (!checkOutDate || checkOutDate <= todayIso) {
                                        setCheckOutDate(addDaysIso(todayIso, 1));
                                    }
                                } else {
                                    if (preToggleCheckOutDate && preToggleCheckOutDate <= todayIso) {
                                        // Bug thật đã sửa: trước đây nhánh này chỉ biết chốt về
                                        // `todayIso` khi ngày ra đang ở tương lai — nó xoá mất
                                        // ngày chủ đã kéo thật (vd "hôm qua") và âm thầm biến
                                        // một lượt ở đúng thành dài thêm một đêm mà không ai
                                        // hay biết. Còn giá trị nhớ được (và nó hợp lệ cho chế
                                        // độ "đã trả phòng", tức không nằm ở tương lai) thì
                                        // phục hồi đúng ngày đó.
                                        setCheckOutDate(preToggleCheckOutDate);
                                    } else if (checkOutDate > todayIso) {
                                        // Không có gì đáng tin để phục hồi (sheet mở sẵn ở chế
                                        // độ còn ở, hoặc chủ đã tự sửa ô ngày ra trong lúc
                                        // toggle đang bật) — rơi về hành vi cũ: "đã trả phòng"
                                        // mà ngày ra ở tương lai cũng bị backend từ chối, kéo
                                        // về hôm nay — đúng `max` của ô ở chế độ này.
                                        setCheckOutDate(todayIso);
                                    }
                                    // Chỉ dọn giá trị nhớ được ở nhánh tắt toggle — nhánh bật
                                    // toggle ở trên vừa mới ghi nó, xoá ngay tại đây (thay vì
                                    // chỉ trong nhánh tắt) sẽ xoá mất giá trị vừa ghi đó.
                                    setPreToggleCheckOutDate(null);
                                }
                            }}
                        />
                        Khách còn ở (chưa trả phòng)
                    </label>

                    <div className="grid grid-cols-2 gap-3">
                        <div className="space-y-1.5">
                            <label htmlFor="backfill-checkin" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">
                                Ngày vào
                            </label>
                            <input
                                id="backfill-checkin"
                                type="date"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-amber-200"
                                value={checkInDate}
                                max={todayIso}
                                onChange={(e) => setCheckInDate(e.target.value)}
                            />
                        </div>
                        <div className="space-y-1.5">
                            <label htmlFor="backfill-checkout" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">
                                {stillStaying ? "Ngày ra dự kiến" : "Ngày ra"}
                            </label>
                            <input
                                id="backfill-checkout"
                                type="date"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-amber-200"
                                value={checkOutDate}
                                min={stillStaying ? addDaysIso(todayIso, 1) : undefined}
                                max={stillStaying ? undefined : todayIso}
                                onChange={(e) => {
                                    setCheckOutDate(e.target.value);
                                    // Chủ tự sửa ngày trong lúc toggle đang bật: giá trị nhớ
                                    // trước-toggle không còn đáng tin để phục hồi khi tắt lại
                                    // (chủ vừa đổi ý) — bỏ nó, tắt toggle sẽ rơi về nhánh chốt
                                    // hôm nay như hành vi cũ.
                                    if (stillStaying) setPreToggleCheckOutDate(null);
                                }}
                            />
                        </div>
                    </div>

                    {checkInDate && checkOutDate && !datesValid && (
                        <div className="rounded-xl p-3 text-sm bg-red-50 text-red-700 border border-red-200">
                            Ngày ra phải sau ngày vào ít nhất 1 đêm.
                        </div>
                    )}

                    <div className="space-y-3">
                        <h3 className="text-xs font-semibold text-slate-500 uppercase tracking-wider">Thông tin khách</h3>
                        <input
                            placeholder="Họ và tên *"
                            className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-amber-200"
                            value={guestName}
                            onChange={(e) => setGuestName(e.target.value)}
                        />
                        <div className="grid grid-cols-2 gap-3">
                            <input
                                placeholder="Số điện thoại"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-amber-200"
                                value={guestPhone}
                                onChange={(e) => setGuestPhone(e.target.value)}
                            />
                            <input
                                placeholder="Số CCCD (tùy chọn)"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-amber-200"
                                value={guestDoc}
                                onChange={(e) => setGuestDoc(e.target.value)}
                            />
                        </div>
                        {/* Feed thẳng vào khai báo tạm trú nộp công an — cùng bộ
                            trường và cách dùng FormField/FormFieldSelect như chế
                            độ "Đầy đủ" của CheckinSheet.tsx, cho hai form cùng một
                            "cảm giác sản phẩm". Giá trị mặc định chỉ là gợi ý ban
                            đầu trong ô; chủ thấy và có thể sửa trước khi gửi. */}
                        <div className="grid grid-cols-2 gap-3">
                            <FormField label="Ngày sinh" value={guestDob} onChange={setGuestDob} />
                            <FormFieldSelect
                                label="Giới tính"
                                value={guestGender}
                                options={["Nam", "Nữ"]}
                                onChange={setGuestGender}
                            />
                            <FormField label="Quốc tịch" value={guestNationality} onChange={setGuestNationality} />
                            <FormField label="Địa chỉ" value={guestAddress} onChange={setGuestAddress} />
                        </div>
                    </div>

                    <div className="grid grid-cols-2 gap-3">
                        <div className="space-y-1.5">
                            <label htmlFor="backfill-total" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">
                                Tiền phòng ({nights} đêm)
                            </label>
                            <input
                                id="backfill-total"
                                type="number"
                                step="1"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-amber-200"
                                value={total}
                                onChange={(e) => {
                                    // Money là MoneyVnd (số nguyên) ở backend. Không có <form> bao
                                    // quanh nên constraint validation của trình duyệt (stepMismatch
                                    // từ step="1") không bao giờ chạy trước khi handleSubmit gọi API
                                    // — phải tự làm tròn ở đây, không được dựa vào step.
                                    setTotal(Math.round(Number(e.target.value) || 0));
                                    setTotalDirty(true);
                                }}
                            />
                            {pricingLoading && !preview && (
                                <p className="text-xs text-slate-400">Đang tính giá gợi ý...</p>
                            )}
                            {pricingError && (
                                <p className="text-xs text-red-500">Không tính được giá gợi ý — có thể sửa tay.</p>
                            )}
                        </div>
                        <div className="space-y-1.5">
                            <label htmlFor="backfill-paid" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">
                                Đã thu
                            </label>
                            <input
                                id="backfill-paid"
                                type="number"
                                step="1"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-amber-200"
                                value={paid}
                                onChange={(e) => {
                                    // Cùng lý do với ô Tiền phòng ở trên — làm tròn ở đây thay vì
                                    // trông cậy vào step="1" (không có tác dụng khi không có <form>).
                                    setPaid(Math.round(Number(e.target.value) || 0));
                                    setPaidDirty(true);
                                }}
                            />
                        </div>
                    </div>
                    {(paidTooHigh || paidNegative) && (
                        <p className="text-xs text-red-600 font-medium">
                            {paidTooHigh
                                ? `Đã thu (${fmtNumber(paid)}₫) không được vượt quá tiền phòng (${fmtNumber(total)}₫).`
                                : "Đã thu không được nhỏ hơn 0."}
                        </p>
                    )}

                    <div className="grid grid-cols-2 gap-3">
                        <div className="space-y-1.5">
                            <label htmlFor="backfill-source" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">
                                Nguồn
                            </label>
                            <select
                                id="backfill-source"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-amber-200"
                                value={source}
                                onChange={(e) => setSource(e.target.value)}
                            >
                                <option value="walk-in">Walk-in</option>
                                <option value="phone">Điện thoại</option>
                                <option value="zalo">Zalo</option>
                                <option value="agoda">Agoda</option>
                                <option value="booking.com">Booking.com</option>
                                <option value="other">Khác</option>
                            </select>
                        </div>
                        <div className="space-y-1.5">
                            <label htmlFor="backfill-notes" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">
                                Ghi chú
                            </label>
                            <input
                                id="backfill-notes"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-amber-200"
                                value={notes}
                                onChange={(e) => setNotes(e.target.value)}
                            />
                        </div>
                    </div>

                    <Button
                        className="w-full h-12 rounded-xl bg-amber-600 hover:bg-amber-700 text-white font-semibold text-sm cursor-pointer"
                        onClick={handleSubmit}
                        disabled={!canSubmit}
                    >
                        {submitting ? "Đang xử lý..." : "Ghi bù vào sổ"}
                    </Button>
                </div>
            </SheetContent>
        </Sheet>
    );
}
