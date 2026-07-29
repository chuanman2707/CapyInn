import type { ButtonHTMLAttributes, ReactNode } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { clearMockResponses, invoke, setMockResponse } from "@test-mocks/tauri-core";
import { useHotelStore } from "@/stores/useHotelStore";
import type { Room } from "@/types";

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

const ROOM: Room = {
  id: "R101",
  name: "R101",
  // A real, multi-word type name — the shipped values have spaces in them.
  type: "Deluxe Balcony",
  floor: 1,
  has_balcony: true,
  // Round on purpose: `base_price × 1 night` is 500.000, which is not what the
  // backend quotes below.
  base_price: 500000,
  max_guests: 2,
  extra_person_fee: 0,
  status: "vacant",
};

/** What a configured rule plus a holiday surcharge actually produces. */
const BACKEND_TOTAL = 632_500;

function pricingResult(total: number, breakdown: { label: string; amount: number }[] = []) {
  return {
    pricing_type: "nightly",
    base_amount: total,
    surcharge_amount: 0,
    weekend_amount: 0,
    total,
    breakdown,
    capped: false,
  };
}

function previewArgs(): { roomId: string; checkIn: string; checkOut: string; guests: unknown }[] {
  return invoke.mock.calls
    .filter(([command]) => command === "calculate_room_price_preview")
    .map(([, args]) => args as never);
}

describe("CheckinSheet total price", () => {
  beforeEach(() => {
    clearMockResponses();
    invoke.mockClear();
    useHotelStore.setState({ isCheckinOpen: true, rooms: [] });
    setMockResponse("get_rooms", () => [ROOM]);
    setMockResponse("calculate_room_price_preview", () => pricingResult(BACKEND_TOTAL));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  /// The sheet rendered `base_price × nights` computed in JavaScript, which
  /// ignores the configured nightly rate, the weekend uplift and any holiday
  /// surcharge. Staff read that number aloud and took a deposit against it.
  it("shows the price the backend will charge, not base_price times nights", async () => {
    render(<CheckinSheet preSelectedRoomId="R101" />);

    await waitFor(() => {
      expect(screen.getByTestId("stay-price-total")).toHaveTextContent("632.500");
    });
    expect(screen.getByTestId("stay-price-total")).not.toHaveTextContent("500.000");
  });

  /// A walk-in is charged from `Local::now()` for `nights` days
  /// (`stay_lifecycle.rs`). A bare date, or a `Z` stamp, prices a different stay
  /// than the one about to be written.
  it("asks about the same instants the check-in will charge for", async () => {
    render(<CheckinSheet preSelectedRoomId="R101" />);

    await waitFor(() => expect(previewArgs().length).toBeGreaterThan(0));

    const { roomId, checkIn, checkOut } = previewArgs()[0];
    expect(roomId).toBe("R101");
    for (const stamp of [checkIn, checkOut]) {
      expect(stamp).toMatch(/[+-]\d{2}:\d{2}$/);
      expect(stamp).not.toContain("Z");
    }
    // `nights` defaults to 1, and the span between the two stamps IS the price.
    expect(Date.parse(checkOut) - Date.parse(checkIn)).toBe(86_400_000);
  });

  /// `stay_lifecycle::check_in` prices with `None`, so a walk-in is never billed
  /// the extra-person fee. Sending a guest count here would quote above what the
  /// desk collects — the same defect as the multiplication, in the other
  /// direction.
  it("asks with no guest count, exactly as the check-in charges", async () => {
    render(<CheckinSheet preSelectedRoomId="R101" />);

    await waitFor(() => expect(previewArgs().length).toBeGreaterThan(0));
    expect(previewArgs()[0].guests).toBeNull();
  });

  /// The local day is what `special_dates` is looked up by. `toISOString()`
  /// reports the UTC day, which before 07:00 in Vietnam is still yesterday.
  it("quotes against the local calendar day, not the UTC one", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date(2026, 3, 20, 2, 0, 0));

    render(<CheckinSheet preSelectedRoomId="R101" />);

    await vi.waitFor(() => expect(previewArgs().length).toBeGreaterThan(0));
    expect(previewArgs()[0].checkIn.slice(0, 10)).toBe("2026-04-20");
  });

  /// A wrong total shown with confidence is worse than no total. The old code
  /// always had a number to show and no way to say "I do not know".
  it("says it could not price the stay rather than falling back to a guess", async () => {
    setMockResponse("calculate_room_price_preview", () => {
      throw new Error("database is locked");
    });

    render(<CheckinSheet preSelectedRoomId="R101" />);

    await waitFor(() => expect(screen.getByTestId("stay-price-error")).toBeInTheDocument());
    expect(screen.queryByTestId("stay-price-total")).not.toBeInTheDocument();
    expect(screen.queryByText(/500\.000/)).not.toBeInTheDocument();
  });

  /// The number came from the breakdown, which was fetched and thrown away.
  /// Without it a total that no longer equals base_price × nights cannot be
  /// explained at the desk.
  it("shows the breakdown behind a total that is not a simple multiplication", async () => {
    setMockResponse("calculate_room_price_preview", () =>
      pricingResult(BACKEND_TOTAL, [
        { label: "Tiền phòng", amount: 550_000 },
        { label: "Phụ thu ngày lễ", amount: 82_500 },
      ]),
    );

    render(<CheckinSheet preSelectedRoomId="R101" />);

    await waitFor(() => expect(screen.getByTestId("stay-price-breakdown")).toBeInTheDocument());
    const breakdown = screen.getByTestId("stay-price-breakdown");
    expect(breakdown).toHaveTextContent("Phụ thu ngày lễ");
    expect(breakdown).toHaveTextContent("82.500");
  });

  /// Price is keyed on the room type, so `rooms.base_price` is not a price the
  /// system charges. Printing it in the picker put a per-room figure beside a
  /// total that no longer derives from it.
  it("does not print a per-room base price beside the quoted total", async () => {
    render(<CheckinSheet preSelectedRoomId="R101" />);

    await waitFor(() => expect(screen.getByTestId("stay-price-total")).toBeInTheDocument());

    const option = screen.getByRole("option", { name: /R101/ });
    expect(option).not.toHaveTextContent("500.000");
  });
});
