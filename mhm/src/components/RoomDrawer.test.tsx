import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import RoomDrawer from "./RoomDrawer";

const { invoke } = vi.hoisted(() => ({
    invoke: vi.fn(),
}));

let roomTypeRates: Record<string, { room_type: string; nightly_rate: number; configured: boolean }> | null =
    null;

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("@/components/CheckoutSettlementModal", () => ({
    default: ({ open }: { open: boolean }) =>
        open ? <div data-testid="checkout-settlement-modal" /> : null,
}));
vi.mock("@/stores/useHotelStore", () => ({
    useHotelStore: () => ({
        checkOut: vi.fn(),
        extendStay: vi.fn(),
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
