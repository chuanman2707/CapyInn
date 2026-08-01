import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import RoomDrawer from "./RoomDrawer";
import { fmtMoney } from "@/lib/format";

// `extendStay`/`shortenStay` are hoisted (rather than declared inline in the
// store mock factory below) so the SAME mock instance is returned on every
// render. The factory re-runs on every call to `useHotelStore()`, so inline
// `vi.fn()` literals would be replaced each render — making it impossible to
// assert call counts across the re-renders a click triggers.
const { invoke, extendStay, shortenStay } = vi.hoisted(() => ({
    invoke: vi.fn(),
    extendStay: vi.fn(),
    shortenStay: vi.fn(),
}));

let roomTypeRates: Record<string, { room_type: string; nightly_rate: number; configured: boolean }> | null =
    null;

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("sonner", () => ({
    toast: { success: vi.fn(), error: vi.fn() },
}));
vi.mock("@/components/CheckoutSettlementModal", () => ({
    default: ({ open }: { open: boolean }) =>
        open ? <div data-testid="checkout-settlement-modal" /> : null,
}));
vi.mock("@/stores/useHotelStore", () => ({
    useHotelStore: () => ({
        checkOut: vi.fn(),
        extendStay,
        shortenStay,
        getStayInfoText: vi.fn(),
        setCheckinOpen: vi.fn(),
        fetchRooms: vi.fn(),
        updateHousekeeping: vi.fn(),
        roomTypeRates,
    }),
}));
vi.mock("@/hooks/useInvoiceDialog", () => ({
    useInvoiceDialog: () => ({
        invoiceOpen: false,
        invoiceData: null,
        invoiceLoading: false,
        openInvoice: vi.fn(),
        closeInvoice: vi.fn(),
    }),
}));

import { toast } from "sonner";

describe("RoomDrawer checkout settlement", () => {
    beforeEach(() => {
        invoke.mockReset();
    });

    it("opens the shared checkout settlement modal from the drawer", async () => {
        const user = userEvent.setup();
        invoke
            .mockResolvedValueOnce({
                room: {
                    id: "101",
                    name: "101",
                    type: "standard",
                    floor: 1,
                    has_balcony: false,
                    base_price: 500000,
                    status: "occupied",
                },
                booking: {
                    id: "B601",
                    room_id: "101",
                    primary_guest_id: "G1",
                    check_in_at: "2026-04-20T08:00:00+07:00",
                    expected_checkout: "2026-04-25T12:00:00+07:00",
                    nights: 5,
                    total_price: 2500000,
                    paid_amount: 0,
                    status: "active",
                    created_at: "2026-04-20T08:00:00+07:00",
                },
                guests: [],
            })
            .mockResolvedValueOnce([]);

        render(<RoomDrawer open onClose={vi.fn()} roomId="101" />);

        await waitFor(() => {
            expect(screen.getByRole("button", { name: /check-out/i })).toBeInTheDocument();
        });
        await user.click(screen.getByRole("button", { name: /check-out/i }));

        expect(screen.getByTestId("checkout-settlement-modal")).toBeInTheDocument();
    });
});

describe("RoomDrawer nightly rate", () => {
    beforeEach(() => {
        invoke.mockReset();
        roomTypeRates = null;
        invoke
            .mockResolvedValueOnce({
                room: {
                    id: "101",
                    name: "101",
                    type: "Phòng Đôi",
                    floor: 1,
                    has_balcony: false,
                    // Lệch xa giá loại phòng, để thấy ngay nếu drawer in số này.
                    base_price: 300000,
                    status: "vacant",
                },
                booking: null,
                guests: [],
            })
            .mockResolvedValue([]);
    });

    it("shows the room type's rate, not the room's base_price", async () => {
        roomTypeRates = {
            "Phòng Đôi": { room_type: "Phòng Đôi", nightly_rate: 480000, configured: true },
        };

        render(<RoomDrawer open onClose={vi.fn()} roomId="101" />);

        const rate = await screen.findByTestId("room-drawer-nightly-rate");
        expect(rate.textContent).toContain("480");
        expect(rate.textContent).not.toContain("300");
    });

    it("shows a dash without the /đêm suffix when the rate is unknown", async () => {
        render(<RoomDrawer open onClose={vi.fn()} roomId="101" />);

        const rate = await screen.findByTestId("room-drawer-nightly-rate");
        expect(rate.textContent).toBe("—");
    });
});

