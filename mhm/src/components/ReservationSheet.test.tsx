import type { ButtonHTMLAttributes, HTMLAttributes, ReactNode } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  createAppErrorException,
  formatAppError,
  type AppError,
} from "@/lib/appError";

const invoke = vi.hoisted(() => vi.fn());
const invokeCommand = vi.hoisted(() => vi.fn());
const createIdempotencyKey = vi.hoisted(() => vi.fn());
const createCorrelationId = vi.hoisted(() => vi.fn());
const toastError = vi.hoisted(() => vi.fn());
const toastSuccess = vi.hoisted(() => vi.fn());
const fetchRooms = vi.hoisted(() => vi.fn());
const openInvoice = vi.hoisted(() => vi.fn());
const closeInvoice = vi.hoisted(() => vi.fn());
const resetAvailability = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke,
}));

vi.mock("@/lib/invokeCommand", () => ({
  createIdempotencyKey,
  invokeCommand,
}));

vi.mock("@/lib/correlationId", () => ({
  createCorrelationId,
}));

vi.mock("@/stores/useHotelStore", () => ({
  useHotelStore: () => ({
    rooms: [
      {
        id: "R101",
        name: "R101",
        type: "standard",
        room_type: "standard",
        floor: 1,
        has_balcony: false,
        base_price: 500000,
        max_guests: 2,
        extra_person_fee: 0,
        status: "vacant",
      },
    ],
    fetchRooms,
  }),
}));

vi.mock("@/hooks/useAvailability", () => ({
  useAvailability: () => ({
    availability: null,
    loading: false,
    reset: resetAvailability,
  }),
}));

vi.mock("@/hooks/useInvoiceDialog", () => ({
  useInvoiceDialog: () => ({
    invoiceOpen: false,
    invoiceData: null,
    invoiceLoading: false,
    openInvoice,
    closeInvoice,
  }),
}));

vi.mock("@/components/ui/sheet", () => ({
  Sheet: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  SheetContent: ({ children, ...props }: HTMLAttributes<HTMLDivElement>) => (
    <div {...props}>{children}</div>
  ),
  SheetHeader: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  SheetTitle: ({ children }: { children: ReactNode }) => <h2>{children}</h2>,
}));

vi.mock("@/components/ui/button", () => ({
  Button: ({
    children,
    ...props
  }: ButtonHTMLAttributes<HTMLButtonElement>) => <button {...props}>{children}</button>,
}));

vi.mock("./InvoiceDialog", () => ({
  default: () => null,
}));

vi.mock("sonner", () => ({
  toast: {
    error: toastError,
    success: toastSuccess,
  },
}));

import ReservationSheet from "./ReservationSheet";

const createReservationError: AppError = {
  code: "BOOKING_INVALID_STATE",
  message: "Room R101 is booked on 2026-04-26. Cannot create reservation.",
  kind: "user",
  support_id: null,
};

describe("ReservationSheet", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invoke.mockResolvedValue(undefined);
    invokeCommand.mockResolvedValue(undefined);
    createIdempotencyKey.mockReturnValue("create_reservation:IDEM-1");
    createCorrelationId.mockReturnValue("COR-5E6F7A8B");
  });

  it("uses invokeCommand with scrubbed monitoring context for the create flow", async () => {
    const user = userEvent.setup();
    render(<ReservationSheet open onOpenChange={vi.fn()} />);

    await waitFor(() => {
      expect(fetchRooms).toHaveBeenCalledTimes(1);
    });

    const [roomSelect, sourceSelect] = screen.getAllByRole("combobox");
    const [nightsInput, depositInput] = screen.getAllByRole("spinbutton");

    fireEvent.change(roomSelect, {
      target: { value: "R101" },
    });
    await user.type(screen.getByPlaceholderText("Họ và tên *"), "Nguyen Van A");
    fireEvent.change(nightsInput, {
      target: { value: "2" },
    });
    fireEvent.change(depositInput, {
      target: { value: "250000" },
    });
    fireEvent.change(sourceSelect, {
      target: { value: "zalo" },
    });
    await user.type(
      screen.getByPlaceholderText("Ghi chú thêm..."),
      "Khách thích tầng cao",
    );

    await user.click(screen.getByRole("button", { name: /đặt phòng/i }));

    expect(createCorrelationId).toHaveBeenCalledTimes(1);
    await waitFor(() => {
      expect(invokeCommand).toHaveBeenCalledWith(
        "create_reservation",
        {
          req: {
            room_id: "R101",
            guest_name: "Nguyen Van A",
            guest_phone: null,
            guest_doc_number: null,
            check_in_date: expect.any(String),
            check_out_date: expect.any(String),
            nights: 2,
            deposit_amount: 250000,
            source: "zalo",
            notes: "Khách thích tầng cao",
          },
          idempotencyKey: "create_reservation:IDEM-1",
        },
        {
          correlationId: "COR-5E6F7A8B",
          monitoringContext: {
            nights: 2,
            deposit_present: true,
            source: "zalo",
            notes_present: true,
          },
        },
      );
    });
  });

  it("formats create flow failures with formatAppError", async () => {
    const user = userEvent.setup();
    const error = createAppErrorException(createReservationError, undefined, {
      correlation_id: "COR-5E6F7A8B",
    });
    invokeCommand.mockRejectedValue(error);

    render(<ReservationSheet open onOpenChange={vi.fn()} />);

    await waitFor(() => {
      expect(fetchRooms).toHaveBeenCalledTimes(1);
    });

    const [roomSelect] = screen.getAllByRole("combobox");

    fireEvent.change(roomSelect, {
      target: { value: "R101" },
    });
    await user.type(screen.getByPlaceholderText("Họ và tên *"), "Nguyen Van A");
    await user.click(screen.getByRole("button", { name: /đặt phòng/i }));

    await waitFor(() => {
      expect(invokeCommand).toHaveBeenCalledWith(
        "create_reservation",
        expect.objectContaining({
          req: expect.objectContaining({
            room_id: "R101",
            guest_name: "Nguyen Van A",
          }),
          idempotencyKey: "create_reservation:IDEM-1",
        }),
        {
          correlationId: "COR-5E6F7A8B",
          monitoringContext: {
            nights: 1,
            deposit_present: false,
            source: "phone",
            notes_present: false,
          },
        },
      );
    });
    expect(toastError).toHaveBeenCalledWith(formatAppError(error));
  });
});
