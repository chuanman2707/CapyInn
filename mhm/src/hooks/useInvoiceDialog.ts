import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";

import type { InvoiceData } from "@/components/InvoicePDF";

export function useInvoiceDialog() {
    const [invoiceOpen, setInvoiceOpen] = useState(false);
    const [invoiceData, setInvoiceData] = useState<InvoiceData | null>(null);
    const [invoiceLoading, setInvoiceLoading] = useState(false);

    const openInvoice = async (bookingId: string) => {
        setInvoiceLoading(true);
        try {
            const data = await invoke<InvoiceData>("generate_invoice", { bookingId });
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
        closeInvoice,
    };
}
