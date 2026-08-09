import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import RoomDrawer from "./RoomDrawer";
import { useAuthStore } from "@/stores/useAuthStore";
import { fmtMoney } from "@/lib/format";

// `extendStay`/`shortenStay`/`fetchRooms` are hoisted (rather than declared
// inline in the store mock factory below) so the SAME mock instance is
// returned on every render. The factory re-runs on every call to
// `useHotelStore()`, so inline `vi.fn()` literals would be replaced each
// render — making it impossible to assert call counts across the re-renders
// a click triggers.
const { invoke, extendStay, shortenStay, fetchRooms } = vi.hoisted(() => ({
    invoke: vi.fn(),
    extendStay: vi.fn(),
    shortenStay: vi.fn(),
    fetchRooms: vi.fn(),
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
// Bản giả tối giản, giống hệt cách BookingDetailPopup.test.tsx đã làm: bài
// test ở đây chỉ cần xác nhận RoomDrawer NỐI DÂY đúng với VoidBookingDialog
// (mở khi bấm "Xóa lượt này", đóng + làm mới danh sách phòng khi onVoided
// bắn), không lặp lại hành vi bên trong VoidBookingDialog — cái đó đã có
// VoidBookingDialog.test.tsx lo. Nền `bg-black/40` không tự stopPropagation,
// giống hệt bản thật, để bài test "bấm ra nền" bắt đúng lỗi nổi bọt nếu có.
vi.mock("@/components/VoidBookingDialog", () => ({
    default: ({ onClose, onVoided }: { onClose: () => void; onVoided: () => void }) => (
        <div className="fixed inset-0 bg-black/40" onClick={onClose}>
            <button onClick={onVoided}>mock xác nhận xóa</button>
        </div>
    ),
}));
const setRoomChangeOpen = vi.fn();

vi.mock("@/stores/useHotelStore", () => ({
    useHotelStore: () => ({
        checkOut: vi.fn(),
        extendStay,
        shortenStay,
        getStayInfoText: vi.fn(),
        setCheckinOpen: vi.fn(),
        setRoomChangeOpen,
        fetchRooms,
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
        setRoomChangeOpen.mockReset();
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

    it("opens the room change sheet for the current booking", async () => {
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
            expect(screen.getByRole("button", { name: /chuyển phòng/i })).toBeInTheDocument();
        });
        await user.click(screen.getByRole("button", { name: /chuyển phòng/i }));

        expect(setRoomChangeOpen).toHaveBeenCalledWith(true, "B601");
    });

    // `roomDetail` là state cục bộ, effect nạp nó chỉ phụ thuộc [open, roomId]
    // — không cái nào đổi khi khách chuyển phòng — và listener "db-updated"
    // toàn cục chỉ làm mới rooms/stats của store. Nếu drawer không đóng thì nó
    // đứng nguyên với phòng cũ và tổng tiền cũ, kèm nút Check-out mở
    // CheckoutSettlementModal ghi đúng cái `roomId` cũ đó.
    it("đóng lại sau khi bàn giao cho sheet chuyển phòng, không đứng lại với phòng cũ", async () => {
        const user = userEvent.setup();
        const onClose = vi.fn();
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

        render(<RoomDrawer open onClose={onClose} roomId="101" />);

        await waitFor(() => {
            expect(screen.getByRole("button", { name: /chuyển phòng/i })).toBeInTheDocument();
        });
        await user.click(screen.getByRole("button", { name: /chuyển phòng/i }));

        expect(setRoomChangeOpen).toHaveBeenCalledWith(true, "B601");
        expect(onClose).toHaveBeenCalled();
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
        paid_amount: number;
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

    describe("paid-amount boundary", () => {
        // Reversal: shortening used to be allowed to push total_price below
        // paid_amount (front desk would hand back cash). check_out_tx refuses
        // that state outright, so the button itself must now stay disabled
        // instead of letting the click fail. The credit for one night must
        // match the backend's exact formula in shorten_stay_tx: integer
        // division, floored — `total_price / nights`.
        it("disables −1 đêm when the credit would drop the total below paid_amount", async () => {
            // total_price=2,500,000 / nights=5 → credit=500,000 →
            // total after shorten = 2,000,000, below paid_amount=2,100,000.
            const { shorten } = await renderDrawer({
                expected_checkout: TOMORROW_AT_TEN,
                nights: 5,
                total_price: 2_500_000,
                paid_amount: 2_100_000,
            });

            expect(shorten).toBeDisabled();
            expect(shorten).toHaveAttribute(
                "title",
                `Khách đã thanh toán ${fmtMoney(2_100_000)}, cao hơn tổng tiền sau khi rút đêm (${fmtMoney(2_000_000)}) — cần xử lý hoàn tiền trước`,
            );
        });

        it("enables −1 đêm when the total after the credit still covers paid_amount", async () => {
            const { shorten } = await renderDrawer({
                expected_checkout: TOMORROW_AT_TEN,
                nights: 5,
                total_price: 2_500_000,
                paid_amount: 1_500_000,
            });

            expect(shorten).not.toBeDisabled();
            expect(shorten).not.toHaveAttribute("title");
        });

        it("enables −1 đêm when the total after the credit lands exactly on paid_amount", async () => {
            // total after shorten = 2,000,000, exactly equal to paid_amount —
            // the guard uses `<`, not `<=`, so this exact-boundary case must
            // stay enabled.
            const { shorten } = await renderDrawer({
                expected_checkout: TOMORROW_AT_TEN,
                nights: 5,
                total_price: 2_500_000,
                paid_amount: 2_000_000,
            });

            expect(shorten).not.toBeDisabled();
            expect(shorten).not.toHaveAttribute("title");
        });

        it("reports the minimum-nights reason, not the paid-amount reason, when both apply", async () => {
            const { shorten } = await renderDrawer({
                expected_checkout: TOMORROW_AT_TEN,
                nights: 1,
                total_price: 2_500_000,
                paid_amount: 999_999_999,
            });

            expect(shorten).toBeDisabled();
            expect(shorten).toHaveAttribute("title", MIN_NIGHTS_REASON);
        });

        it("reports the checkout-today reason, not the paid-amount reason, when both apply", async () => {
            const { shorten } = await renderDrawer({
                expected_checkout: TODAY_AT_MIDDAY,
                nights: 5,
                total_price: 2_500_000,
                paid_amount: 999_999_999,
            });

            expect(shorten).toBeDisabled();
            expect(shorten).toHaveAttribute("title", CHECKOUT_TODAY_REASON);
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

describe("RoomDrawer void action", () => {
    function adminUser() {
        return { id: "u1", name: "Chủ", role: "admin" as const, active: true, created_at: "2026-08-07T09:00:00+07:00" };
    }

    function receptionistUser() {
        return { id: "u2", name: "Lễ tân", role: "receptionist" as const, active: true, created_at: "2026-08-07T09:00:00+07:00" };
    }

    // `group_id` mặc định null — khớp hình dạng thật `get_room_detail` luôn trả
    // (mhm/src-tauri/src/queries/booking/room_queries.rs): trường luôn có mặt,
    // không thuộc đoàn thì là null chứ không phải vắng mặt. Override cho phép
    // bài test đoàn (bên dưới) dựng đúng ca `group_id` khác null.
    function buildActiveBookingDetail(overrides: Partial<{ group_id: string | null }> = {}) {
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
                expected_checkout: "2026-04-25T12:00:00+07:00",
                nights: 5,
                total_price: 2500000,
                paid_amount: 0,
                status: "active",
                created_at: "2026-04-20T08:00:00+07:00",
            },
            guests: [],
            group_id: null,
            ...overrides,
        };
    }

    // Helper dựng sẵn ngăn kéo với một lượt đang ở (status active) — tên
    // khớp brief Task 12. Trả về cả `container` để bài test "bấm ra nền hộp
    // xác nhận" có thể tìm nền đó bằng class, giống hệt cách
    // BookingDetailPopup.test.tsx đã làm cho Task 11.
    async function renderDrawerWithActiveBooking(
        onClose: () => void = vi.fn(),
        overrides: Partial<{ group_id: string | null }> = {},
    ) {
        invoke.mockReset();
        invoke.mockResolvedValueOnce(buildActiveBookingDetail(overrides)).mockResolvedValueOnce([]);
        const user = userEvent.setup();
        const view = render(<RoomDrawer open onClose={onClose} roomId="101" />);
        await screen.findByRole("button", { name: /xóa lượt này/i });
        return { user, onClose, ...view };
    }

    beforeEach(() => {
        invoke.mockReset();
        fetchRooms.mockReset();
        setRoomChangeOpen.mockReset();
    });

    it("admin thấy nút xóa lượt đang ở, không kèm gợi ý nào", async () => {
        useAuthStore.setState({ user: adminUser() });
        await renderDrawerWithActiveBooking();
        const button = screen.getByRole("button", { name: /xóa lượt này/i });
        expect(button.hasAttribute("disabled")).toBe(false);
        // Ghim luôn việc KHÔNG có gợi ý nào đi kèm — thiếu dòng này, một bản
        // sửa lỡ render một gợi ý mồ côi dưới nút đã bật (mất đồng bộ với
        // voidDisabled) vẫn xanh, vì trước giờ chỉ mỗi ca lễ tân kiểm tra hint.
        expect(button.parentElement?.querySelector("p")).toBeNull();
    });

    it("lễ tân không bấm được nút xóa", async () => {
        useAuthStore.setState({ user: receptionistUser() });
        await renderDrawerWithActiveBooking();
        expect(
            screen.getByRole("button", { name: /xóa lượt này/i }).hasAttribute("disabled"),
        ).toBe(true);
        expect(screen.getByText(/chỉ chủ khách sạn xóa được/i)).toBeTruthy();
    });

    // Task 11 (BookingDetailPopup) đã khoá trước cho lượt thuộc đoàn nhờ
    // `BookingWithGuest.group_id`. Trước bản sửa này, RoomDrawer không có dữ
    // liệu đó (`Booking` từ `get_room_detail` không mang group_id) nên admin
    // thấy nút bật, bấm, đợi preview tải xong mới biết bị chặn — hai lối vào
    // của cùng một hành động phá hoại nhưng cư xử khác nhau. Giờ
    // `RoomWithBooking.group_id` (mhm/src-tauri/src/queries/booking/room_queries.rs)
    // mang dữ liệu đó lên tới đây — kể cả admin cũng phải bị khoá TRƯỚC khi bấm.
    it("lượt thuộc đoàn thì nút xóa bị khoá với lý do riêng, kể cả với admin", async () => {
        useAuthStore.setState({ user: adminUser() });
        await renderDrawerWithActiveBooking(undefined, { group_id: "GRP-1" });

        const button = screen.getByRole("button", { name: /xóa lượt này/i });
        expect(button.hasAttribute("disabled")).toBe(true);
        expect(screen.getByText(/thuộc đoàn/i)).toBeTruthy();
    });

    it("phòng trống thì không có nút xóa", async () => {
        useAuthStore.setState({ user: adminUser() });
        invoke.mockReset();
        invoke
            .mockResolvedValueOnce({
                room: {
                    id: "101",
                    name: "101",
                    type: "standard",
                    floor: 1,
                    has_balcony: false,
                    base_price: 500000,
                    status: "vacant",
                },
                booking: null,
                guests: [],
            })
            .mockResolvedValueOnce([]);

        render(<RoomDrawer open onClose={vi.fn()} roomId="101" />);

        await screen.findByText(/check-in phòng này/i);
        expect(screen.queryByRole("button", { name: /xóa lượt này/i })).toBeNull();
    });

    // Ý đồ thiết kế: nút xóa đứng CUỐI CÙNG trong nội dung ngăn kéo — sau giá,
    // ghi chú (bookingSection) lẫn hàng nút chính (actionsSection: copy lưu
    // trú, đêm, chuyển phòng, Check-out) — không được lẫn vào giữa các hành
    // động chính. Trước bài test này không gì ghim vị trí đó cả; một sửa đổi
    // lỡ dời voidSection lên trước actionsSection trong RoomDrawer.tsx vẫn sẽ
    // xanh ở mọi test khác. Neo vào nút Check-out — nút cuối cùng, luôn có
    // mặt, của actionsSection — làm mốc ổn định để so vị trí.
    it("nút xóa nằm sau nút Check-out, không lẫn vào hàng nút chính", async () => {
        useAuthStore.setState({ user: adminUser() });
        await renderDrawerWithActiveBooking();

        const checkoutButton = screen.getByRole("button", { name: /check-out/i });
        const voidButton = screen.getByRole("button", { name: /xóa lượt này/i });

        // Node.DOCUMENT_POSITION_FOLLOWING trên compareDocumentPosition gọi từ
        // checkoutButton nghĩa là voidButton đứng SAU nó trong cây DOM.
        expect(
            checkoutButton.compareDocumentPosition(voidButton) & Node.DOCUMENT_POSITION_FOLLOWING,
        ).toBeTruthy();
    });

    // `get_room_detail` (mhm/src-tauri/src/queries/booking/room_queries.rs)
    // chỉ SELECT booking có status = 'active', nên trên thực tế backend
    // không bao giờ trả về nhánh này — nhưng điều kiện hiển thị đọc thẳng
    // `booking.status` của component, không dựa vào việc backend đã lọc sẵn.
    // Test này khoá đúng yêu cầu đề bài: "phòng ... đã trả không được có nút
    // này", phòng khi component được tái dùng ở một đường dữ liệu khác sau này.
    it("lượt không còn active (đã trả) thì không có nút xóa", async () => {
        useAuthStore.setState({ user: adminUser() });
        const detail = buildActiveBookingDetail();
        invoke.mockReset();
        invoke
            .mockResolvedValueOnce({
                ...detail,
                booking: { ...detail.booking, status: "checked_out" },
            })
            .mockResolvedValueOnce([]);

        render(<RoomDrawer open onClose={vi.fn()} roomId="101" />);

        await waitFor(() => expect(screen.getByRole("button", { name: /check-out/i })).toBeInTheDocument());
        expect(screen.queryByRole("button", { name: /xóa lượt này/i })).toBeNull();
    });

    it("void thành công thì đóng ngăn kéo và làm mới danh sách phòng", async () => {
        useAuthStore.setState({ user: adminUser() });
        const onClose = vi.fn();
        const { user } = await renderDrawerWithActiveBooking(onClose);

        await user.click(screen.getByRole("button", { name: /xóa lượt này/i }));
        await user.click(screen.getByRole("button", { name: /mock xác nhận xóa/i }));

        await waitFor(() => expect(onClose).toHaveBeenCalledTimes(1));
        expect(fetchRooms).toHaveBeenCalled();
    });

    // SlideDrawer (mhm/src/components/shared/SlideDrawer.tsx) đặt nền bấm-để-
    // đóng trên MỘT div riêng (`absolute inset-0`), không phải trên chính div
    // bọc ngoài cùng như BookingDetailPopup — nên VoidBookingDialog dù được
    // chèn ở đâu trong cây của RoomDrawer cũng không phải hậu duệ của nền đó.
    // Test này xác nhận trực tiếp: bấm ra nền của hộp xác nhận xoá (mock)
    // không kích hoạt `onClose` của ngăn kéo phòng.
    it("bấm ra nền của hộp xác nhận xoá không đóng theo ngăn kéo phòng", async () => {
        useAuthStore.setState({ user: adminUser() });
        const onClose = vi.fn();
        const { user, container } = await renderDrawerWithActiveBooking(onClose);

        await user.click(screen.getByRole("button", { name: /xóa lượt này/i }));

        const voidBackdrop = container.querySelector('[class*="bg-black/40"]');
        expect(voidBackdrop).not.toBeNull();
        fireEvent.click(voidBackdrop!);

        expect(onClose).not.toHaveBeenCalled();
    });
});
