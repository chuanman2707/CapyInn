import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import RoomDrawer from "./RoomDrawer";

const { invoke } = vi.hoisted(() => ({
    invoke: vi.fn(),
}));

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
