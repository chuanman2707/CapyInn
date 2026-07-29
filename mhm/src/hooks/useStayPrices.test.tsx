import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import { sumRoomPrices, useStayPrices } from "./useStayPrices";
import type { PricingResult } from "@/types";

function price(total: number): PricingResult {
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

/**
 * `sumRoomPrices` guards a case the component tests cannot reach: when one type
 * fails, `Promise.all` rejects and the whole map is emptied, so the partial-sum
 * branch never fires through the UI. It fires when a caller sums rooms against a
 * map that is merely incomplete — which is what a future partial-success path
 * would produce.
 */
describe("sumRoomPrices", () => {
    const byType = { "Standard Room": price(660_000), "Deluxe Balcony": price(924_000) };

    it("sums each room against its own type's quote", () => {
        expect(
            sumRoomPrices(
                [{ type: "Standard Room" }, { type: "Standard Room" }, { type: "Deluxe Balcony" }],
                byType,
            ),
        ).toBe(2_244_000);
    });

    it("returns null rather than a total that quietly skips a room", () => {
        expect(sumRoomPrices([{ type: "Standard Room" }, { type: "Suite" }], byType)).toBeNull();
    });

    it("is zero for no rooms, not null", () => {
        expect(sumRoomPrices([], byType)).toBe(0);
    });
});

function Probe({ roomTypes }: { roomTypes: string[] }) {
    const { byType, loading, failed } = useStayPrices({ roomTypes, nights: 1 });
    return (
        <div data-testid="probe">
            {loading ? "loading" : failed ? "failed" : Object.keys(byType).sort().join(",")}
        </div>
    );
}

describe("useStayPrices", () => {
    beforeEach(() => {
        vi.clearAllMocks();
        invoke.mockImplementation((command: string, args: { roomType: string }) => {
            if (command !== "calculate_price_preview") return Promise.resolve(null);
            return Promise.resolve(price(args.roomType === "Deluxe Balcony" ? 924_000 : 660_000));
        });
    });

    const previewCalls = () =>
        invoke.mock.calls.filter(([command]) => command === "calculate_price_preview");

    /// Callers pass the types of the rooms they selected. Ten standard rooms are
    /// one question, not ten — the charge resolves every room id to its type
    /// before pricing, so identical types cannot produce different answers.
    it("asks once per distinct type even when the caller repeats itself", async () => {
        render(<Probe roomTypes={["Standard Room", "Standard Room", "Deluxe Balcony", "Standard Room"]} />);

        await waitFor(() =>
            expect(screen.getByTestId("probe")).toHaveTextContent("Deluxe Balcony,Standard Room"),
        );
        expect(previewCalls()).toHaveLength(2);
    });

    it("does not re-ask when the caller passes a new array with the same types", async () => {
        const { rerender } = render(<Probe roomTypes={["Standard Room", "Deluxe Balcony"]} />);
        await waitFor(() => expect(previewCalls()).toHaveLength(2));

        rerender(<Probe roomTypes={["Deluxe Balcony", "Standard Room"]} />);
        await waitFor(() =>
            expect(screen.getByTestId("probe")).toHaveTextContent("Deluxe Balcony,Standard Room"),
        );

        expect(previewCalls()).toHaveLength(2);
    });

    it("asks nothing at all when there are no types", async () => {
        render(<Probe roomTypes={[]} />);

        await waitFor(() => expect(screen.getByTestId("probe")).toHaveTextContent(""));
        expect(previewCalls()).toHaveLength(0);
    });

    /// The shipped room types are "Standard Room" and "Deluxe Balcony" —
    /// `rooms.type` stores the display name, spaces and all. An earlier version
    /// of this hook joined the type list on a space and split it back apart,
    /// which tore those into four types that do not exist. Each one still
    /// *priced*, because an unknown type falls back to the house default instead
    /// of erroring, so `failed` stayed false and the sheet showed "…" forever
    /// with no error. Every test here used single-word types and saw nothing.
    it("keeps a multi-word type name intact", async () => {
        render(<Probe roomTypes={["Standard Room", "Deluxe Balcony"]} />);

        await waitFor(() => expect(previewCalls()).toHaveLength(2));
        expect(previewCalls().map(([, args]) => args.roomType).sort()).toEqual([
            "Deluxe Balcony",
            "Standard Room",
        ]);
    });

    it("reports failure without a partial map when one type cannot be priced", async () => {
        invoke.mockImplementation((command: string, args: { roomType: string }) => {
            if (command !== "calculate_price_preview") return Promise.resolve(null);
            if (args.roomType === "Deluxe Balcony") return Promise.reject(new Error("database is locked"));
            return Promise.resolve(price(660_000));
        });

        render(<Probe roomTypes={["Standard Room", "Deluxe Balcony"]} />);

        await waitFor(() => expect(screen.getByTestId("probe")).toHaveTextContent("failed"));
    });

    /// A superseded request must not overwrite a newer one. Without the guard the
    /// slow first answer lands last and the sheet shows the price of a room the
    /// user has already moved away from.
    it("ignores an answer that arrives after the question changed", async () => {
        const resolvers: Record<string, (value: PricingResult) => void> = {};
        invoke.mockImplementation((command: string, args: { roomType: string }) => {
            if (command !== "calculate_price_preview") return Promise.resolve(null);
            return new Promise<PricingResult>((resolve) => {
                resolvers[args.roomType] = resolve;
            });
        });

        const { rerender } = render(<Probe roomTypes={["Standard Room"]} />);
        await waitFor(() => expect(previewCalls()).toHaveLength(1));

        rerender(<Probe roomTypes={["Deluxe Balcony"]} />);
        await waitFor(() => expect(previewCalls()).toHaveLength(2));

        // The newer question answers first, then the stale one arrives.
        resolvers["Deluxe Balcony"](price(924_000));
        await waitFor(() =>
            expect(screen.getByTestId("probe")).toHaveTextContent("Deluxe Balcony"),
        );
        resolvers["Standard Room"](price(660_000));

        await waitFor(() =>
            expect(screen.getByTestId("probe")).toHaveTextContent("Deluxe Balcony"),
        );
        expect(screen.getByTestId("probe")).not.toHaveTextContent("Standard Room");
    });
});
