import type { ButtonHTMLAttributes, InputHTMLAttributes, ReactNode } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const invokeWriteCommand = vi.hoisted(() => vi.fn());
const createCorrelationId = vi.hoisted(() => vi.fn());
const toastSuccess = vi.hoisted(() => vi.fn());
const fetchRooms = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke,
}));

vi.mock("@/lib/invokeCommand", () => ({
  invokeWriteCommand,
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
        status: "booked",
      },
    ],
    fetchRooms,
  }),
}));

vi.mock("@/components/ui/input", () => ({
  Input: (props: InputHTMLAttributes<HTMLInputElement>) => <input {...props} />,
}));

vi.mock("@/components/ui/badge", () => ({
  Badge: ({ children }: { children: ReactNode }) => <span>{children}</span>,
}));

vi.mock("@/components/ui/button", () => ({
  Button: ({
    children,
    ...props
  }: ButtonHTMLAttributes<HTMLButtonElement>) => <button {...props}>{children}</button>,
}));

vi.mock("@/components/ReservationSheet", () => ({
  default: () => null,
}));

vi.mock("@/components/RoomDrawer", () => ({
  default: () => null,
}));

vi.mock("sonner", () => ({
  toast: {
    success: toastSuccess,
    error: vi.fn(),
  },
}));

import Reservations from "./Reservations";

function formatLocalDate(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function bookedReservation() {
  const today = new Date();
  const tomorrow = new Date(today);
  tomorrow.setDate(today.getDate() + 1);

  return {
    id: "B101",
    room_id: "R101",
    guest_name: "Nguyen Van A",
    guest_phone: "0900000000",
    check_in_at: formatLocalDate(today),
    expected_checkout: formatLocalDate(tomorrow),
    scheduled_checkin: formatLocalDate(today),
    scheduled_checkout: formatLocalDate(tomorrow),
    nights: 1,
    total_price: 500000,
    paid_amount: 50000,
    status: "booked",
    source: "phone",
    deposit_amount: 50000,
  };
}

async function openBookedReservationActions(user: ReturnType<typeof userEvent.setup>) {
  render(<Reservations />);

  await waitFor(() => {
    expect(screen.getAllByText("Nguyen Van A").length).toBeGreaterThan(0);
  });

  await user.click(screen.getAllByText("Nguyen Van A")[0]);
}

describe("Reservations", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_all_bookings") return [bookedReservation()];
      return undefined;
    });
    invokeWriteCommand.mockResolvedValue(undefined);
    createCorrelationId.mockReturnValue("COR-5E6F7A8B");
  });

  it("confirms reservations through invokeWriteCommand with correlation id", async () => {
    const user = userEvent.setup();
    await openBookedReservationActions(user);

    await user.click(screen.getByRole("button", { name: /check-in/i }));

    expect(createCorrelationId).toHaveBeenCalledTimes(1);
    await waitFor(() => {
      expect(invokeWriteCommand).toHaveBeenCalledWith(
        "confirm_reservation",
        { bookingId: "B101" },
        { correlationId: "COR-5E6F7A8B" },
      );
    });
    expect(toastSuccess).toHaveBeenCalledWith("Check-in reservation thành công!");
  });

  it("cancels reservations through invokeWriteCommand with correlation id", async () => {
    const user = userEvent.setup();
    await openBookedReservationActions(user);

    await user.click(screen.getByRole("button", { name: /hủy/i }));

    expect(createCorrelationId).toHaveBeenCalledTimes(1);
    await waitFor(() => {
      expect(invokeWriteCommand).toHaveBeenCalledWith(
        "cancel_reservation",
        { bookingId: "B101" },
        { correlationId: "COR-5E6F7A8B" },
      );
    });
    expect(toastSuccess).toHaveBeenCalledWith("Đã hủy reservation. Tiền cọc được giữ lại.");
  });
});
