import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import RoomDetailPanel from "./RoomDetailPanel";

const checkOut = vi.fn();
let roomTypeRates: Record<string, { room_type: string; nightly_rate: number; configured: boolean }> | null =
    null;

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
                        max_guests: 2,
                        extra_person_fee: 0,
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

describe("RoomDetailPanel nightly rate", () => {
    const roomDetail = {
        room: {
            id: "101",
            name: "101",
            type: "Phòng Đôi",
            floor: 1,
            has_balcony: false,
            // Lệch xa giá loại phòng, để thấy ngay nếu panel in số này.
            base_price: 300000,
            max_guests: 2,
            extra_person_fee: 0,
            status: "vacant" as const,
        },
        booking: null,
        guests: [],
    };

    beforeEach(() => {
        roomTypeRates = null;
    });

    it("shows the room type's rate, not the room's base_price", () => {
        roomTypeRates = {
            "Phòng Đôi": { room_type: "Phòng Đôi", nightly_rate: 480000, configured: true },
        };

        render(<RoomDetailPanel mode="page" roomDetail={roomDetail} />);

        expect(screen.getByTestId("room-detail-nightly-rate").textContent).toContain("480");
        expect(screen.getByTestId("room-detail-nightly-rate").textContent).not.toContain("300");
    });

    it("marks a rate nobody configured, because the fix is to configure one", () => {
        roomTypeRates = {
            "Phòng Đôi": { room_type: "Phòng Đôi", nightly_rate: 300000, configured: false },
        };

        render(<RoomDetailPanel mode="page" roomDetail={roomDetail} />);

        expect(screen.getByTestId("room-detail-nightly-rate").textContent).toContain(
            "chưa cấu hình bảng giá",
        );
    });

    it("shows the same type rate in sheet mode, which renders its own price line", () => {
        // Hai nhánh render riêng, nên phải chốt cả hai: sửa một nhánh về
        // `base_price` mà nhánh kia vẫn xanh là đúng cái bẫy ở đây.
        roomTypeRates = {
            "Phòng Đôi": { room_type: "Phòng Đôi", nightly_rate: 480000, configured: true },
        };

        render(<RoomDetailPanel mode="sheet" roomDetail={roomDetail} onClose={vi.fn()} />);

        const rate = screen.getByTestId("room-sheet-nightly-rate");
        expect(rate.textContent).toContain("480");
        expect(rate.textContent).not.toContain("300");
    });

    it("shows a dash and drops the /đêm suffix when the rate is unknown", () => {
        render(<RoomDetailPanel mode="page" roomDetail={roomDetail} />);

        // "—/đêm" đọc như giá bằng không mỗi đêm; bỏ hậu tố khi không biết giá.
        expect(screen.getByTestId("room-detail-nightly-rate").textContent).toBe("—");
    });
});

describe("RoomDetailPanel payment balance", () => {
    function renderWithPayment(totalPrice: number, paidAmount: number) {
        return render(
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
                        max_guests: 2,
                        extra_person_fee: 0,
                        status: "occupied",
                    },
                    booking: {
                        id: "B700",
                        room_id: "101",
                        primary_guest_id: "G1",
                        check_in_at: "2026-04-20T08:00:00+07:00",
                        expected_checkout: "2026-04-22T12:00:00+07:00",
                        nights: 2,
                        total_price: totalPrice,
                        paid_amount: paidAmount,
                        status: "active",
                        created_at: "2026-04-20T08:00:00+07:00",
                    },
                    guests: [],
                }}
            />,
        );
    }

    /// The panel rendered "Còn nợ −3.800.000đ" in slate grey, which reads as
    /// *nothing owed*. Meanwhile `CheckoutSettlementModal` refuses to check the
    /// booking out until the refund is handled — so the screen the desk looks at
    /// first gave no sign of the state that blocks them.
    it("names an overpayment as money owed back to the guest", () => {
        renderWithPayment(1_200_000, 5_000_000);

        expect(screen.getByText("Trả lại khách")).toBeInTheDocument();
        expect(screen.getByText("3.800.000đ")).toBeInTheDocument();
        expect(screen.queryByText(/-3\.800\.000/)).not.toBeInTheDocument();
        expect(screen.queryByText("Còn nợ")).not.toBeInTheDocument();
    });

    it("still names a real debt a debt", () => {
        renderWithPayment(1_200_000, 500_000);

        expect(screen.getByText("Còn nợ")).toBeInTheDocument();
        expect(screen.getByText("700.000đ")).toBeInTheDocument();
    });

    it("says a fully paid booking is settled rather than owing zero", () => {
        renderWithPayment(1_200_000, 1_200_000);

        expect(screen.getByText("Đã đủ")).toBeInTheDocument();
    });
});

