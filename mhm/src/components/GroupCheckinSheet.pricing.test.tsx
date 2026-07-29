import type {
  ButtonHTMLAttributes,
  InputHTMLAttributes,
  LabelHTMLAttributes,
  ReactNode,
} from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const autoAssignRooms = vi.hoisted(() => vi.fn());
const groupCheckIn = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({ invoke }));

/**
 * Three rooms, two types. `base_price × nights` for 1 night would be 1,700,000 —
 * a number the backend below never returns, so any test that passes on it is
 * reading the old JavaScript multiplication.
 */
const ROOMS = [
  { id: "R101", name: "R101", type: "Standard Room", floor: 1, has_balcony: false, base_price: 500000, max_guests: 2, extra_person_fee: 0, status: "vacant" },
  { id: "R102", name: "R102", type: "Standard Room", floor: 1, has_balcony: false, base_price: 500000, max_guests: 2, extra_person_fee: 0, status: "vacant" },
  { id: "R201", name: "R201", type: "Deluxe Balcony", floor: 2, has_balcony: true, base_price: 700000, max_guests: 2, extra_person_fee: 0, status: "vacant" },
];

const PRICE_BY_TYPE: Record<string, number> = {
  "Standard Room": 660_000,
  "Deluxe Balcony": 924_000,
};

/** 660,000 × 2 + 924,000 — what the folio will record. */
const EXPECTED_TOTAL = "2.244.000";

vi.mock("@/stores/useHotelStore", () => ({
  useHotelStore: () => ({
    isGroupCheckinOpen: true,
    setGroupCheckinOpen: vi.fn(),
    rooms: ROOMS,
    groupCheckIn,
    autoAssignRooms,
    loading: false,
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

vi.mock("@/components/ui/input", () => ({
  Input: (props: InputHTMLAttributes<HTMLInputElement>) => <input {...props} />,
}));

vi.mock("@/components/ui/label", () => ({
  Label: ({ children, ...props }: LabelHTMLAttributes<HTMLLabelElement>) => (
    <label {...props}>{children}</label>
  ),
}));

vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

import GroupCheckinSheet from "./GroupCheckinSheet";

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

function previewCalls(): { roomType: string; checkIn: string; checkOut: string }[] {
  return invoke.mock.calls
    .filter(([command]) => command === "calculate_price_preview")
    .map(([, args]) => args);
}

/** Drives the wizard from step 1 to the confirmation screen. */
async function reachConfirmation() {
  const user = userEvent.setup();
  render(<GroupCheckinSheet />);

  const textboxes = screen.getAllByRole("textbox");
  await user.type(textboxes[0], "Đoàn Test");
  await user.type(textboxes[1], "Trưởng đoàn");
  await user.click(screen.getByRole("button", { name: /Tiếp theo/i }));
  await user.click(screen.getByRole("button", { name: /Tự động chọn/i }));
  await user.click(screen.getByRole("button", { name: /Tiếp theo/i }));
  await user.click(screen.getByRole("button", { name: /Tiếp theo/i }));

  return user;
}

describe("GroupCheckinSheet pricing", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    autoAssignRooms.mockResolvedValue({
      assignments: ROOMS.map((room) => ({ room, floor: room.floor })),
    });
    groupCheckIn.mockResolvedValue(undefined);
    invoke.mockImplementation((command: string, args: { roomType: string }) => {
      if (command !== "calculate_price_preview") return Promise.resolve(null);
      return Promise.resolve(pricingResult(PRICE_BY_TYPE[args.roomType] ?? 0));
    });
  });

  /// A group is the larger transaction of the two, and it charged through the
  /// same engine a single check-in does while displaying a JavaScript product.
  it("totals what the backend will charge, not base_price times nights", async () => {
    await reachConfirmation();

    await waitFor(() => {
      expect(screen.getByTestId("group-price-total")).toHaveTextContent(EXPECTED_TOTAL);
    });
    expect(screen.getByTestId("group-price-total")).not.toHaveTextContent("1.700.000");
  });

  /// The charge resolves each room id to its type and then runs the same
  /// type-keyed lookup the preview does, so two rooms of one type cannot differ.
  /// Quoting per room would be two identical round trips.
  it("asks once per distinct room type, not once per room", async () => {
    await reachConfirmation();

    await waitFor(() => expect(previewCalls().length).toBeGreaterThan(0));
    const types = previewCalls().map((call) => call.roomType).sort();
    expect(types).toEqual(["Deluxe Balcony", "Standard Room"]);
  });

  /// A total that silently omits one room is the same class of lie the
  /// multiplication was — so there is no partial sum, only an admission.
  it("refuses to show a partial total when a type cannot be priced", async () => {
    invoke.mockImplementation((command: string, args: { roomType: string }) => {
      if (command !== "calculate_price_preview") return Promise.resolve(null);
      if (args.roomType === "Deluxe Balcony") return Promise.reject(new Error("database is locked"));
      return Promise.resolve(pricingResult(PRICE_BY_TYPE["Standard Room"]));
    });

    await reachConfirmation();

    await waitFor(() => {
      expect(screen.getByTestId("group-price-error")).toBeInTheDocument();
    });
    expect(screen.queryByTestId("group-price-total")).not.toBeInTheDocument();
    // Not the two standard rooms on their own, either.
    expect(screen.queryByText(/1\.320\.000/)).not.toBeInTheDocument();
  });

  /// `group_check_in_tx` prices a reservation from the bare `YYYY-MM-DD` strings,
  /// not the `14:00`/`12:00` timestamps composed beside them. A quote built from
  /// the current instant would answer a different question entirely.
  it("quotes a reservation from bare dates, the way the backend prices one", async () => {
    const user = await reachConfirmation();

    // Back to step 1 to set a future date, then forward again.
    for (let i = 0; i < 3; i += 1) {
      await user.click(screen.getByRole("button", { name: /Quay lại/i }));
    }
    const dateInput = document.querySelector<HTMLInputElement>('input[type="date"]');
    expect(dateInput).not.toBeNull();
    invoke.mockClear();
    await user.clear(dateInput!);
    await user.type(dateInput!, "2030-06-15");

    await waitFor(() => expect(previewCalls().length).toBeGreaterThan(0));
    for (const call of previewCalls()) {
      expect(call.checkIn).toBe("2030-06-15");
      expect(call.checkOut).toBe("2030-06-16");
    }
  });

  /// Walk-in pricing keys off `Local::now()`, so the quote must carry a local
  /// offset rather than a UTC-converted day.
  it("quotes a walk-in from an offset-bearing timestamp", async () => {
    await reachConfirmation();

    await waitFor(() => expect(previewCalls().length).toBeGreaterThan(0));
    for (const call of previewCalls()) {
      expect(call.checkIn).toMatch(/[+-]\d{2}:\d{2}$/);
      expect(Date.parse(call.checkOut) - Date.parse(call.checkIn)).toBe(86_400_000);
    }
  });
});
