import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import UnifiedRoomCard from "./UnifiedRoomCard";
import { useHotelStore } from "@/stores/useHotelStore";
import type { Room, RoomTypeRate } from "@/types";

const room: Room = {
    id: "R-101",
    name: "Phòng 101",
    type: "Phòng Đôi",
    floor: 1,
    has_balcony: false,
    // Cố ý lệch xa giá loại phòng: nếu thẻ in số này thì thấy ngay.
    base_price: 300_000,
    max_guests: 2,
    extra_person_fee: 0,
    status: "vacant",
};

function setRates(rates: Record<string, RoomTypeRate> | null) {
    useHotelStore.setState({ roomTypeRates: rates });
}

function renderCard() {
    return render(
        <UnifiedRoomCard room={room} onOpenDrawer={vi.fn()} onQuickAction={vi.fn()} />,
    );
}

describe("UnifiedRoomCard nightly rate", () => {
    beforeEach(() => {
        setRates(null);
    });

    it("shows the room type's rate from the pricing engine, not the room's base_price", () => {
        setRates({
            "Phòng Đôi": { room_type: "Phòng Đôi", nightly_rate: 480_000, configured: true },
        });

        renderCard();

        const rate = screen.getByTestId("room-card-nightly-rate");
        expect(rate.textContent).toContain("480");
        // Đây là cả lý do của thay đổi này: 300k là `base_price`, engine bỏ qua nó
        // khi loại phòng có bảng giá, nên in nó ra là đọc sai giá cho khách.
        expect(rate.textContent).not.toContain("300");
    });

    it("shows a dash instead of a price when the rates could not be loaded", () => {
        renderCard();

        expect(screen.getByTestId("room-card-nightly-rate").textContent).toBe("—");
    });

    it("never prints base_price anywhere on the card, even with no rate to show", () => {
        // Chốt trên toàn thẻ, không chỉ ô giá: kéo `base_price` sang một chỗ khác
        // trên thẻ thì test ô giá vẫn xanh.
        const { container } = renderCard();

        expect(container.textContent).not.toContain("300");
    });
});
