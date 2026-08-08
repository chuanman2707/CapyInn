import { renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { clearMockResponses, invoke, setMockResponse } from "@test-mocks/tauri-core";
import type { PricingResult } from "@/types";

import { sumRoomPrices, sumRoomPricesWithOverrides, useRoomPrices } from "./useRoomPrices";

function pricingResult(total: number): PricingResult {
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

const WINDOW = { checkIn: "2026-04-20", checkOut: "2026-04-22" };

describe("useRoomPrices", () => {
  beforeEach(() => {
    clearMockResponses();
    invoke.mockClear();
  });

  it("quotes every room and keys the answers by room id", async () => {
    setMockResponse("calculate_room_price_preview", (args) =>
      pricingResult(args?.roomId === "R101" ? 600_000 : 800_000),
    );

    const { result } = renderHook(() =>
      useRoomPrices({ roomIds: ["R101", "R202"], ...WINDOW, guests: null }),
    );

    await waitFor(() => expect(Object.keys(result.current.byRoomId)).toHaveLength(2));
    expect(result.current.byRoomId.R101.total).toBe(600_000);
    expect(result.current.byRoomId.R202.total).toBe(800_000);
    expect(result.current.failed).toBe(false);
  });

  /// Room ids are operator-entered free text. An earlier version of this idea
  /// round-tripped the list through a space-delimited string, which tore
  /// `"Phòng 5A"` into two ids that do not exist — and each fragment still
  /// *priced*, at the house default, so nothing reported an error.
  it("survives a room id containing a space", async () => {
    setMockResponse("calculate_room_price_preview", () => pricingResult(500_000));

    const { result } = renderHook(() =>
      useRoomPrices({ roomIds: ["Phòng 5A"], ...WINDOW, guests: null }),
    );

    await waitFor(() => expect(result.current.byRoomId["Phòng 5A"]).toBeDefined());
    const asked = invoke.mock.calls
      .filter(([command]) => command === "calculate_room_price_preview")
      .map(([, args]) => (args as { roomId: string }).roomId);
    expect(asked).toEqual(["Phòng 5A"]);
  });

  /// `roomIds` is a fresh array on every render. Keying the effect on the array
  /// itself would re-quote forever; keying on nothing would never re-quote.
  it("does not re-quote when the same ids arrive in a new array", async () => {
    setMockResponse("calculate_room_price_preview", () => pricingResult(500_000));

    const { rerender, result } = renderHook(
      ({ ids }: { ids: string[] }) => useRoomPrices({ roomIds: ids, ...WINDOW, guests: null }),
      { initialProps: { ids: ["R101"] } },
    );

    await waitFor(() => expect(result.current.byRoomId.R101).toBeDefined());
    const before = invoke.mock.calls.length;

    rerender({ ids: ["R101"] });
    await waitFor(() => expect(result.current.loading).toBe(false));

    expect(invoke.mock.calls.length).toBe(before);
  });

  /// A failure must also clear what an *earlier* success left behind. Without
  /// that, changing the dates to a window that cannot be priced leaves the old
  /// quotes on screen, and the group total keeps adding up — from a stay nobody
  /// is booking any more.
  it("clears the previous quotes when a later lookup fails", async () => {
    setMockResponse("calculate_room_price_preview", () => pricingResult(600_000));

    const { rerender, result } = renderHook(
      ({ checkIn }: { checkIn: string }) =>
        useRoomPrices({ roomIds: ["R101"], checkIn, checkOut: "2026-04-30", guests: null }),
      { initialProps: { checkIn: "2026-04-20" } },
    );

    await waitFor(() => expect(result.current.byRoomId.R101).toBeDefined());

    setMockResponse("calculate_room_price_preview", () => {
      throw new Error("database is locked");
    });
    rerender({ checkIn: "2026-04-21" });

    await waitFor(() => expect(result.current.failed).toBe(true));
    expect(result.current.byRoomId).toEqual({});
    expect(sumRoomPrices(["R101"], result.current.byRoomId)).toBeNull();
  });

  /// One failure must not leave a partial map behind: a total summed from three
  /// of four rooms is the same class of lie as multiplying in JavaScript.
  it("reports failure rather than a partial set of quotes", async () => {
    setMockResponse("calculate_room_price_preview", (args) => {
      if (args?.roomId === "R202") throw new Error("database is locked");
      return pricingResult(600_000);
    });

    const { result } = renderHook(() =>
      useRoomPrices({ roomIds: ["R101", "R202"], ...WINDOW, guests: null }),
    );

    await waitFor(() => expect(result.current.failed).toBe(true));
    expect(result.current.byRoomId).toEqual({});
  });
});

describe("sumRoomPrices", () => {
  it("adds the quotes up", () => {
    expect(
      sumRoomPrices(["A", "B"], { A: pricingResult(100), B: pricingResult(250) }),
    ).toBe(350);
  });

  /// The caller renders `…` on `null`. Returning a number here would show a
  /// total that silently leaves a booked room out of it.
  it("refuses to total a set with an unpriced room", () => {
    expect(sumRoomPrices(["A", "B"], { A: pricingResult(100) })).toBeNull();
  });

  it("totals an empty selection as zero, not null", () => {
    expect(sumRoomPrices([], {})).toBe(0);
  });
});

describe("sumRoomPricesWithOverrides", () => {
  /// A group check-in fixture template from the plan used `reduce` with
  /// `?? 0`: a room with neither an override nor an engine price silently
  /// counted as 0₫, which is exactly the "quiet wrong total" `sumRoomPrices`
  /// exists to refuse. This pins that a manually-priced room still leaves the
  /// *other*, unpriced room able to force the whole total to `null`.
  it("still refuses to total when a non-overridden room has no engine price", () => {
    // B is overridden and needs no engine quote; A has neither an override
    // nor an engine price, so the whole total must stay `null`.
    expect(sumRoomPricesWithOverrides(["A", "B"], {}, { B: 999 }, 2)).toBeNull();
  });

  it("adds override × nights for an overridden room and the engine quote for the rest", () => {
    expect(
      sumRoomPricesWithOverrides(
        ["A", "B"],
        { A: pricingResult(100), B: pricingResult(250) },
        { A: 40 },
        3,
      ),
      // A: 40/night × 3 nights = 120 (not the engine's 100). B: untouched, 250.
    ).toBe(370);
  });

  it("prices a fully-overridden group without needing any engine quote", () => {
    expect(sumRoomPricesWithOverrides(["A", "B"], {}, { A: 100, B: 200 }, 1)).toBe(300);
  });

  it("totals an empty selection as zero, not null", () => {
    expect(sumRoomPricesWithOverrides([], {}, {}, 1)).toBe(0);
  });

  /// I1 (review Task 18): a cleared price field sends `Number("") === 0`, and
  /// `0` is not a valid rate — the backend rejects `rate <= 0`
  /// (`group_lifecycle.rs`). Counting `0 != null` as a real override would
  /// silently price the room at 0₫ instead of falling back to its engine
  /// quote — the same "quiet wrong total" class of bug `sumRoomPrices` exists
  /// to refuse.
  it("does not count a zero override as a real price — falls back to the engine quote", () => {
    expect(
      sumRoomPricesWithOverrides(
        ["A", "B"],
        { A: pricingResult(632_500), B: pricingResult(1_012_000) },
        { A: 0 },
        2,
      ),
      // A: override is 0, ignored — falls back to its own engine quote,
      // 632.500. B: untouched, 1.012.000. Not 0 + 1.012.000 = 1.012.000.
    ).toBe(1_644_500);
  });

  it("still refuses to total when a zero-overridden room also lacks an engine price", () => {
    expect(
      sumRoomPricesWithOverrides(["A", "B"], { B: pricingResult(999) }, { A: 0 }, 2),
    ).toBeNull();
  });
});
