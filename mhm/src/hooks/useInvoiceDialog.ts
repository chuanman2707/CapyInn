import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";

import type { InvoiceData } from "@/components/InvoicePDF";
import { invokeWriteCommand } from "@/lib/invokeCommand";

export function useInvoiceDialog() {
    const [invoiceOpen, setInvoiceOpen] = useState(false);
    const [invoiceData, setInvoiceData] = useState<InvoiceData | null>(null);
    const [invoiceLoading, setInvoiceLoading] = useState(false);

    const openInvoice = async (bookingId: string) => {
        setInvoiceLoading(true);
        try {
            const data = await invokeWriteCommand<InvoiceData>("generate_invoice", { bookingId });
            setInvoiceData(data);
            setInvoiceOpen(true);
        } catch (err) {
            toast.error("Lỗi tạo invoice: " + err);
        } finally {
            setInvoiceLoading(false);
        }
    };

    /**
     * Xem hóa đơn của một booking đã khép lại: đọc bản đã phát hành trước,
     * chỉ sinh mới khi chưa có. Tránh ghi ledger cho một thao tác chỉ để xem.
     */
    const viewInvoice = async (bookingId: string) => {
        setInvoiceLoading(true);
        try {
            const existing = await invoke<InvoiceData | null>("get_invoice", { bookingId });
            const data = existing ?? await invokeWriteCommand<InvoiceData>("generate_invoice", { bookingId });
            setInvoiceData(data);
            setInvoiceOpen(true);
        } catch (err) {
            toast.error("Lỗi tạo invoice: " + err);
        } finally {
            setInvoiceLoading(false);
        }
    };

    const closeInvoice = () => {
        setInvoiceOpen(false);
    };

    return {
        invoiceOpen,
        invoiceData,
        invoiceLoading,
        openInvoice,
        viewInvoice,
        closeInvoice,
    };
}
