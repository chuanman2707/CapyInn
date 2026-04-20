import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, beforeEach, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";

import CheckoutSettlementModal from "./CheckoutSettlementModal";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const booking = {
    id: "B500",
    room_id: "101",
    primary_guest_id: "G1",
    check_in_at: "2026-04-20T08:00:00+07:00",
    expected_checkout: "2026-04-25T12:00:00+07:00",
    nights: 5,
    total_price: 2500000,
    paid_amount: 0,
    status: "active" as const,
    created_at: "2026-04-20T08:00:00+07:00",
};

describe("CheckoutSettlementModal", () => {
    beforeEach(() => {
        vi.mocked(invoke).mockReset();
    });

    it("loads the default actual-nights preview from the backend", async () => {
        vi.mocked(invoke).mockResolvedValueOnce({
            settlement_mode: "actual_nights",
            settled_nights: 1,
            recommended_total: 500000,
            explanation: "Thanh toán theo số đêm thực tế: 1 đêm",
        });

        render(
            <CheckoutSettlementModal
                open
                roomId="101"
                booking={booking}
                onClose={vi.fn()}
                onConfirm={vi.fn()}
            />
        );

        await waitFor(() => {
            expect(
                screen.getByText("Thanh toán theo số đêm thực tế: 1 đêm")
            ).toBeInTheDocument();
        });
        expect(screen.getByDisplayValue("500000")).toBeInTheDocument();
    });

    it("requests a fresh preview when switching to hourly mode", async () => {
        const user = userEvent.setup();
        vi.mocked(invoke)
            .mockResolvedValueOnce({
                settlement_mode: "actual_nights",
                settled_nights: 1,
                recommended_total: 500000,
                explanation: "Thanh toán theo số đêm thực tế: 1 đêm",
            })
            .mockResolvedValueOnce({
                settlement_mode: "hourly",
                settled_nights: 1,
                recommended_total: 300000,
                explanation: "Thanh toán theo giờ: nhập tay số tiền quyết toán",
            });

        render(
            <CheckoutSettlementModal
                open
                roomId="101"
                booking={booking}
                onClose={vi.fn()}
                onConfirm={vi.fn()}
            />
        );

        await waitFor(() => expect(screen.getByDisplayValue("500000")).toBeInTheDocument());
        await user.click(screen.getByRole("button", { name: "Theo giờ" }));

        await waitFor(() => {
            expect(
                screen.getByText("Thanh toán theo giờ: nhập tay số tiền quyết toán")
            ).toBeInTheDocument();
        });
        expect(screen.getByDisplayValue("300000")).toBeInTheDocument();
        expect(invoke).toHaveBeenCalledWith("preview_checkout_settlement", {
            req: { booking_id: "B500", settlement_mode: "hourly" },
        });
    });

    it("keeps a manual override after the preview is loaded", async () => {
        const user = userEvent.setup();
        vi.mocked(invoke).mockResolvedValue({
            settlement_mode: "actual_nights",
            settled_nights: 1,
            recommended_total: 500000,
            explanation: "Thanh toán theo số đêm thực tế: 1 đêm",
        });

        render(
            <CheckoutSettlementModal
                open
                roomId="101"
                booking={booking}
                onClose={vi.fn()}
                onConfirm={vi.fn()}
            />
        );

        await waitFor(() => expect(screen.getByDisplayValue("500000")).toBeInTheDocument());
        const input = screen.getByLabelText("Thanh toán cuối");
        await user.clear(input);
        await user.type(input, "200000");

        expect(screen.getByDisplayValue("200000")).toBeInTheDocument();
    });

    it("blocks confirm when the booking is already overpaid for the requested total", async () => {
        vi.mocked(invoke).mockResolvedValueOnce({
            settlement_mode: "actual_nights",
            settled_nights: 1,
            recommended_total: 500000,
            explanation: "Thanh toán theo số đêm thực tế: 1 đêm",
        });

        render(
            <CheckoutSettlementModal
                open
                roomId="101"
                booking={{ ...booking, paid_amount: 700000 }}
                onClose={vi.fn()}
                onConfirm={vi.fn()}
            />
        );

        await waitFor(() => expect(screen.getByDisplayValue("500000")).toBeInTheDocument());
        expect(screen.getByText(/refund/i)).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Xác nhận" })).toBeDisabled();
    });
});
