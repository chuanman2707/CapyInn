import { useEffect, useState } from "react";
import { useHotelStore } from "../stores/useHotelStore";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import EmptyState from "@/components/shared/EmptyState";
import SlideDrawer from "@/components/shared/SlideDrawer";
import { formatAppError } from "@/lib/appError";
import { fmtMoney, fmtDateShort } from "@/lib/format";
import { toast } from "sonner";
import { Users, Plus, Trash2, FileText, LogOut, ChevronRight } from "lucide-react";
import type { GroupDetailResponse, GroupService, BookingWithGuest, GroupInvoiceData } from "@/types";
import InvoiceDialog from "@/components/InvoiceDialog";

const STATUS_COLORS: Record<string, string> = {
    active: "bg-emerald-50 text-emerald-700",
    partial_checkout: "bg-orange-50 text-orange-600",
    completed: "bg-slate-100 text-slate-500",
};

const STATUS_LABELS: Record<string, string> = {
    active: "Active",
    partial_checkout: "Partial",
    completed: "Completed",
};

function getLegacyErrorMessage(error: unknown): string {
    if (typeof error === "string") {
        return error;
    }
    if (error instanceof Error) {
        return error.message;
    }
    return "Có lỗi xảy ra";
}

export default function GroupManagement() {
    const { groups, fetchGroups, getGroupDetail, groupCheckout, addGroupService, removeGroupService, generateGroupInvoice } = useHotelStore();
    const [filter, setFilter] = useState<string>("");
    const [selectedGroupId, setSelectedGroupId] = useState<string | null>(null);
    const [detail, setDetail] = useState<GroupDetailResponse | null>(null);
    const [loadingDetail, setLoadingDetail] = useState(false);

    // Service form
    const [svcName, setSvcName] = useState("");
    const [svcQty, setSvcQty] = useState(1);
    const [svcPrice, setSvcPrice] = useState(0);
    const [svcNote, setSvcNote] = useState("");

    // Checkout selection
    const [checkoutIds, setCheckoutIds] = useState<string[]>([]);

    // Invoice dialog
    const [invoiceData, setInvoiceData] = useState<GroupInvoiceData | null>(null);
    const [invoiceOpen, setInvoiceOpen] = useState(false);

    useEffect(() => {
        fetchGroups(filter || undefined);
    }, [filter]);

    const openDetail = async (groupId: string) => {
        setSelectedGroupId(groupId);
        setLoadingDetail(true);
        try {
            const d = await getGroupDetail(groupId);
            setDetail(d);
        } catch (err) {
            toast.error(formatAppError(err));
        }
        setLoadingDetail(false);
    };

    const refreshDetail = async () => {
        if (selectedGroupId) {
            const d = await getGroupDetail(selectedGroupId);
            setDetail(d);
            fetchGroups(filter || undefined);
        }
    };

    const handleAddService = async () => {
        if (!selectedGroupId || !svcName.trim()) return;
        try {
            await addGroupService({
                group_id: selectedGroupId,
                name: svcName,
                quantity: svcQty,
                unit_price: svcPrice,
                note: svcNote || undefined,
            });
            toast.success("Đã thêm dịch vụ");
            setSvcName("");
            setSvcQty(1);
            setSvcPrice(0);
            setSvcNote("");
            await refreshDetail();
        } catch (err) {
            toast.error(getLegacyErrorMessage(err));
        }
    };

    const handleRemoveService = async (svcId: string) => {
        try {
            await removeGroupService(svcId);
            toast.success("Đã xóa dịch vụ");
            await refreshDetail();
        } catch (err) {
            toast.error(getLegacyErrorMessage(err));
        }
    };

    const handleCheckout = async () => {
        if (!selectedGroupId || checkoutIds.length === 0) return;
        try {
            await groupCheckout({
                group_id: selectedGroupId,
                booking_ids: checkoutIds,
            });
            toast.success(`Checkout ${checkoutIds.length} phòng`);
            setCheckoutIds([]);
            await refreshDetail();
        } catch (err) {
            toast.error(formatAppError(err));
        }
    };

    const handleInvoice = async () => {
        if (!selectedGroupId) return;
        try {
            const invoice = await generateGroupInvoice(selectedGroupId);
            setInvoiceData(invoice);
            setInvoiceOpen(true);
        } catch (err) {
            toast.error(getLegacyErrorMessage(err));
        }
    };

    const toggleCheckout = (bookingId: string) => {
        setCheckoutIds((prev) =>
            prev.includes(bookingId) ? prev.filter((id) => id !== bookingId) : [...prev, bookingId]
        );
    };

    return (
        <div className="space-y-6">
            {/* Header */}
            <div className="flex items-center justify-between">
                <h2 className="text-lg font-bold text-brand-text flex items-center gap-2">
                    <Users size={20} className="text-brand-primary" />
                    Quản lý đoàn
                </h2>
                <div className="flex gap-2">
                    {["", "active", "partial_checkout", "completed"].map((f) => (
                        <Button
                            key={f}
                            variant={filter === f ? "default" : "outline"}
                            size="sm"
                            onClick={() => setFilter(f)}
                            className="rounded-lg text-xs"
                        >
                            {f === "" ? "Tất cả" : STATUS_LABELS[f] || f}
                        </Button>
                    ))}
                </div>
            </div>

            {/* Group Table */}
            <div className="bg-white rounded-2xl shadow-soft overflow-hidden">
                <Table>
                    <TableHeader>
                        <TableRow className="border-b border-slate-100 hover:bg-transparent">
                            <TableHead className="text-xs uppercase text-slate-400 font-semibold">Tên đoàn</TableHead>
                            <TableHead className="text-xs uppercase text-slate-400 font-semibold">Trưởng đoàn</TableHead>
                            <TableHead className="text-xs uppercase text-slate-400 font-semibold">Phòng</TableHead>
                            <TableHead className="text-xs uppercase text-slate-400 font-semibold">Status</TableHead>
                            <TableHead className="text-xs uppercase text-slate-400 font-semibold">Ngày tạo</TableHead>
                            <TableHead className="text-xs uppercase text-slate-400 font-semibold text-right">Actions</TableHead>
                        </TableRow>
                    </TableHeader>
                    <TableBody>
                        {groups.map((g) => (
                            <TableRow
                                key={g.id}
                                className="border-b border-slate-50 hover:bg-slate-50/50 cursor-pointer transition-colors"
                                onClick={() => openDetail(g.id)}
                            >
                                <TableCell className="font-semibold text-brand-text py-3">{g.group_name}</TableCell>
                                <TableCell className="py-3">{g.organizer_name}</TableCell>
                                <TableCell className="py-3 font-medium">{g.total_rooms}</TableCell>
                                <TableCell className="py-3">
                                    <Badge className={`border-0 rounded-md py-0.5 px-2 font-semibold text-[11px] ${STATUS_COLORS[g.status] || ""}`}>
                                        {STATUS_LABELS[g.status] || g.status}
                                    </Badge>
                                </TableCell>
                                <TableCell className="text-brand-muted py-3">{fmtDateShort(g.created_at)}</TableCell>
                                <TableCell className="text-right py-3">
                                    <Button variant="ghost" size="sm" className="rounded-lg">
                                        <ChevronRight size={16} />
                                    </Button>
                                </TableCell>
                            </TableRow>
                        ))}
                        {groups.length === 0 && (
                            <TableRow>
                                <TableCell colSpan={6} className="text-center py-8">
                                    <EmptyState message="Chưa có đoàn nào" />
                                </TableCell>
                            </TableRow>
                        )}
                    </TableBody>
                </Table>
            </div>

            {/* Detail Drawer */}
            <SlideDrawer
                open={!!selectedGroupId}
                onClose={() => { setSelectedGroupId(null); setDetail(null); setCheckoutIds([]); }}
                title={detail?.group.group_name || "Chi tiết đoàn"}
                width="w-[560px]"
            >
                {loadingDetail ? (
                    <div className="flex items-center justify-center h-40 text-brand-muted">Đang tải...</div>
                ) : detail ? (
                    <div className="space-y-6">
                        {/* Group Info */}
                        <div className="bg-slate-50 rounded-xl p-4 space-y-2">
                            <div className="flex justify-between text-sm">
                                <span className="text-brand-muted">Trưởng đoàn</span>
                                <span className="font-semibold">{detail.group.organizer_name}</span>
                            </div>
                            {detail.group.organizer_phone && (
                                <div className="flex justify-between text-sm">
                                    <span className="text-brand-muted">SĐT</span>
                                    <span>{detail.group.organizer_phone}</span>
                                </div>
                            )}
                            <div className="flex justify-between text-sm">
                                <span className="text-brand-muted">Status</span>
                                <Badge className={`border-0 rounded-md py-0.5 px-2 font-semibold text-[11px] ${STATUS_COLORS[detail.group.status]}`}>
                                    {STATUS_LABELS[detail.group.status]}
                                </Badge>
                            </div>
                        </div>

                        {/* Room List */}
                        <div>
                            <h3 className="text-sm font-bold text-brand-text mb-3">
                                Danh sách phòng ({detail.bookings.length})
                            </h3>
                            <div className="space-y-2">
                                {detail.bookings.map((b: BookingWithGuest) => (
                                    <div
                                        key={b.id}
                                        className={`flex items-center gap-3 p-3 rounded-xl border transition-all ${checkoutIds.includes(b.id) ? "border-red-300 bg-red-50/50" : "border-slate-100"
                                            }`}
                                    >
                                        {b.status === "active" && (
                                            <input
                                                type="checkbox"
                                                checked={checkoutIds.includes(b.id)}
                                                onChange={() => toggleCheckout(b.id)}
                                                className="w-4 h-4 rounded accent-red-500"
                                            />
                                        )}
                                        <div className="flex-1 min-w-0">
                                            <p className="font-semibold text-sm">{b.room_name}</p>
                                            <p className="text-xs text-brand-muted">{b.guest_name}</p>
                                        </div>
                                        <div className="text-right">
                                            <p className="text-sm font-bold">{fmtMoney(b.total_price)}</p>
                                            <Badge className={`border-0 rounded-md py-0 px-1.5 text-[10px] font-semibold ${b.status === "active" ? "bg-emerald-50 text-emerald-600" : "bg-slate-100 text-slate-500"
                                                }`}>
                                                {b.status === "active" ? "Active" : "Checked out"}
                                            </Badge>
                                        </div>
                                    </div>
                                ))}
                            </div>

                            {checkoutIds.length > 0 && detail.group.status !== "completed" && (
                                <Button
                                    onClick={handleCheckout}
                                    className="w-full mt-3 rounded-xl bg-red-500 hover:bg-red-600 text-white"
                                >
                                    <LogOut size={16} className="mr-2" />
                                    Checkout {checkoutIds.length} phòng
                                </Button>
                            )}
                        </div>

                        {/* Services */}
                        <div>
                            <h3 className="text-sm font-bold text-brand-text mb-3">Dịch vụ kèm</h3>
                            {detail.services.length > 0 && (
                                <div className="space-y-2 mb-3">
                                    {detail.services.map((svc: GroupService) => (
                                        <div key={svc.id} className="flex items-center justify-between p-2 rounded-lg border border-slate-100">
                                            <div>
                                                <p className="text-sm font-medium">{svc.name}</p>
                                                <p className="text-xs text-brand-muted">{svc.quantity} × {fmtMoney(svc.unit_price)}</p>
                                            </div>
                                            <div className="flex items-center gap-2">
                                                <span className="text-sm font-bold">{fmtMoney(svc.total_price)}</span>
                                                <button onClick={() => handleRemoveService(svc.id)} className="text-red-400 hover:text-red-600 cursor-pointer">
                                                    <Trash2 size={14} />
                                                </button>
                                            </div>
                                        </div>
                                    ))}
                                </div>
                            )}

                            {detail.group.status !== "completed" && (
                                <div className="bg-slate-50 rounded-xl p-3 space-y-2">
                                    <div className="grid grid-cols-2 gap-2">
                                        <Input value={svcName} onChange={(e) => setSvcName(e.target.value)} placeholder="Tên dịch vụ" className="text-sm" />
                                        <Input type="number" value={svcPrice} onChange={(e) => setSvcPrice(+e.target.value)} placeholder="Đơn giá" className="text-sm" />
                                    </div>
                                    <div className="grid grid-cols-2 gap-2">
                                        <Input type="number" value={svcQty} onChange={(e) => setSvcQty(+e.target.value)} min={1} placeholder="SL" className="text-sm" />
                                        <Input value={svcNote} onChange={(e) => setSvcNote(e.target.value)} placeholder="Ghi chú" className="text-sm" />
                                    </div>
                                    <Button onClick={handleAddService} className="w-full rounded-lg text-sm" disabled={!svcName.trim() || svcPrice <= 0}>
                                        <Plus size={14} className="mr-1" /> Thêm dịch vụ
                                    </Button>
                                </div>
                            )}
                        </div>

                        {/* Totals */}
                        <div className="bg-brand-primary/5 rounded-xl p-4 space-y-2">
                            <div className="flex justify-between text-sm">
                                <span>Tổng phòng</span>
                                <span className="font-semibold">{fmtMoney(detail.total_room_cost)}</span>
                            </div>
                            <div className="flex justify-between text-sm">
                                <span>Tổng dịch vụ</span>
                                <span className="font-semibold">{fmtMoney(detail.total_service_cost)}</span>
                            </div>
                            <hr className="border-slate-200" />
                            <div className="flex justify-between text-base font-bold">
                                <span>Tổng cộng</span>
                                <span className="text-brand-primary">{fmtMoney(detail.grand_total)}</span>
                            </div>
                            <div className="flex justify-between text-sm">
                                <span>Đã thanh toán</span>
                                <span className="font-semibold text-emerald-600">{fmtMoney(detail.paid_amount)}</span>
                            </div>
                            <div className="flex justify-between text-sm">
                                <span>Còn lại</span>
                                <span className="font-bold text-red-500">
                                    {fmtMoney(detail.grand_total - detail.paid_amount)}
                                </span>
                            </div>
                        </div>

                        {/* Invoice button */}
                        <Button onClick={handleInvoice} variant="outline" className="w-full rounded-xl">
                            <FileText size={16} className="mr-2" /> Xuất hóa đơn đoàn
                        </Button>
                    </div>
                ) : null}
            </SlideDrawer>

            {/* Group Invoice Dialog */}
            <InvoiceDialog
                open={invoiceOpen}
                onOpenChange={setInvoiceOpen}
                data={null}
                groupData={invoiceData}
            />
        </div>
    );
}
