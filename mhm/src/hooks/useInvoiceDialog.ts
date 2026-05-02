import { useState } from "react";
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
