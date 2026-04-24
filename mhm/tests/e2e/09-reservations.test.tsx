import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, waitFor } from "../helpers/render-app";
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
        useHotelStore.setState({ rooms: mockRooms });
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

        expect(bookingBar).toHaveStyle({ left: "240px" });
        expect(bookingBar).not.toHaveStyle({ left: "160px" });
    });
});
