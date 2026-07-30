import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { CalendarDays } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { invokeWriteCommand } from "@/lib/invokeCommand";
import {
    groupSpecialDates,
    overlappingDates,
    type SpecialDateRange,
    type SpecialDateRow,
} from "@/lib/specialDateRanges";

/** Quá ngần này thì liệt kê hết chỉ tổ rối; phần còn lại đếm số. */
const MAX_LISTED_CLASHES = 10;

type PendingWrite = {
    remove: string[];
    from: string;
    to: string;
    label: string;
    upliftPct: number;
};

export default function SpecialDatesSection() {
    const [rows, setRows] = useState<SpecialDateRow[]>([]);
    const [editing, setEditing] = useState<SpecialDateRange | null>(null);
    const [label, setLabel] = useState("");
    const [from, setFrom] = useState("");
    const [to, setTo] = useState("");
    const [upliftPct, setUpliftPct] = useState("30");
    const [clashes, setClashes] = useState<SpecialDateRow[] | null>(null);
    const [pending, setPending] = useState<PendingWrite | null>(null);
    const [deleting, setDeleting] = useState<SpecialDateRange | null>(null);
    const [loadError, setLoadError] = useState(false);

    const reload = useCallback(() => {
        invoke<SpecialDateRow[]>("get_special_dates")
            .then((data) => {
                setRows(data);
                setLoadError(false);
            })
            .catch(() => {
                // Không rõ DB có gì — tuyệt đối không giả vờ là "rỗng", vì đó
                // là dữ liệu mà bảng ghi-đè dùng để so trùng. Rỗng giả sẽ vô
                // hiệu hoá cảnh báo ghi đè.
                setLoadError(true);
                toast.error("Không tải được danh sách đợt cao điểm đã khai");
            });
    }, []);

    useEffect(reload, [reload]);

    const ranges = groupSpecialDates(rows);

    const resetForm = () => {
        setEditing(null);
        setLabel("");
        setFrom("");
        setTo("");
        setUpliftPct("30");
        setClashes(null);
        setPending(null);
    };

    const startEdit = (range: SpecialDateRange) => {
        setEditing(range);
        setLabel(range.label);
        setFrom(range.from);
        setTo(range.to);
        setUpliftPct(String(range.uplift_pct));
    };

    const write = async (request: PendingWrite) => {
        try {
            await invokeWriteCommand("save_special_date_range", request);
            toast.success("Đã lưu đợt cao điểm");
            resetForm();
            reload();
        } catch (error) {
            toast.error(error instanceof Error ? error.message : String(error));
        }
    };

    const handleSave = () => {
        // Bất kỳ lần bấm lưu nào cũng coi như từ bỏ hộp ghi-đè cũ (nếu có) —
        // nó có thể đang mô tả một yêu cầu đã lỗi thời.
        setClashes(null);
        setPending(null);

        if (loadError) {
            toast.error(
                "Chưa tải được danh sách đợt đã khai nên không thể so trùng an toàn. Tải lại trang rồi thử lại.",
            );
            return;
        }

        const trimmed = label.trim();
        if (!trimmed || !from || !to || to < from) {
            toast.error("Điền tên đợt và khoảng ngày hợp lệ");
            return;
        }

        const trimmedUplift = upliftPct.trim();
        const upliftValue = Number(trimmedUplift);
        if (
            trimmedUplift === "" ||
            !Number.isFinite(upliftValue) ||
            upliftValue < 0 ||
            upliftValue > 500
        ) {
            toast.error("Nhập % phụ thu hợp lệ, từ 0 đến 500");
            return;
        }

        // Ngày rơi ra khỏi khoảng mới phải bị xoá thật, nếu không nó nằm lại
        // thành một ngày lễ mồ côi. Chúng đi kèm lệnh ghi chứ không thành một
        // lệnh xoá riêng, để cả hai nửa chung một transaction.
        const remove = (editing?.dates ?? []).filter((date) => date < from || date > to);
        const request: PendingWrite = {
            remove,
            from,
            to,
            label: trimmed,
            upliftPct: upliftValue,
        };

        const conflicts = overlappingDates(rows, from, to, editing?.dates ?? []);
        if (conflicts.length > 0) {
            setClashes(conflicts);
            setPending(request);
            return;
        }

        void write(request);
    };

    const handleDelete = async (range: SpecialDateRange) => {
        try {
            await invokeWriteCommand("delete_special_dates", { dates: range.dates });
            toast.success("Đã xoá đợt cao điểm");
            resetForm();
            reload();
        } catch (error) {
            toast.error(error instanceof Error ? error.message : String(error));
        } finally {
            setDeleting(null);
        }
    };

    return (
        <div className="space-y-6">
            <div>
                <h3 className="text-lg font-bold mb-1 flex items-center gap-2">
                    <CalendarDays size={20} className="text-emerald-500" />
                    Mùa cao điểm
                </h3>
                <p className="text-sm text-brand-muted">
                    Khai những đợt tăng giá theo ngày — Tết, lễ, mùa du lịch. Giá phòng tự cộng
                    thêm cho đúng những đêm nằm trong đợt.
                </p>
            </div>

            {loadError ? (
                <p className="text-sm text-red-600">
                    Không tải được danh sách đợt cao điểm đã khai. Tải lại trang trước khi thêm
                    hoặc sửa, để tránh ghi đè nhầm ngày đã khai.
                </p>
            ) : ranges.length === 0 ? (
                <p className="text-sm text-brand-muted">
                    Chưa khai đợt cao điểm nào. Thêm một đợt ở dưới.
                </p>
            ) : (
                <div className="space-y-2">
                    {ranges.map((range) => (
                        <div
                            key={`${range.from}-${range.label}`}
                            className="flex items-center justify-between p-4 bg-slate-50 rounded-xl"
                        >
                            <div>
                                <p className="font-semibold text-sm">{range.label}</p>
                                <p className="text-xs text-brand-muted">
                                    {range.from} – {range.to} ({range.days} ngày) &nbsp;|&nbsp; +
                                    {range.uplift_pct}%
                                </p>
                            </div>
                            <div className="flex gap-2">
                                <Button
                                    variant="outline"
                                    size="sm"
                                    className="rounded-lg"
                                    onClick={() => startEdit(range)}
                                >
                                    Sửa
                                </Button>
                                <Button
                                    variant="outline"
                                    size="sm"
                                    className="rounded-lg text-red-600"
                                    onClick={() => setDeleting(range)}
                                >
                                    Xoá
                                </Button>
                            </div>
                        </div>
                    ))}
                </div>
            )}

            <div className="p-5 bg-slate-50 rounded-2xl space-y-4">
                <h4 className="font-bold text-sm">
                    {editing ? `Sửa: ${editing.label}` : "Thêm đợt cao điểm"}
                </h4>
                <div className="grid grid-cols-2 gap-3">
                    <div>
                        <Label htmlFor="special-label">Tên đợt</Label>
                        <Input
                            id="special-label"
                            value={label}
                            onChange={(event) => setLabel(event.target.value)}
                            placeholder="Tết Nguyên đán"
                            className="mt-1.5"
                        />
                    </div>
                    <div>
                        <Label htmlFor="special-uplift">% phụ thu</Label>
                        <Input
                            id="special-uplift"
                            type="number"
                            min={0}
                            max={500}
                            value={upliftPct}
                            onChange={(event) => setUpliftPct(event.target.value)}
                            className="mt-1.5 w-24"
                        />
                    </div>
                    <div>
                        <Label htmlFor="special-from">Từ ngày</Label>
                        <Input
                            id="special-from"
                            type="date"
                            value={from}
                            onChange={(event) => setFrom(event.target.value)}
                            className="mt-1.5"
                        />
                    </div>
                    <div>
                        <Label htmlFor="special-to">Đến ngày</Label>
                        <Input
                            id="special-to"
                            type="date"
                            min={from || undefined}
                            value={to}
                            onChange={(event) => setTo(event.target.value)}
                            className="mt-1.5"
                        />
                    </div>
                </div>
                <div className="flex gap-2">
                    <Button
                        onClick={handleSave}
                        className="bg-brand-primary text-white rounded-xl"
                    >
                        {editing ? "Cập nhật" : "Thêm"}
                    </Button>
                    {editing && (
                        <Button variant="outline" className="rounded-xl" onClick={resetForm}>
                            Huỷ sửa
                        </Button>
                    )}
                </div>
            </div>

            {clashes && pending && (
                <div className="p-5 border border-amber-300 bg-amber-50 rounded-2xl space-y-3">
                    <p className="text-sm font-semibold">
                        {clashes.length} ngày đã khai sẽ bị ghi đè
                    </p>
                    <ul className="text-xs text-brand-muted space-y-0.5">
                        {clashes.slice(0, MAX_LISTED_CLASHES).map((clash) => (
                            <li key={clash.date}>
                                {clash.date} — {clash.label} +{clash.uplift_pct}% → {pending.label} +
                                {pending.upliftPct}%
                            </li>
                        ))}
                        {clashes.length > MAX_LISTED_CLASHES && (
                            <li>…và {clashes.length - MAX_LISTED_CLASHES} ngày nữa</li>
                        )}
                    </ul>
                    <div className="flex gap-2">
                        <Button
                            className="bg-brand-primary text-white rounded-xl"
                            onClick={() => {
                                const request = pending;
                                setClashes(null);
                                setPending(null);
                                void write(request);
                            }}
                        >
                            Tiếp tục
                        </Button>
                        <Button
                            variant="outline"
                            className="rounded-xl"
                            onClick={() => {
                                setClashes(null);
                                setPending(null);
                            }}
                        >
                            Huỷ
                        </Button>
                    </div>
                </div>
            )}

            {deleting && (
                <div className="p-5 border border-red-300 bg-red-50 rounded-2xl space-y-3">
                    <p className="text-sm font-semibold">
                        Xoá &quot;{deleting.label}&quot; ({deleting.days} ngày)?
                    </p>
                    <div className="flex gap-2">
                        <Button
                            className="bg-red-600 text-white rounded-xl"
                            onClick={() => void handleDelete(deleting)}
                        >
                            Xoá đợt này
                        </Button>
                        <Button
                            variant="outline"
                            className="rounded-xl"
                            onClick={() => setDeleting(null)}
                        >
                            Giữ lại
                        </Button>
                    </div>
                </div>
            )}
        </div>
    );
}
