import { useState } from "react";
import { pdf } from "@react-pdf/renderer";
import InvoicePDF, { type InvoiceData } from "./InvoicePDF";
import GroupInvoice from "./GroupInvoice";
import { Sheet, SheetContent, SheetHeader, SheetTitle } from "@/components/ui/sheet";
import { Button } from "@/components/ui/button";
import { Download, Printer, FileText } from "lucide-react";
import { toast } from "sonner";
import type { GroupInvoiceData } from "@/types";

interface Props {
    open: boolean;
    onOpenChange: (v: boolean) => void;
    data: InvoiceData | null;
    groupData?: GroupInvoiceData | null;
}

export default function InvoiceDialog({ open, onOpenChange, data, groupData }: Props) {
    const [loading, setLoading] = useState(false);

    const isGroup = !!groupData;
    if (!data && !groupData) return null;

    const renderPdf = () =>
        isGroup
            ? <InvoicePDF groupData={groupData!} />
            : <InvoicePDF data={data!} />;

    const downloadFilename = isGroup
        ? `HOA-DON-DOAN-${groupData!.group.group_name.replace(/\s+/g, "-")}.pdf`
        : `${data!.invoice_number}.pdf`;

    const handleDownload = async () => {
        setLoading(true);
        try {
            const blob = await pdf(renderPdf()).toBlob();
            const url = URL.createObjectURL(blob);
            const a = document.createElement("a");
            a.href = url;
            a.download = downloadFilename;
            document.body.appendChild(a);
            a.click();
            document.body.removeChild(a);
            URL.revokeObjectURL(url);
            toast.success("PDF đã tải thành công");
        } catch (err: any) {
            console.error("PDF generation error:", err);
            toast.error("Lỗi tạo PDF: " + (err?.message || String(err)));
        } finally {
            setLoading(false);
        }
    };

    const handlePrint = async () => {
        setLoading(true);
        try {
            const blob = await pdf(renderPdf()).toBlob();
            const url = URL.createObjectURL(blob);
            const iframe = document.createElement("iframe");
            iframe.style.display = "none";
            iframe.src = url;
            document.body.appendChild(iframe);
            iframe.onload = () => {
                iframe.contentWindow?.print();
                setTimeout(() => {
                    document.body.removeChild(iframe);
                    URL.revokeObjectURL(url);
                }, 1000);
            };
        } catch (err) {
            console.error(err);
            toast.error("Lỗi in");
        } finally {
            setLoading(false);
        }
    };

    const fmtVnd = (n: number) => n.toLocaleString("en-US") + "d";
    const fmtDate = (iso: string) => {
        if (!iso) return "";
        const d = iso.slice(0, 10);
        const [y, m, day] = d.split("-");
        return `${day}/${m}/${y}`;
    };

    const sheetTitle = isGroup
        ? `Hóa đơn đoàn — ${groupData!.group.group_name}`
        : `Invoice ${data!.invoice_number}`;

    return (
        <Sheet open={open} onOpenChange={onOpenChange}>
            <SheetContent
                side="right"
                className="w-full sm:max-w-lg overflow-y-auto p-0"
            >
                <SheetHeader className="px-6 pt-6 pb-4 border-b border-border">
                    <SheetTitle className="flex items-center gap-2 text-lg">
                        <FileText className="w-5 h-5 text-blue-600" />
                        {sheetTitle}
                    </SheetTitle>
                </SheetHeader>

                <div className="p-6 space-y-6">
                    {/* Actions */}
                    <div className="flex gap-3">
                        <Button
                            onClick={handleDownload}
                            disabled={loading}
                            className="flex-1 gap-2"
                        >
                            <Download className="w-4 h-4" />
                            {loading ? "Đang tạo..." : "Tải PDF"}
                        </Button>
                        <Button
                            variant="outline"
                            onClick={handlePrint}
                            disabled={loading}
                            className="flex-1 gap-2"
                        >
                            <Printer className="w-4 h-4" />
                            In
                        </Button>
                    </div>

                    {/* Preview */}
                    {isGroup ? (
                        <GroupInvoice data={groupData!} />
                    ) : (
                        <div className="rounded-lg border border-border bg-card overflow-hidden">
                            {/* Hotel header */}
                            <div className="bg-[#1B2A4A] text-white p-4">
                                <div className="font-bold text-base">{data!.hotel_name}</div>
                                <div className="text-xs text-blue-200/80 mt-1">
                                    {data!.hotel_address}
                                    {data!.hotel_phone ? ` · ${data!.hotel_phone}` : ""}
                                </div>
                            </div>

                            {/* Title */}
                            <div className="px-4 py-3 border-b border-[#C5A55A] flex justify-between items-center">
                                <div className="font-bold text-[#1B2A4A] tracking-wide">
                                    BOOKING CONFIRMATION
                                </div>
                                <div className="text-right">
                                    <div className="text-sm font-semibold text-[#1B2A4A]">
                                        {data!.invoice_number}
                                    </div>
                                    <div className="text-xs text-muted-foreground">
                                        {fmtDate(data!.created_at)}
                                    </div>
                                </div>
                            </div>

                            {/* Guest + Room */}
                            <div className="grid grid-cols-2 gap-3 p-4">
                                <div className="bg-muted/50 rounded-md p-3">
                                    <div className="text-[10px] uppercase tracking-wider text-muted-foreground font-semibold mb-1">
                                        Guest
                                    </div>
                                    <div className="font-semibold text-sm">{data!.guest_name}</div>
                                    {data!.guest_phone && (
                                        <div className="text-xs text-muted-foreground">
                                            Phone: {data!.guest_phone}
                                        </div>
                                    )}
                                </div>
                                <div className="bg-muted/50 rounded-md p-3">
                                    <div className="text-[10px] uppercase tracking-wider text-muted-foreground font-semibold mb-1">
                                        Room
                                    </div>
                                    <div className="font-semibold text-sm">
                                        {data!.room_name} — {data!.room_type}
                                    </div>
                                    <div className="text-xs text-muted-foreground">
                                        {data!.nights} night(s)
                                    </div>
                                </div>
                            </div>

                            {/* Dates */}
                            <div className="px-4 space-y-1 text-sm">
                                <div className="flex justify-between py-1 border-b border-border/50">
                                    <span className="text-muted-foreground">Check-in</span>
                                    <span className="font-medium">{fmtDate(data!.check_in)}</span>
                                </div>
                                <div className="flex justify-between py-1 border-b border-border/50">
                                    <span className="text-muted-foreground">Check-out</span>
                                    <span className="font-medium">{fmtDate(data!.check_out)}</span>
                                </div>
                            </div>

                            {/* Pricing */}
                            <div className="p-4">
                                <div className="bg-[#2D4373] text-white text-xs font-bold tracking-wide px-3 py-2 rounded-t-md">
                                    PRICE BREAKDOWN
                                </div>
                                <div className="border border-t-0 border-border rounded-b-md divide-y divide-border/50">
                                    {data!.pricing_breakdown.map((line, i) => (
                                        <div key={i} className="flex justify-between px-3 py-2 text-sm">
                                            <span>{line.label}</span>
                                            <span className="font-semibold">{fmtVnd(line.amount)}</span>
                                        </div>
                                    ))}
                                </div>

                                {/* Totals */}
                                <div className="mt-3 space-y-1 text-sm">
                                    <div className="flex justify-between px-1">
                                        <span className="text-muted-foreground">Subtotal</span>
                                        <span className="font-semibold">{fmtVnd(data!.total)}</span>
                                    </div>
                                    {data!.deposit_amount > 0 && (
                                        <div className="flex justify-between px-1">
                                            <span className="text-muted-foreground">Deposit</span>
                                            <span className="font-semibold text-green-600">
                                                -{fmtVnd(data!.deposit_amount)}
                                            </span>
                                        </div>
                                    )}
                                    <div className="flex justify-between bg-[#1B2A4A] text-white rounded-md px-3 py-2 mt-2">
                                        <span className="font-bold">BALANCE DUE</span>
                                        <span className="font-bold text-[#C5A55A]">
                                            {fmtVnd(data!.balance_due)}
                                        </span>
                                    </div>
                                </div>
                            </div>

                            {/* Notes — for a booking that changed rooms this is
                                where the checkout settlement wording lives
                                (invoice_generation.rs moves it out of the
                                money lines so they still sum to Subtotal). */}
                            {(data!.notes ?? "").trim() !== "" && (
                                <div className="mx-4 mb-4 p-3 bg-muted/50 rounded-md border-l-2 border-[#2D4373]">
                                    <div className="text-[10px] uppercase tracking-wider text-[#1B2A4A] font-bold mb-2">
                                        Ghi chú
                                    </div>
                                    <div className="text-xs text-muted-foreground whitespace-pre-line leading-relaxed">
                                        {data!.notes}
                                    </div>
                                </div>
                            )}

                            {/* Policy */}
                            {data!.policy_text && (
                                <div className="mx-4 mb-4 p-3 bg-muted/50 rounded-md border-l-2 border-[#C5A55A]">
                                    <div className="text-[10px] uppercase tracking-wider text-[#1B2A4A] font-bold mb-2">
                                        Policies
                                    </div>
                                    <div className="text-xs text-muted-foreground whitespace-pre-line leading-relaxed">
                                        {data!.policy_text}
                                    </div>
                                </div>
                            )}
                        </div>
                    )}
                </div>
            </SheetContent>
        </Sheet>
    );
}
