import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "../helpers/render-app";
import userEvent from "@testing-library/user-event";
import Reservations from "@/pages/Reservations";
import { setMockResponse, clearMockResponses, invoke } from "@test-mocks/tauri-core";
import { useHotelStore } from "@/stores/useHotelStore";
import { createBookingWithGuest, createAllRooms } from "../helpers/mock-data";

const mockRooms = createAllRooms();

// Create bookings with dates visible in the current timeline viewport
const now = new Date();
const tomorrow = new Date(now);
tomorrow.setDate(tomorrow.getDate() + 1);

const mockBookings = [
    createBookingWithGuest({
        id: "b1",
        room_id: "2A",
        guest_name: "Nguyễn Văn A",
        status: "active",
        total_price: 400000,
        check_in_at: now.toISOString(),
        expected_checkout: tomorrow.toISOString(),
    }),
    createBookingWithGuest({
        id: "b2",
        room_id: "3B",
        guest_name: "Trần Thị B",
        status: "active",
        total_price: 300000,
        check_in_at: now.toISOString(),
        expected_checkout: tomorrow.toISOString(),
    }),
    createBookingWithGuest({
        id: "b3",
        room_id: "5A",
        guest_name: "Lê Văn C",
        status: "checked_out",
        total_price: 400000,
        check_in_at: now.toISOString(),
        expected_checkout: tomorrow.toISOString(),
    }),
];

