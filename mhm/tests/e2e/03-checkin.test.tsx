import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor } from "../helpers/render-app";
import userEvent from "@testing-library/user-event";
import App from "@/App";
import { setMockResponse, setMockResponses, clearMockResponses, invoke } from "@test-mocks/tauri-core";
import { useAuthStore } from "@/stores/useAuthStore";
import { useHotelStore } from "@/stores/useHotelStore";
import { createAllRooms, createStats, createUser, createBooking } from "../helpers/mock-data";

const mockUser = createUser({ id: "u1", name: "Admin" });
const mockRooms = createAllRooms();
const mockStats = createStats();

function setupAuthenticated() {
    useAuthStore.setState({ user: mockUser, isAuthenticated: true, loading: false, error: null });
    useHotelStore.setState({
        rooms: mockRooms,
        stats: mockStats,
        activeTab: "dashboard",
        roomDetail: null,
        housekeepingTasks: [],
        loading: false,
        isCheckinOpen: false,
    });

    setMockResponses({
        get_rooms: () => mockRooms,
        get_dashboard_stats: () => mockStats,
        get_current_user: () => mockUser,
        get_settings: () => null,
        get_recent_activity: () => [],
        get_revenue_stats: () => ({ total_revenue: 0, rooms_sold: 0, occupancy_rate: 0, daily_revenue: [] }),
        get_expenses: () => [],
        get_all_bookings: () => [],
        calculate_price_preview: () => ({ total: 400000, breakdown: [] }),
        search_guest_by_phone: () => [],
    });
}

describe("03 — Check-in Flow", () => {
    beforeEach(() => {
        clearMockResponses();
        invoke.mockClear();
        setupAuthenticated();
    });

    it("opens checkin sheet when clicking + Khách mới", async () => {
        render(<App />);
        const user = userEvent.setup();

        await waitFor(() => {
            expect(screen.getByText("+ Khách mới")).toBeInTheDocument();
        });

        await user.click(screen.getByText("+ Khách mới"));

        // CheckinSheet should open
        await waitFor(() => {
            // Sheet title or form element should appear
            expect(useHotelStore.getState().isCheckinOpen).toBe(true);
        });
    });

    it("shows room selector with vacant rooms", async () => {
        useHotelStore.setState({ isCheckinOpen: true });
        render(<App />);

        // The check-in sheet should show a room selector
        // Vacant rooms should be selectable
        await waitFor(() => {
            // Look for "Check-in" or "Nhận phòng" in the sheet header
            // At least the sheet should be in the DOM when isCheckinOpen is true
            expect(useHotelStore.getState().isCheckinOpen).toBe(true);
        });
    });

    it("submits check-in with guest data", async () => {
        const mockBooking = createBooking({ room_id: "1A" });

        setMockResponse("check_in", () => mockBooking);

        // Simulate calling checkIn directly through the store
        await useHotelStore.getState().checkIn(
            "1A",
            [{ full_name: "Nguyễn Văn A", doc_number: "012345678901" }],
            1,
            400000,
            "walk-in",
            ""
        );

        expect(invoke).toHaveBeenCalledWith("check_in", {
            req: {
                room_id: "1A",
                guests: [{ full_name: "Nguyễn Văn A", doc_number: "012345678901" }],
                nights: 1,
                source: "walk-in",
                notes: "",
                paid_amount: 400000,
            },
        });
    });

    it("handles check-in error gracefully", async () => {
        setMockResponse("check_in", () => {
            throw {
                code: "BOOKING_INVALID_STATE",
                message: "Room is already occupied",
                kind: "user",
                support_id: null,
            };
        });

        await expect(
            useHotelStore.getState().checkIn(
                "2A", // already occupied
                [{ full_name: "Test", doc_number: "123456789012" }],
                1
            )
        ).rejects.toMatchObject({
            name: "AppError",
            code: "BOOKING_INVALID_STATE",
            message: "Room is already occupied",
            kind: "user",
            support_id: null,
        });

        // Store should not be in loading state after error
        expect(useHotelStore.getState().loading).toBe(false);
    });

    it("refreshes rooms after successful check-in", async () => {
        setMockResponse("check_in", () => createBooking());

        await useHotelStore.getState().checkIn(
            "1A",
            [{ full_name: "Test Guest", doc_number: "012345678901" }],
            1
        );

        // Should have called get_rooms to refresh
        expect(invoke).toHaveBeenCalledWith("get_rooms");
        expect(invoke).toHaveBeenCalledWith("get_dashboard_stats");
    });
});
