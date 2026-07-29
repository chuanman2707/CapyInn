import type {
  ButtonHTMLAttributes,
  InputHTMLAttributes,
  LabelHTMLAttributes,
  ReactNode,
} from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { clearMockResponses, invoke, setMockResponse } from "@test-mocks/tauri-core";

const autoAssignRooms = vi.hoisted(() => vi.fn());
const groupCheckIn = vi.hoisted(() => vi.fn());
const setGroupCheckinOpen = vi.hoisted(() => vi.fn());

// Two rooms, two prices, and a multi-word type name. Deliberately not tidier
// than production data: the shipped type names contain spaces, and a fixture
// without one has hidden a bug in this exact code path before.
const ROOMS = [
  {
    id: "R101",
    name: "R101",
    type: "Standard Room",
    room_type: "Standard Room",
    floor: 1,
    has_balcony: false,
    base_price: 500000,
    max_guests: 2,
    extra_person_fee: 0,
    status: "vacant",
  },
  {
    id: "R202",
    name: "R202",
    type: "Deluxe Balcony",
    room_type: "Deluxe Balcony",
    floor: 2,
    has_balcony: true,
    base_price: 800000,
    max_guests: 2,
    extra_person_fee: 0,
    status: "vacant",
  },
];

vi.mock("@/stores/useHotelStore", () => ({
  useHotelStore: () => ({
    isGroupCheckinOpen: true,
    setGroupCheckinOpen,
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

/** Two rooms, quoted individually — not 500.000 + 800.000. */
const QUOTES: Record<string, number> = { R101: 632_500, R202: 1_012_000 };

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

function previewArgs(): { roomId: string; checkIn: string; checkOut: string; guests: unknown }[] {
  return invoke.mock.calls
    .filter(([command]) => command === "calculate_room_price_preview")
    .map(([, args]) => args as never);
}

/** Clicks forward until the summary, which is where the group total is shown. */
async function advanceToSummary(user: ReturnType<typeof userEvent.setup>) {
  for (let step = 0; step < 3; step += 1) {
    if (screen.queryByTestId("group-price-total") || screen.queryByTestId("group-price-error")) {
      return;
    }
    const next = screen.queryByRole("button", { name: /Tiếp theo/i });
    if (!next) return;
    await user.click(next);
  }
}

/** Walks the wizard as far as picking both rooms, which is what starts quoting. */
async function pickBothRooms() {
  const user = userEvent.setup();
  render(<GroupCheckinSheet />);

  const textboxes = screen.getAllByRole("textbox");
  await user.type(textboxes[0], "Đoàn Hà Nội");
  await user.type(textboxes[1], "Trần Văn B");
  await user.click(screen.getByRole("button", { name: /Tiếp theo/i }));
  await user.click(screen.getByRole("button", { name: /Chọn tay/i }));
  await user.click(screen.getByRole("button", { name: /R101/ }));
  await user.click(screen.getByRole("button", { name: /R202/ }));

  // The wizard will not advance without a master room, and the master picker
  // renders below the grid with the same room names — take the later one.
  const masterCandidates = screen.getAllByRole("button", { name: /R101/ });
  await user.click(masterCandidates[masterCandidates.length - 1]);

  return user;
}

describe("GroupCheckinSheet total price", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    clearMockResponses();
    invoke.mockClear();
    groupCheckIn.mockResolvedValue(undefined);
    autoAssignRooms.mockResolvedValue({ assignments: [] });
    setMockResponse("calculate_room_price_preview", (args) =>
      pricingResult(QUOTES[(args as { roomId: string }).roomId] ?? 0),
    );
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  /// The sheet summed `base_price × nights` per room in JavaScript, which
  /// ignores the configured rate, the weekend uplift and any holiday surcharge —
  /// once per room, so the group total was wrong by a multiple.
  it("quotes each room through the engine instead of multiplying", async () => {
    const user = await pickBothRooms();

    await waitFor(() =>
      expect(Array.from(new Set(previewArgs().map((a) => a.roomId))).sort()).toEqual([
        "R101",
        "R202",
      ]),
    );

    await advanceToSummary(user);

    // 632.500 + 1.012.000, the two engine quotes — not 500.000 + 800.000, which
    // is what multiplying the base prices by one night would have shown.
    const total = await screen.findByTestId("group-price-total");
    expect(total).toHaveTextContent("1.644.500");
    expect(total).not.toHaveTextContent("1.300.000");
  });

  /// `group_lifecycle.rs` charges with `None`, so the group is never billed the
  /// extra-person fee. Quoting a guest count would show more than it collects.
  it("asks with no guest count, exactly as the group charge does", async () => {
    await pickBothRooms();

    await waitFor(() => expect(previewArgs().length).toBeGreaterThan(0));
    for (const args of previewArgs()) {
      expect(args.guests).toBeNull();
    }
  });

  /// A walk-in group is priced from `Local::now()`; a reservation from bare
  /// dates. Asking with the wrong pair prices a different stay than the one
  /// being booked.
  it("asks about the instants a walk-in group will be charged for", async () => {
    await pickBothRooms();

    await waitFor(() => expect(previewArgs().length).toBeGreaterThan(0));

    const { checkIn, checkOut } = previewArgs()[0];
    expect(checkIn).toMatch(/[+-]\d{2}:\d{2}$/);
    expect(checkIn).not.toContain("Z");
    expect(Date.parse(checkOut) - Date.parse(checkIn)).toBe(86_400_000);
  });

  /// `toISOString()` reports the UTC day, which before 07:00 in Vietnam is
  /// yesterday. `isReservation` compares the arrival against it, so at 02:00 a
  /// walk-in group read as a *reservation* and was quoted from bare dates —
  /// while `group_lifecycle.rs`, comparing against its own local today, would
  /// charge it down the walk-in branch. Two branches, one stay.
  ///
  /// Asserting the day alone does not catch this: both branches say
  /// "2026-04-20" here. The offset is what distinguishes them.
  it("prices a 02:00 group as the walk-in the backend will charge it as", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date(2026, 3, 20, 2, 0, 0));

    await pickBothRooms();

    await vi.waitFor(() => expect(previewArgs().length).toBeGreaterThan(0));
    const { checkIn } = previewArgs()[0];
    expect(checkIn.slice(0, 10)).toBe("2026-04-20");
    expect(checkIn).toMatch(/T\d{2}:\d{2}:\d{2}[+-]\d{2}:\d{2}$/);
  });

  /// A total summed from one of two rooms is the same class of lie the
  /// multiplication was. There has to be a way to say "I do not know".
  it("says it could not price the group rather than showing a partial total", async () => {
    setMockResponse("calculate_room_price_preview", (args) => {
      if ((args as { roomId: string }).roomId === "R202") {
        throw new Error("database is locked");
      }
      return pricingResult(632_500);
    });

    const user = await pickBothRooms();
    await advanceToSummary(user);

    await screen.findByTestId("group-price-error");
    expect(screen.queryByTestId("group-price-total")).not.toBeInTheDocument();
    // Neither the multiplication nor a total summed from the one room that did
    // price successfully.
    expect(screen.queryByText(/1\.300\.000/)).not.toBeInTheDocument();
    expect(screen.queryByText(/632\.500/)).not.toBeInTheDocument();
  });
});
