import type { ButtonHTMLAttributes, ReactNode } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const setCheckinOpen = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn().mockResolvedValue(() => {}) }));

const ROOM = {
  id: "R101",
  name: "R101",
  type: "standard",
  room_type: "standard",
  floor: 1,
  has_balcony: false,
  // Deliberately a round number: `base_price × nights` for 2 nights is
  // 1,000,000, which is NOT what the backend below quotes.
  base_price: 500000,
  max_guests: 2,
  extra_person_fee: 0,
  status: "vacant",
};

vi.mock("@/stores/useHotelStore", () => ({
  useHotelStore: () => ({
    rooms: [ROOM],
    checkIn: vi.fn(),
    fetchRooms: vi.fn(),
    isCheckinOpen: true,
    setCheckinOpen,
  }),
}));

vi.mock("@/components/ui/sheet", () => ({
  Sheet: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  SheetContent: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  SheetHeader: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  SheetTitle: ({ children }: { children: ReactNode }) => <h2>{children}</h2>,
}));

vi.mock("@/components/ui/button", () => ({
  Button: ({ children, ...props }: ButtonHTMLAttributes<HTMLButtonElement>) => (
    <button {...props}>{children}</button>
  ),
}));

vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

import CheckinSheet from "./CheckinSheet";

/** The uplifted figure a configured rule + a holiday actually produces. */
const BACKEND_TOTAL = 1_265_000;

function previewCalls(): { checkIn: string; checkOut: string; roomType: string }[] {
  return invoke.mock.calls
    .filter(([command]) => command === "calculate_price_preview")
    .map(([, args]) => args);
}

function pricingResult(total: number) {
  return {
    pricing_type: "nightly",
    base_amount: total,
    surcharge_amount: 0,
    weekend_amount: 0,
    total,
    breakdown: [],
    capped: false,
  };
}

describe("CheckinSheet total price", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invoke.mockImplementation((command: string) => {
      if (command === "calculate_price_preview") {
        return Promise.resolve(pricingResult(BACKEND_TOTAL));
      }
      return Promise.resolve(null);
    });
  });

  /// The sheet used to render `base_price × nights` computed in JavaScript, which
  /// ignores the configured nightly rate, the weekend uplift and holiday
  /// surcharges. Staff read that number aloud and collected a deposit against it.
  it("shows the price the backend will charge, not base_price times nights", async () => {
    render(<CheckinSheet preSelectedRoomId="R101" />);

    await waitFor(() => {
      expect(screen.getByTestId("stay-price-total")).toHaveTextContent("1.265.000");
    });
    expect(screen.getByTestId("stay-price-total")).not.toHaveTextContent("500.000");
  });

  it("asks the backend with the room's type and an offset-bearing timestamp", async () => {
    render(<CheckinSheet preSelectedRoomId="R101" />);

    await waitFor(() => expect(invoke).toHaveBeenCalledWith(
      "calculate_price_preview",
      expect.objectContaining({ roomType: "standard", pricingType: "nightly" }),
    ));

    const args = invoke.mock.calls.find(
      ([command]) => command === "calculate_price_preview",
    )?.[1] as { checkIn: string; checkOut: string };

    // A bare `Z` means the local calendar day was thrown away, which is how a
    // check-in just after midnight would be priced against yesterday's holiday.
    for (const stamp of [args.checkIn, args.checkOut]) {
      expect(stamp).toMatch(/[+-]\d{2}:\d{2}$/);
    }
  });

  /// The span between the two stamps IS the price, and nothing pinned it: an
  /// earlier version of this file passed unchanged when the checkout was moved
  /// five days out. `nights` defaults to 1.
  it("quotes exactly the number of nights being booked", async () => {
    render(<CheckinSheet preSelectedRoomId="R101" />);

    await waitFor(() => expect(invoke).toHaveBeenCalledWith(
      "calculate_price_preview",
      expect.anything(),
    ));

    const args = invoke.mock.calls.find(
      ([command]) => command === "calculate_price_preview",
    )?.[1] as { checkIn: string; checkOut: string };

    const spanMs = Date.parse(args.checkOut) - Date.parse(args.checkIn);
    expect(spanMs).toBe(86_400_000);
  });

  /// The number came from `PricingResult.breakdown`, which was fetched and thrown
  /// away. Without it a total that no longer equals base_price × nights is
  /// unexplainable at the desk.
  it("shows the breakdown behind a total that is not a simple multiplication", async () => {
    invoke.mockImplementation((command: string) => {
      if (command !== "calculate_price_preview") return Promise.resolve(null);
      return Promise.resolve({
        ...pricingResult(BACKEND_TOTAL),
        breakdown: [
          { label: "Tiền phòng", amount: 1_100_000 },
          { label: "Phụ thu ngày lễ", amount: 165_000 },
        ],
      });
    });

    render(<CheckinSheet preSelectedRoomId="R101" />);

    await waitFor(() => {
      expect(screen.getByTestId("stay-price-breakdown")).toBeInTheDocument();
    });
    const breakdown = screen.getByTestId("stay-price-breakdown");
    expect(breakdown).toHaveTextContent("Phụ thu ngày lễ");
    expect(breakdown).toHaveTextContent("165.000");
    expect(breakdown).toHaveTextContent("1.100.000");
  });

  /// A wrong total shown with confidence is worse than no total. The old code had
  /// no way to express "I do not know" — it always had a number to show.
  it("says it could not price the stay rather than falling back to a guess", async () => {
    invoke.mockImplementation((command: string) => {
      if (command === "calculate_price_preview") {
        return Promise.reject(new Error("database is locked"));
      }
      return Promise.resolve(null);
    });

    render(<CheckinSheet preSelectedRoomId="R101" />);

    await waitFor(() => {
      expect(screen.getByTestId("stay-price-error")).toBeInTheDocument();
    });
    expect(screen.queryByTestId("stay-price-total")).not.toBeInTheDocument();
    expect(screen.queryByText(/1\.000\.000/)).not.toBeInTheDocument();
  });

  /// The sheet stays open while staff scan documents and type guest details, but
  /// check-in stamps its own clock at submit. A quote taken at 23:50 on a holiday
  /// is charged against the next day's rules five minutes later, so the quote has
  /// to notice the date turning.
  it("re-quotes when the local date turns over", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const beforeMidnight = new Date(2026, 3, 20, 23, 59, 30);
      vi.setSystemTime(beforeMidnight);

      render(<CheckinSheet preSelectedRoomId="R101" />);
      await vi.waitFor(() => expect(previewCalls()).toHaveLength(1));
      expect(previewCalls()[0].checkIn.slice(0, 10)).toBe("2026-04-20");

      vi.setSystemTime(new Date(2026, 3, 21, 0, 0, 30));
      await vi.advanceTimersByTimeAsync(31_000);

      await vi.waitFor(() => expect(previewCalls().length).toBeGreaterThan(1));
      const calls = previewCalls();
      expect(calls[calls.length - 1].checkIn.slice(0, 10)).toBe("2026-04-21");
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not re-quote while the date has not changed", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      vi.setSystemTime(new Date(2026, 3, 20, 12, 0, 0));

      render(<CheckinSheet preSelectedRoomId="R101" />);
      await vi.waitFor(() => expect(previewCalls()).toHaveLength(1));

      vi.setSystemTime(new Date(2026, 3, 20, 12, 10, 0));
      await vi.advanceTimersByTimeAsync(10 * 60_000);

      expect(previewCalls()).toHaveLength(1);
    } finally {
      vi.useRealTimers();
    }
  });
});
