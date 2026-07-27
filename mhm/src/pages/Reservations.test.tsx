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

vi.mock("@/components/InvoiceDialog", () => ({
  default: ({ open }: { open: boolean }) => (open ? <div>invoice-dialog</div> : null),
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

function dateOffsetFromToday(days: number): string {
  const d = new Date();
  d.setDate(d.getDate() + days);
  return formatLocalDate(d);
}

function bookingAt(overrides: Record<string, unknown>) {
  return {
    ...bookedReservation(),
    ...overrides,
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

  it("closes the popup when editing a reservation", async () => {
    const user = userEvent.setup();
    await openBookedReservationActions(user);

    expect(screen.getByText(/Reservation — Nguyen Van A/)).toBeTruthy();

    await user.click(screen.getByRole("button", { name: /chỉnh sửa/i }));

    expect(screen.queryByText(/Reservation — Nguyen Van A/)).toBeNull();
  });
});

describe("Reservations timeline geometry", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invokeWriteCommand.mockResolvedValue(undefined);
    createCorrelationId.mockReturnValue("COR-5E6F7A8B");
  });

  function mockBookings(bookings: unknown[]) {
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_all_bookings") return bookings;
      return undefined;
    });
  }

  it("places a bar half a day into the check-in cell and half a day into the check-out cell", async () => {
    // Lưới bắt đầu ở hôm nay - 3. Khách nhận hôm nay - 1, trả hôm nay + 3.
    // rawStart = (-1 - -3) + 0.5 = 2.5 -> left  = 200px
    // rawEnd   = ( 3 - -3) + 0.5 = 6.5 -> width = (6.5 - 2.5) * 80 = 320px
    mockBookings([
      bookingAt({
        id: "B-GEO",
        scheduled_checkin: dateOffsetFromToday(-1),
        scheduled_checkout: dateOffsetFromToday(3),
      }),
    ]);

    render(<Reservations />);

    const bar = await screen.findByTestId("booking-bar-B-GEO");
    expect(bar.style.left).toBe("200px");
    expect(bar.style.width).toBe("320px");
  });

  it("keeps a visible bar for a booking that checks in and out on the same day", async () => {
    mockBookings([
      bookingAt({
        id: "B-SAMEDAY",
        scheduled_checkin: dateOffsetFromToday(0),
        scheduled_checkout: dateOffsetFromToday(0),
        nights: 0,
      }),
    ]);

    render(<Reservations />);

    const bar = await screen.findByTestId("booking-bar-B-SAMEDAY");
    expect(bar.style.width).toBe("40px");
  });

  it("does not render bookings that ended before the visible window", async () => {
    mockBookings([
      bookingAt({
        id: "B-PAST",
        scheduled_checkin: dateOffsetFromToday(-20),
        scheduled_checkout: dateOffsetFromToday(-10),
        status: "checked_out",
      }),
    ]);

    render(<Reservations />);

    await waitFor(() => {
      expect(screen.getByText(/Không tìm thấy|Chưa có booking|Room R101/)).toBeTruthy();
    });
    expect(screen.queryByTestId("booking-bar-B-PAST")).toBeNull();
  });

  it("does not render bookings that start after the visible window", async () => {
    mockBookings([
      bookingAt({
        id: "B-FUTURE",
        scheduled_checkin: dateOffsetFromToday(40),
        scheduled_checkout: dateOffsetFromToday(42),
      }),
    ]);

    render(<Reservations />);

    await waitFor(() => {
      expect(screen.getByText("Room R101")).toBeTruthy();
    });
    expect(screen.queryByTestId("booking-bar-B-FUTURE")).toBeNull();
  });

  it("clips a bar at the left edge and squares off the clipped corner", async () => {
    mockBookings([
      bookingAt({
        id: "B-CLIPPED",
        scheduled_checkin: dateOffsetFromToday(-10),
        scheduled_checkout: dateOffsetFromToday(2),
      }),
    ]);

    render(<Reservations />);

    const bar = await screen.findByTestId("booking-bar-B-CLIPPED");
    expect(bar.style.left).toBe("0px");
    // rawEnd = (2 - -3) + 0.5 = 5.5 -> width = 5.5 * 80 = 440px
    expect(bar.style.width).toBe("440px");
    expect(bar.querySelector(".rounded-l-none")).not.toBeNull();
  });
});

