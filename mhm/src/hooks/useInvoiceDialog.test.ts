import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

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
      hotel_name: "CapyInn",
      room_name: "101",
      guest_name: "Nguyen Van A",
      check_in: "2026-01-01",
      check_out: "2026-01-02",
      nights: 1,
      room_total: 500000,
      service_total: 0,
      discount: 0,
      total: 500000,
      paid: 500000,
      balance: 0,
      services: [],
    };
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