describe("RoomDrawer nights stepper", () => {
    // Built from the live clock (never hardcoded), so these boundary tests
    // don't rot as "today" moves forward.
    function atLocalTime(base: Date, hours: number, minutes = 0): Date {
        const d = new Date(base);
        d.setHours(hours, minutes, 0, 0);
        return d;
    }

    function addDays(base: Date, days: number): Date {
        const d = new Date(base);
        d.setDate(d.getDate() + days);
        return d;
    }

    const NOW = new Date();
    const TODAY_AT_MIDDAY = atLocalTime(NOW, 12).toISOString();
    const YESTERDAY_AT_TEN = atLocalTime(addDays(NOW, -1), 10).toISOString();
    const TOMORROW_AT_TEN = atLocalTime(addDays(NOW, 1), 10).toISOString();

    const CHECKOUT_TODAY_REASON = "Đêm cuối là hôm nay — dùng Check-out";
    const MIN_NIGHTS_REASON = "Lưu trú tối thiểu 1 đêm";

    type BookingOverrides = Partial<{
        nights: number;
        expected_checkout: string;
        total_price: number;
    }>;

    function buildDetail(bookingOverrides: BookingOverrides = {}) {
        return {
            room: {
                id: "101",
                name: "101",
                type: "standard",
                floor: 1,
                has_balcony: false,
                base_price: 500000,
                status: "occupied",
            },
            booking: {
                id: "B601",
                room_id: "101",
                primary_guest_id: "G1",
                check_in_at: "2026-04-20T08:00:00+07:00",
                expected_checkout: TOMORROW_AT_TEN,
                nights: 5,
                total_price: 2500000,
                paid_amount: 0,
                status: "active",
                created_at: "2026-04-20T08:00:00+07:00",
                ...bookingOverrides,
            },
            guests: [],
        };
    }

    // Queues successive `get_room_detail` responses: the first call (mount)
    // gets `details[0]`, the next call gets `details[1]`, etc. Once the queue
    // is down to its last entry, that entry keeps being returned.
    function mockRoomDetailSequence(...details: ReturnType<typeof buildDetail>[]) {
        const queue = [...details];
        invoke.mockImplementation((cmd: string) => {
            if (cmd === "get_room_detail") {
                const next = queue.length > 1 ? queue.shift()! : queue[0];
                return Promise.resolve(next);
            }
            if (cmd === "get_housekeeping_tasks") {
                return Promise.resolve([]);
            }
            return Promise.reject(new Error(`Unexpected invoke call: ${cmd}`));
        });
    }

    async function renderDrawer(bookingOverrides: BookingOverrides = {}) {
        mockRoomDetailSequence(buildDetail(bookingOverrides));
        const user = userEvent.setup();
        render(<RoomDrawer open onClose={vi.fn()} roomId="101" />);
        const shorten = await screen.findByRole("button", { name: /−1 đêm/ });
        const extend = screen.getByRole("button", { name: /\+1 đêm/ });
        return { user, shorten, extend };
    }

    beforeEach(() => {
        invoke.mockReset();
        extendStay.mockReset();
        shortenStay.mockReset();
        vi.mocked(toast.success).mockClear();
        vi.mocked(toast.error).mockClear();
        roomTypeRates = null;
    });

    describe("date boundary", () => {
        it("disables −1 đêm when checkout is today at midday, even with several nights left", async () => {
            // A missing setHours(0,0,0,0) truncation would wrongly enable this:
            // today-at-12:00 is still > today-at-00:00.
            const { shorten } = await renderDrawer({
                expected_checkout: TODAY_AT_MIDDAY,
                nights: 5,
            });

            expect(shorten).toBeDisabled();
            expect(shorten).toHaveAttribute("title", CHECKOUT_TODAY_REASON);
        });

        it("disables −1 đêm when checkout was yesterday", async () => {
            const { shorten } = await renderDrawer({
                expected_checkout: YESTERDAY_AT_TEN,
                nights: 5,
            });

            expect(shorten).toBeDisabled();
            expect(shorten).toHaveAttribute("title", CHECKOUT_TODAY_REASON);
        });

        it("enables −1 đêm when checkout is tomorrow", async () => {
            const { shorten } = await renderDrawer({
                expected_checkout: TOMORROW_AT_TEN,
                nights: 5,
            });

            expect(shorten).not.toBeDisabled();
            expect(shorten).not.toHaveAttribute("title");
        });
    });

    describe("minimum-nights boundary", () => {
        it("disables −1 đêm with the minimum-nights reason for a 1-night booking", async () => {
            const { shorten } = await renderDrawer({
                expected_checkout: TOMORROW_AT_TEN,
                nights: 1,
            });

            expect(shorten).toBeDisabled();
            expect(shorten).toHaveAttribute("title", MIN_NIGHTS_REASON);
        });
    });

    describe("double-click protection", () => {
        it("calls shortenStay exactly once for two rapid clicks on −1 đêm", async () => {
            let resolveShorten!: () => void;
            shortenStay.mockImplementation(
                () => new Promise<void>((resolve) => { resolveShorten = resolve; }),
            );
            const { user, shorten } = await renderDrawer({
                expected_checkout: TOMORROW_AT_TEN,
                nights: 5,
            });

            await user.click(shorten);
            await user.click(shorten);

            expect(shortenStay).toHaveBeenCalledTimes(1);

            resolveShorten();
            await waitFor(() => expect(shorten).not.toBeDisabled());
        });

        it("does not call extendStay when +1 đêm is clicked while a shorten mutation is in flight", async () => {
            let resolveShorten!: () => void;
            shortenStay.mockImplementation(
                () => new Promise<void>((resolve) => { resolveShorten = resolve; }),
            );
            const { user, shorten, extend } = await renderDrawer({
                expected_checkout: TOMORROW_AT_TEN,
                nights: 5,
            });

            await user.click(shorten);
            await user.click(extend);

            expect(shortenStay).toHaveBeenCalledTimes(1);
            expect(extendStay).not.toHaveBeenCalled();

            resolveShorten();
            await waitFor(() => expect(shorten).not.toBeDisabled());
        });
    });

    describe("busy flag reset on failure", () => {
        it("re-enables −1 đêm after shortenStay rejects, so it isn't permanently dead", async () => {
            shortenStay.mockRejectedValueOnce(new Error("boom"));
            const { user, shorten } = await renderDrawer({
                expected_checkout: TOMORROW_AT_TEN,
                nights: 5,
            });

            await user.click(shorten);

            await waitFor(() => expect(shorten).not.toBeDisabled());
            expect(shortenStay).toHaveBeenCalledTimes(1);
        });
    });

    describe("toast freshness", () => {
        it("reports the refetched nights/total in the success toast, not the pre-mutation values", async () => {
            shortenStay.mockResolvedValueOnce(undefined);
            mockRoomDetailSequence(
                buildDetail({ nights: 5, total_price: 2500000, expected_checkout: TOMORROW_AT_TEN }),
                buildDetail({ nights: 4, total_price: 2000000, expected_checkout: TOMORROW_AT_TEN }),
            );
            const user = userEvent.setup();
            render(<RoomDrawer open onClose={vi.fn()} roomId="101" />);
            const shorten = await screen.findByRole("button", { name: /−1 đêm/ });

            await user.click(shorten);

            await waitFor(() => expect(toast.success).toHaveBeenCalled());
            const [message] = vi.mocked(toast.success).mock.calls[0] as [string];
            expect(message).toContain("4 đêm");
            expect(message).toContain(fmtMoney(2000000));
            expect(message).not.toContain("5 đêm");
            expect(message).not.toContain(fmtMoney(2500000));
        });
    });

    describe("error formatting", () => {
        it("shows a formatted message, never [object Object], when shortenStay rejects", async () => {
            shortenStay.mockRejectedValueOnce({ some: "unrecognized object" });
            const { user, shorten } = await renderDrawer({
                expected_checkout: TOMORROW_AT_TEN,
                nights: 5,
            });

            await user.click(shorten);

            await waitFor(() => expect(toast.error).toHaveBeenCalled());
            const [message] = vi.mocked(toast.error).mock.calls[0] as [string];
            expect(message).not.toContain("[object Object]");
            expect(message).toContain("Có lỗi hệ thống, vui lòng thử lại");
        });
    });
});
