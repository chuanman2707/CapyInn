import { render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { clearMockResponses, invoke, setMockResponse } from "@test-mocks/tauri-core";

vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

import Dashboard from "./Dashboard";

function expenseRange(): { from: string; to: string } | null {
    const call = invoke.mock.calls.find(([command]) => command === "get_expenses");
    return call ? (call[1] as { from: string; to: string }) : null;
}

describe("Dashboard expense range", () => {
    beforeEach(() => {
        clearMockResponses();
        invoke.mockClear();
        setMockResponse("get_analytics", () => ({ daily_revenue: [] }));
        setMockResponse("get_expenses", () => []);
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    /// The backend buckets revenue and expenses by `substr(created_at, 1, 10)` of
    /// a **local** rfc3339 stamp (`revenue_queries::local_date_sql`). A range built
    /// from `toISOString()` is a UTC range, so for the first seven hours of every
    /// Vietnamese day it asked about a window shifted a day back.
    it("asks for a local-day window, not a UTC one", async () => {
        vi.useFakeTimers({ shouldAdvanceTime: true });
        vi.setSystemTime(new Date("2026-04-22T02:00:00+07:00"));

        render(<Dashboard />);

        await vi.waitFor(() => expect(expenseRange()).not.toBeNull());
        // 02:00 local on the 22nd is 19:00 UTC on the 21st — `toISOString()` would
        // have said "2026-04-21" here.
        expect(expenseRange()!.to).toBe("2026-04-22");
    });

    it("spans thirty days through the calendar, across a month boundary", async () => {
        vi.useFakeTimers({ shouldAdvanceTime: true });
        vi.setSystemTime(new Date("2026-05-01T02:00:00+07:00"));

        render(<Dashboard />);

        await vi.waitFor(() => expect(expenseRange()).not.toBeNull());
        expect(expenseRange()).toEqual({ from: "2026-04-01", to: "2026-05-01" });
    });
});
