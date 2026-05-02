import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { InvoiceData } from "@/components/InvoicePDF";

const invokeWriteCommand = vi.hoisted(() => vi.fn());
const toastError = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invokeCommand", () => ({
  invokeWriteCommand,
}));

vi.mock("sonner", () => ({
  toast: {
    error: toastError,
  },
}));

import { useInvoiceDialog } from "./useInvoiceDialog";

describe("useInvoiceDialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("generates invoice through invokeWriteCommand and opens the dialog", async () => {
    const invoice = {
      id: "invoice-1",
      invoice_number: "INV-20260501-001",
      booking_id: "booking-1",
      hotel_name: "CapyInn",
      hotel_address: "",
      hotel_phone: "",
      room_name: "101",
      room_type: "standard",
      guest_name: "Nguyen Van A",
      guest_phone: null,
      check_in: "2026-01-01",
      check_out: "2026-01-02",
      nights: 1,
      pricing_breakdown: [{ label: "1 night(s) x 500000d", amount: 500000 }],
      subtotal: 500000,
      deposit_amount: 0,
      total: 500000,
      balance_due: 0,
      policy_text: null,
      notes: null,
      status: "issued",
      created_at: "2026-05-01T09:00:00+07:00",
    } satisfies InvoiceData;
    invokeWriteCommand.mockResolvedValueOnce(invoice);
    const { result } = renderHook(() => useInvoiceDialog());

    await act(async () => {
      await result.current.openInvoice("booking-1");
    });

    expect(invokeWriteCommand).toHaveBeenCalledWith("generate_invoice", {
      bookingId: "booking-1",
    });
    expect(result.current.invoiceOpen).toBe(true);
    expect(result.current.invoiceData).toBe(invoice);
    expect(result.current.invoiceLoading).toBe(false);
  });

  it("shows an error and turns loading off when invoice generation fails", async () => {
    invokeWriteCommand.mockRejectedValueOnce(new Error("backend failed"));
    const { result } = renderHook(() => useInvoiceDialog());

    await act(async () => {
      await result.current.openInvoice("booking-1");
    });

    expect(toastError).toHaveBeenCalledWith(
      "Lỗi tạo invoice: Error: backend failed",
    );
    expect(result.current.invoiceOpen).toBe(false);
    expect(result.current.invoiceData).toBeNull();
    expect(result.current.invoiceLoading).toBe(false);
  });
});
