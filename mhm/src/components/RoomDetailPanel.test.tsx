import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import RoomDetailPanel from "./RoomDetailPanel";

const checkOut = vi.fn();

vi.mock("@/components/CheckoutSettlementModal", () => ({
    default: ({ open }: { open: boolean }) =>
        open ? <div data-testid="checkout-settlement-modal" /> : null,
}));

vi.mock("@/stores/useHotelStore", () => ({
    useHotelStore: () => ({
        checkOut,
        extendStay: vi.fn(),
        getStayInfoText: vi.fn(),
        setTab: vi.fn(),
        setCheckinOpen: vi.fn(),
        loading: false,
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

describe("RoomDetailPanel checkout settlement", () => {
    beforeEach(() => {
        checkOut.mockReset();
    });

    it("opens the shared checkout settlement modal from the detail page", async () => {
        const user = userEvent.setup();

        render(
            <RoomDetailPanel
                mode="page"
                roomDetail={{
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
                        id: "B600",
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
                }}
            />
        );

        await user.click(screen.getByRole("button", { name: /check-out/i }));

        expect(screen.getByTestId("checkout-settlement-modal")).toBeInTheDocument();
    });
});