describe("09 — Reservations", () => {
    beforeEach(() => {
        clearMockResponses();
        invoke.mockClear();

        // Reservations page needs rooms for the timeline grid
        useHotelStore.setState({
            rooms: mockRooms,
            isCheckinOpen: false,
            checkinRoomId: null,
            checkinNights: null,
        });
        setMockResponse("get_rooms", () => mockRooms);
        setMockResponse("get_all_bookings", () => mockBookings);
        setMockResponse("check_availability", () => ({ available: true, conflicts: [], max_nights: null }));
        setMockResponse("get_rooms_availability", () => mockRooms.map(r => ({
            room: r,
            current_booking: null,
            upcoming_reservations: [],
            next_available_until: null,
        })));
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    it("loads and displays booking list", async () => {
        render(<Reservations />);

        await waitFor(() => {
            expect(invoke).toHaveBeenCalledWith("get_all_bookings", expect.anything());
        });
    });

    it("shows guest names in booking bars", async () => {
        render(<Reservations />);

        await waitFor(() => {
            expect(screen.getByText("Nguyễn Văn A")).toBeInTheDocument();
        });

        expect(screen.getByText("Trần Thị B")).toBeInTheDocument();
    });

    it("shows room IDs in timeline", async () => {
        render(<Reservations />);

        // Rooms are shown as "Room {id}" in the timeline sidebar
        await waitFor(() => {
            expect(screen.getByText("Room 2A")).toBeInTheDocument();
        });

        expect(screen.getByText("Room 3B")).toBeInTheDocument();
    });

    it("renders booking status", async () => {
        render(<Reservations />);

        // Bookings should render — we verify by checking invoke was called
        await waitFor(() => {
            expect(invoke).toHaveBeenCalledWith("get_all_bookings", expect.anything());
        });
    });

    it("filters bookings by the search input", async () => {
        const user = userEvent.setup();

        render(<Reservations />);

        await waitFor(() => {
            expect(screen.getByText("Nguyễn Văn A")).toBeInTheDocument();
        });

        await user.type(screen.getByPlaceholderText("Tìm khách..."), "Trần");

        await waitFor(() => {
            expect(screen.getByText("Trần Thị B")).toBeInTheDocument();
        });

        expect(screen.queryByText("Nguyễn Văn A")).not.toBeInTheDocument();
        expect(screen.queryByText("Lê Văn C")).not.toBeInTheDocument();
    });

    it("positions same-day check-ins on the local calendar day column", async () => {
        vi.useFakeTimers({ toFake: ["Date"] });
        vi.setSystemTime(new Date("2026-04-24T15:36:12+07:00"));
        setMockResponse("get_all_bookings", () => [
            createBookingWithGuest({
                id: "same-day-checkin",
                room_id: "2A",
                guest_name: "Khách ngày 24",
                status: "active",
                check_in_at: "2026-04-24T08:00:00+07:00",
                expected_checkout: "2026-04-25T08:00:00+07:00",
            }),
        ]);

        render(<Reservations />);

        const guestName = await screen.findByText("Khách ngày 24");
        const bookingBar = guestName.closest(".absolute");

        // Grid starts today - 3 (col 0). Check-in on today (col 3) renders half a
        // day into that cell: (3 + 0.5) * 80 = 280px. A timezone bug that shifts
        // the calendar day back would land it a full column earlier at 200px.
        expect(bookingBar).toHaveStyle({ left: "280px" });
        expect(bookingBar).not.toHaveStyle({ left: "200px" });
    });

    it("cắt bar của booking đã trả tại ngày trả thực tế", async () => {
        const dayMs = 86400000;
        const base = new Date();
        setMockResponse("get_all_bookings", () => [
            createBookingWithGuest({
                id: "b-early",
                room_id: "1A",
                guest_name: "Khách Trả Sớm",
                status: "checked_out",
                check_in_at: new Date(base.getTime() - 2 * dayMs).toISOString(),
                expected_checkout: new Date(base.getTime() + 2 * dayMs).toISOString(),
                actual_checkout: base.toISOString(),
            }),
        ]);

        render(<Reservations />);

        const bar = await screen.findByTestId("booking-bar-b-early");
        // Cột đầu = hôm nay − 3. Check-in cách đây 2 ngày → cột 1 → left (1 + 0.5) × 80 = 120px.
        // Kết thúc tại actual_checkout (hôm nay, cột 3) → width (3.5 − 1.5) × 80 = 160px,
        // KHÔNG phải kéo tới expected_checkout (cột 5, width 320px).
        expect(bar.style.left).toBe("120px");
        expect(bar.style.width).toBe("160px");
    });

    it("cắt bar khi khách bị gia hạn rồi trả phòng (dữ liệu thật, timestamp có offset)", async () => {
        // Ca thật do chủ khách sạn báo: Su Huijan phòng 1B nhận 29/07 16:33, bị bấm
        // gia hạn nên expected_checkout thành 31/07, rồi trả phòng 30/07 09:54.
        // Backend ghi timestamp dạng RFC3339 kèm offset "+07:00" và micro-giây
        // (chrono Local::now().to_rfc3339()) — khác định dạng "Z" mà test trên dùng,
        // nên đường parse này cần được phủ riêng.
        const atLocalOffset = (dayShift: number, hour: number, minute: number) => {
            const d = new Date();
            d.setDate(d.getDate() + dayShift);
            d.setHours(hour, minute, 8, 0);
            const offsetMinutes = -d.getTimezoneOffset();
            const sign = offsetMinutes >= 0 ? "+" : "-";
            const pad = (n: number) => String(Math.abs(n)).padStart(2, "0");
            const stamp = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
                + `T${pad(d.getHours())}:${pad(d.getMinutes())}:08.752504`
                + `${sign}${pad(Math.trunc(offsetMinutes / 60))}:${pad(offsetMinutes % 60)}`;
            return stamp;
        };

        setMockResponse("get_all_bookings", () => [
            createBookingWithGuest({
                id: "b-su-huijan",
                room_id: "1B",
                guest_name: "Su Huijan",
                status: "checked_out",
                check_in_at: atLocalOffset(-1, 16, 33),
                expected_checkout: atLocalOffset(1, 16, 33),
                actual_checkout: atLocalOffset(0, 9, 54),
            }),
        ]);

        render(<Reservations />);

        const bar = await screen.findByTestId("booking-bar-b-su-huijan");
        // Cột đầu = hôm nay − 3. Nhận phòng hôm qua → cột 2 → left (2 + 0.5) × 80 = 200px.
        // Kết thúc tại actual_checkout (hôm nay, cột 3) → width (3.5 − 2.5) × 80 = 80px.
        // Bản chưa vá vẽ tới expected_checkout (ngày mai, cột 4) → width 160px.
        expect(bar.style.left).toBe("200px");
        expect(bar.style.width).toBe("80px");
        expect(bar.style.width).not.toBe("160px");
    });

    it("bấm ô hôm nay mở check-in với phòng và số đêm", async () => {
        render(<Reservations />);
        const cell = await screen.findByTestId("cell-1A-3"); // cột 3 = hôm nay
        fireEvent.mouseDown(cell, { button: 0 });
        fireEvent.mouseUp(window);

        const state = useHotelStore.getState();
        expect(state.isCheckinOpen).toBe(true);
        expect(state.checkinRoomId).toBe("1A");
        expect(state.checkinNights).toBe(1);
    });

    it("kéo 2 ô tương lai mở form đặt phòng với ngày điền sẵn", async () => {
        render(<Reservations />);
        const start = await screen.findByTestId("cell-1A-5");
        fireEvent.mouseDown(start, { button: 0, clientX: 400 });
        // Vùng chọn mở rộng theo TOẠ ĐỘ chuột, không theo sự kiện hover: WebKit
        // ngưng bắn hover khi đang giữ nút chuột. Mỗi cột rộng 80px và trong
        // jsdom mọi rect đều bằng 0, nên clientX 500 rơi vào cột 6.
        fireEvent.mouseMove(window, { clientX: 500 });
        fireEvent.mouseUp(window);

        expect(await screen.findByText("Đặt phòng trước")).toBeInTheDocument();
        const iso = (d: Date) =>
            `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
        const expectedCheckIn = new Date(Date.now() + 2 * 86400000);
        const expectedCheckOut = new Date(Date.now() + 4 * 86400000);
        expect(screen.getByDisplayValue(iso(expectedCheckIn))).toBeInTheDocument();
        expect(screen.getByDisplayValue(iso(expectedCheckOut))).toBeInTheDocument();
    });

    it("bấm vào bar booking mở popup chi tiết mà không bắt đầu kéo chọn", async () => {
        render(<Reservations />);
        const bar = await screen.findByTestId("booking-bar-b3"); // status: checked_out
        fireEvent.click(bar);

        // Popup mở
        expect(await screen.findByText("Đã trả — Lê Văn C")).toBeInTheDocument();

        // Không có selection/form nào khác bị kích hoạt bởi cú click này
        expect(useHotelStore.getState().isCheckinOpen).toBe(false);
        expect(screen.queryByText("Đặt phòng trước")).not.toBeInTheDocument();
    });

    it("nhấn chuột phải trên ô không bắt đầu kéo chọn", async () => {
        render(<Reservations />);
        const cell = await screen.findByTestId("cell-1A-3");
        fireEvent.mouseDown(cell, { button: 2 });
        fireEvent.mouseUp(window);

        expect(useHotelStore.getState().isCheckinOpen).toBe(false);
        expect(screen.queryByText("Đặt phòng trước")).not.toBeInTheDocument();
    });

    it("nhấn Escape khi đang kéo sẽ huỷ selection, không mở form nào", async () => {
        render(<Reservations />);
        const start = await screen.findByTestId("cell-1A-5");
        fireEvent.mouseDown(start, { button: 0, clientX: 400 });
        fireEvent.mouseMove(window, { clientX: 500 }); // cột 6
        fireEvent.keyDown(window, { key: "Escape" });
        fireEvent.mouseUp(window);

        expect(useHotelStore.getState().isCheckinOpen).toBe(false);
        expect(screen.queryByText("Đặt phòng trước")).not.toBeInTheDocument();
    });

    it("bấm ô quá khứ mở form ghi bù", async () => {
        render(<Reservations />);
        const cell = await screen.findByTestId("cell-1A-0"); // cột 0 = hôm nay − 3
        fireEvent.mouseDown(cell, { button: 0 });
        fireEvent.mouseUp(window);
        expect(await screen.findByText("Ghi bù sổ khách")).toBeInTheDocument();
    });

    it("thả chuột ngoài cửa sổ (window blur) huỷ selection đang kéo", async () => {
        render(<Reservations />);
        const start = await screen.findByTestId("cell-1A-5");
        fireEvent.mouseDown(start, { button: 0, clientX: 400 });
        fireEvent.mouseMove(window, { clientX: 500 }); // cột 6
        fireEvent(window, new Event("blur"));
        // mouseUp xảy ra sau khi quay lại cửa sổ — selection phải đã bị xoá từ blur,
        // nên mouseUp này không được mở form nào.
        fireEvent.mouseUp(window);

        expect(useHotelStore.getState().isCheckinOpen).toBe(false);
        expect(screen.queryByText("Đặt phòng trước")).not.toBeInTheDocument();
    });
});


// WebKit (WKWebView — engine của app trên macOS) KHÔNG cập nhật hover khi đang
// giữ nút chuột: nó ngưng bắn cặp mouseover/mouseout suốt cú kéo. `onMouseEnter`
// của React được suy ra chính từ cặp đó, nên bản cũ đứng nguyên ở ô đầu và cú
// kéo của chủ khách sạn (2/8 → 5/8 phòng 1B) không mở rộng được vùng chọn.
// Chromium bắn bình thường nên bug vô hình ở đó — và jsdom thì không có engine
// nào cả, nên `fireEvent.mouseOver` chỉ chứng minh được đúng cái cơ chế đã hỏng.
//
// Test này kéo mà KHÔNG phát một sự kiện hover nào: chỉ mousedown, mousemove
// trên window, mouseup — đúng những sự kiện mà mọi engine đều bắn khi kéo.
describe("09 — Reservations kéo chọn không dựa vào hover", () => {
    beforeEach(() => {
        clearMockResponses();
        invoke.mockClear();
        useHotelStore.setState({
            rooms: mockRooms,
            isCheckinOpen: false,
            checkinRoomId: null,
            checkinNights: null,
        });
        setMockResponse("get_rooms", () => mockRooms);
        setMockResponse("get_all_bookings", () => []);
        setMockResponse("check_availability", () => ({ available: true, conflicts: [], max_nights: null }));
    });

    const iso = (offsetDays: number) => {
        const d = new Date();
        d.setDate(d.getDate() + offsetDays);
        return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
    };

    it("mở rộng vùng chọn theo toạ độ chuột, không cần sự kiện hover", async () => {
        render(<Reservations />);

        // Cột 0 = hôm nay − 3, mỗi cột rộng 80px. Trong jsdom mọi
        // getBoundingClientRect đều bằng 0, nên clientX chính là toạ độ trong
        // lưới: cột 5 = 400..479, cột 8 = 640..719.
        fireEvent.mouseDown(await screen.findByTestId("cell-1B-5"), { button: 0, clientX: 400 });
        fireEvent.mouseMove(window, { clientX: 500 }); // cột 6
        fireEvent.mouseMove(window, { clientX: 660 }); // cột 8
        fireEvent.mouseUp(window);

        expect(await screen.findByText("Đặt phòng trước")).toBeInTheDocument();
        // Kéo 4 ô (cột 5→8) = 4 đêm: đến hôm nay+2, đi hôm nay+6.
        expect(screen.getByDisplayValue(iso(2))).toBeInTheDocument();
        expect(screen.getByDisplayValue(iso(6))).toBeInTheDocument();
    });

    it("không mở rộng sang phòng khác khi chuột trượt lên hàng trên", async () => {
        render(<Reservations />);

        fireEvent.mouseDown(await screen.findByTestId("cell-1B-5"), { button: 0, clientX: 400 });
        // Chuột trượt sang cột 8 nhưng ở hàng khác: cú kéo vẫn thuộc phòng 1B.
        fireEvent.mouseMove(window, { clientX: 660, clientY: -500 });
        fireEvent.mouseUp(window);

        expect(await screen.findByText("Đặt phòng trước")).toBeInTheDocument();
        expect((screen.getByLabelText(/^Phòng$/) as HTMLSelectElement).value).toBe("1B");
    });
});
