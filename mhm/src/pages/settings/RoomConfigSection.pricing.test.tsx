import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import RoomConfigSection from "./RoomConfigSection";
import { clearMockResponses, setMockResponse } from "@test-mocks/tauri-core";
import { useHotelStore } from "@/stores/useHotelStore";
import type { RoomTypeRate } from "@/types";

const invokeCommand = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invokeCommand", () => ({ invokeCommand }));
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

const ROOM = {
    id: "R-101",
    name: "Phòng 101",
    type: "Phòng Đôi",
    floor: 1,
    has_balcony: false,
    // Lệch xa giá loại phòng, để thấy ngay nếu màn hình cấu hình in số này.
    base_price: 300_000,
    max_guests: 2,
    extra_person_fee: 0,
    status: "vacant",
};

/**
 * Trả bảng giá qua đúng lệnh thật, không set thẳng vào store: màn hình này gọi
 * `fetchRoomTypeRates` lúc mount, nên state đặt tay sẽ bị ghi đè — và chính việc
 * bị ghi đè đó là bằng chứng đường dây mount hoạt động.
 */
function respondWithRates(rates: RoomTypeRate[]) {
    setMockResponse("get_room_type_rates", () => rates);
}

/** Đọc bảng giá thất bại thật, không phải "loại phòng thiếu trong danh sách". */
function failTheRateRead() {
    setMockResponse("get_room_type_rates", () => {
        throw new Error("db locked");
    });
}

const CONFIGURED: RoomTypeRate[] = [
    { room_type: "Phòng Đôi", nightly_rate: 480_000, configured: true },
];
const DERIVED_AT_BASE_PRICE: RoomTypeRate[] = [
    { room_type: "Phòng Đôi", nightly_rate: 300_000, configured: false },
];

describe("RoomConfigSection room row price", () => {
    beforeEach(() => {
        invokeCommand.mockReset();
        invokeCommand.mockImplementation(async (command: string) => {
            if (command === "get_rooms") return [ROOM];
            if (command === "get_room_types") return [{ id: "T1", name: "Phòng Đôi" }];
            return [];
        });
        clearMockResponses();
        useHotelStore.setState({ roomTypeRates: null });
    });

    it("shows the type's rate as the room's price, not the room's base_price", async () => {
        respondWithRates(CONFIGURED);

        render(<RoomConfigSection />);

        const price = await screen.findByTestId("room-row-nightly-rate");
        expect(price.textContent).toContain("480");
        expect(price.textContent).not.toContain("300");
    });

    it("warns that the room's own base_price is not the number being charged", async () => {
        respondWithRates(CONFIGURED);

        render(<RoomConfigSection />);

        // Cảnh báo này là thứ duy nhất nói cho admin biết 300.000 họ đã gõ vào
        // đang không có tác dụng gì.
        const warning = await screen.findByTestId("room-row-base-price-unused");
        expect(warning.textContent).toContain("300");
        expect(warning.textContent).toContain("không được dùng");
    });

    it("does not warn when the room's base_price is what the type charges", async () => {
        respondWithRates(DERIVED_AT_BASE_PRICE);

        render(<RoomConfigSection />);

        await screen.findByTestId("room-row-nightly-rate");
        expect(screen.queryByTestId("room-row-base-price-unused")).not.toBeInTheDocument();
    });

    it("shows a dash rather than a price it could not read", async () => {
        failTheRateRead();

        render(<RoomConfigSection />);

        const price = await screen.findByTestId("room-row-nightly-rate");
        expect(price.textContent).toBe("—");
        // Và không cảnh báo bịa: không biết giá loại phòng thì không kết luận
        // được `base_price` có được dùng hay không.
        expect(screen.queryByTestId("room-row-base-price-unused")).not.toBeInTheDocument();
    });

    it("asks for the type rates on mount, since this screen has its own room list", async () => {
        // Không có bước này thì màn hình sửa giá — chỗ cần bảng giá nhất — luôn
        // hiện "chưa đọc được", vì nó không dùng danh sách phòng của store.
        respondWithRates(CONFIGURED);

        render(<RoomConfigSection />);

        await waitFor(() => {
            expect(useHotelStore.getState().roomTypeRates?.["Phòng Đôi"]?.nightly_rate).toBe(
                480_000,
            );
        });
    });
});

describe("RoomFormDialog base price role", () => {
    beforeEach(() => {
        invokeCommand.mockReset();
        invokeCommand.mockImplementation(async (command: string) => {
            if (command === "get_rooms") return [ROOM];
            if (command === "get_room_types") return [{ id: "T1", name: "Phòng Đôi" }];
            return [];
        });
        clearMockResponses();
        useHotelStore.setState({ roomTypeRates: null });
    });

    async function openForm() {
        const user = userEvent.setup();
        render(<RoomConfigSection />);
        await screen.findByTestId("room-row-nightly-rate");
        await user.click(screen.getByRole("button", { name: /thêm phòng/i }));
        return user;
    }

    it("says the number will not affect what guests pay when the type has a rule", async () => {
        respondWithRates(CONFIGURED);

        await openForm();

        const role = await screen.findByTestId("base-price-role");
        expect(role.textContent).toContain("đã có bảng giá");
        expect(role.textContent).toContain("480");
        expect(role.textContent).toContain("không");
    });

    it("says the lowest room id decides the type price when the type has no rule", async () => {
        respondWithRates(DERIVED_AT_BASE_PRICE);

        await openForm();

        const role = await screen.findByTestId("base-price-role");
        expect(role.textContent).toContain("chưa có bảng giá");
        expect(role.textContent).toContain("mã nhỏ nhất");
    });

    it("admits it does not know when the rates could not be read", async () => {
        failTheRateRead();

        await openForm();

        const role = await screen.findByTestId("base-price-role");
        expect(role.textContent).toContain("Chưa đọc được bảng giá");
    });

    it("no longer labels the field as the room's price", async () => {
        respondWithRates(CONFIGURED);

        await openForm();

        // "Giá cơ bản" đọc như giá khách trả. Nhãn phải nói đây là giá gốc của
        // phòng, thứ engine có thể bỏ qua.
        expect(screen.getByText(/Giá gốc của phòng/)).toBeInTheDocument();
        expect(screen.queryByText(/Giá cơ bản/)).not.toBeInTheDocument();
    });
});
