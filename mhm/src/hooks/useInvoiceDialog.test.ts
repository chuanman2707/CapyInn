import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { InvoiceData } from "@/components/InvoicePDF";

const invokeWriteCommand = vi.hoisted(() => vi.fn());
const toastError = vi.hoisted(() => vi.fn());
const invoke = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invokeCommand", () => ({
  invokeWriteCommand,
}));

vi.mock("sonner", () => ({
  toast: {
    error: toastError,
  },
}));

vi.mock("@tauri-apps/api/core", () => ({
  invoke,
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
      settlement_note: null,
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

  const existingInvoice = {
    id: "invoice-9",
    invoice_number: "INV-20260726-009",
    booking_id: "booking-9",
    hotel_name: "CapyInn",
    hotel_address: "",
    hotel_phone: "",
    room_name: "1B",
    room_type: "standard",
    guest_name: "Hoseo Kim",
    guest_phone: null,
    check_in: "2026-07-23",
    check_out: "2026-07-25",
    nights: 2,
    pricing_breakdown: [{ label: "2 night(s) x 600000d", amount: 1200000 }],
    subtotal: 1200000,
    deposit_amount: 0,
    total: 1200000,
    balance_due: 0,
    policy_text: null,
    notes: null,
    settlement_note: null,
    status: "issued",
    created_at: "2026-07-25T09:12:00+07:00",
  } satisfies InvoiceData;

  it("opens an existing invoice without touching the write command", async () => {
    invoke.mockResolvedValueOnce(existingInvoice);
    const { result } = renderHook(() => useInvoiceDialog());

    await act(async () => {
      await result.current.viewInvoice("booking-9");
    });

    expect(invoke).toHaveBeenCalledWith("get_invoice", { bookingId: "booking-9" });
    expect(invokeWriteCommand).not.toHaveBeenCalled();
    expect(result.current.invoiceOpen).toBe(true);
    expect(result.current.invoiceData).toBe(existingInvoice);
    expect(result.current.invoiceLoading).toBe(false);
  });

  it("falls back to generating when no invoice exists yet", async () => {
    invoke.mockResolvedValueOnce(null);
    invokeWriteCommand.mockResolvedValueOnce(existingInvoice);
    const { result } = renderHook(() => useInvoiceDialog());

    await act(async () => {
      await result.current.viewInvoice("booking-9");
    });

    expect(invoke).toHaveBeenCalledWith("get_invoice", { bookingId: "booking-9" });
    expect(invokeWriteCommand).toHaveBeenCalledWith("generate_invoice", {
      bookingId: "booking-9",
    });
    expect(result.current.invoiceOpen).toBe(true);
    expect(result.current.invoiceData).toBe(existingInvoice);
  });

  it("shows an error when reading the invoice fails", async () => {
    invoke.mockRejectedValueOnce(new Error("db down"));
    const { result } = renderHook(() => useInvoiceDialog());

    await act(async () => {
      await result.current.viewInvoice("booking-9");
    });

    expect(toastError).toHaveBeenCalledWith("Lỗi tạo invoice: Error: db down");
    expect(result.current.invoiceOpen).toBe(false);
    expect(result.current.invoiceLoading).toBe(false);
  });
});