describe("Reservations date navigation", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_all_bookings") return [];
      return undefined;
    });
    invokeWriteCommand.mockResolvedValue(undefined);
    createCorrelationId.mockReturnValue("COR-5E6F7A8B");
  });

  it("shows the month of the visible range and updates it when paging", async () => {
    const user = userEvent.setup();
    render(<Reservations />);

    const label = await screen.findByTestId("timeline-range-label");
    const initial = label.textContent ?? "";
    expect(initial).toMatch(/THÁNG/);

    // Nhảy 8 tuần về phía trước chắc chắn ra khỏi tháng hiện tại.
    const next = screen.getByRole("button", { name: /tuần sau/i });
    for (let i = 0; i < 8; i += 1) {
      await user.click(next);
    }

    expect(screen.getByTestId("timeline-range-label").textContent).not.toBe(initial);
  });

  it("hides the today button until the range moves, then resets the range", async () => {
    const user = userEvent.setup();
    render(<Reservations />);

    await screen.findByTestId("timeline-range-label");
    const initialLabel = screen.getByTestId("timeline-range-label").textContent;
    expect(screen.queryByRole("button", { name: /hôm nay/i })).toBeNull();

    await user.click(screen.getByRole("button", { name: /tuần sau/i }));
    const todayButton = screen.getByRole("button", { name: /hôm nay/i });
    expect(todayButton).toBeTruthy();

    await user.click(todayButton);

    expect(screen.queryByRole("button", { name: /hôm nay/i })).toBeNull();
    expect(screen.getByTestId("timeline-range-label").textContent).toBe(initialLabel);
  });

  it("keeps the today marker when today is still inside the range after paging back", async () => {
    const user = userEvent.setup();
    render(<Reservations />);

    await screen.findByTestId("timeline-range-label");
    await user.click(screen.getByRole("button", { name: /tuần trước/i }));

    // Lùi 7 ngày: lưới phủ hôm nay - 10 đến hôm nay + 5, hôm nay vẫn nằm trong.
    expect(screen.getByTestId("timeline-today-marker")).toBeTruthy();
  });
});

describe("Reservations checked-out bookings", () => {
  function checkedOutBooking() {
    return {
      ...bookedReservation(),
      id: "B-OUT",
      guest_name: "Hoseo Kim",
      status: "checked_out",
      actual_checkout: "2026-07-25T09:12:00+07:00",
      paid_amount: 500000,
      scheduled_checkin: dateOffsetFromToday(-2),
      scheduled_checkout: dateOffsetFromToday(-1),
    };
  }

  beforeEach(() => {
    vi.clearAllMocks();
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_all_bookings") return [checkedOutBooking()];
      if (command === "get_invoice") return { id: "invoice-1" };
      return undefined;
    });
    invokeWriteCommand.mockResolvedValue(undefined);
    createCorrelationId.mockReturnValue("COR-5E6F7A8B");
  });

  it("opens a read-only popup when a checked-out bar is clicked", async () => {
    const user = userEvent.setup();
    render(<Reservations />);

    await user.click(await screen.findByTestId("booking-bar-B-OUT"));

    expect(screen.getByText(/Đã trả — Hoseo Kim/)).toBeTruthy();
    expect(screen.getByText("Trả phòng lúc")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /^hủy$/i })).toBeNull();
  });

  it("reads the existing invoice instead of generating a new one", async () => {
    const user = userEvent.setup();
    render(<Reservations />);

    await user.click(await screen.findByTestId("booking-bar-B-OUT"));
    await user.click(screen.getByRole("button", { name: /xem hóa đơn/i }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("get_invoice", { bookingId: "B-OUT" });
    });
    expect(invokeWriteCommand).not.toHaveBeenCalledWith(
      "generate_invoice",
      expect.anything(),
    );
    expect(await screen.findByText("invoice-dialog")).toBeTruthy();
  });
});
