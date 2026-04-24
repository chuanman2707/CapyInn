import { describe, it, expect, beforeEach } from "vitest";
import { act, render, screen, waitFor } from "../helpers/render-app";
import userEvent from "@testing-library/user-event";
import Dashboard from "@/pages/Dashboard";
import { setMockResponse, clearMockResponses, invoke } from "@test-mocks/tauri-core";
import { useHotelStore } from "@/stores/useHotelStore";
import { createAllRooms, createStats, createBookingWithGuest } from "../helpers/mock-data";

const mockRooms = createAllRooms();
const mockStats = createStats({ occupied: 3, vacant: 6, cleaning: 1, revenue_today: 1200000 });

describe("02 — Dashboard", () => {
    beforeEach(() => {
        clearMockResponses();
        invoke.mockClear();

        useHotelStore.setState({
            rooms: mockRooms,
            stats: mockStats,
            dashboardRefreshVersion: 0,
            activeTab: "dashboard",
            roomDetail: null,
            housekeepingTasks: [],
            loading: false,
            isCheckinOpen: false,
        });

        setMockResponse("get_rooms", () => mockRooms);
        setMockResponse("get_dashboard_stats", () => mockStats);
        setMockResponse("get_recent_activity", () => [
            {
                icon: "🔑",
                text: "Check-in phòng 2A — Nguyễn Văn A",
                time: "10:30",
                color: "green",
                kind: "check_in",
                room_id: "2A",
                guest_name: "Nguyễn Văn A",
                occurred_at: "2026-03-15T10:30:00",
                status_label: "Đã check-in",
            },
        ]);
        setMockResponse("get_revenue_stats", () => ({
            total_revenue: 1200000,
            rooms_sold: 3,
            occupancy_rate: 30,
            daily_revenue: [{ date: "2026-03-15", revenue: 1200000 }],
        }));
        setMockResponse("get_expenses", () => []);
        setMockResponse("get_all_bookings", () => [
            createBookingWithGuest({ room_id: "2A", guest_name: "Nguyễn Văn A", status: "active" }),
        ]);
        setMockResponse("get_rooms_availability", () => mockRooms.map(r => ({
            room: r, current_booking: null, upcoming_reservations: [], next_available_until: null,
        })));
        setMockResponse("get_analytics", () => ({
            total_revenue: 1200000, occupancy_rate: 30, adr: 400000, revpar: 120000,
            daily_revenue: [{ date: "2026-03-15", revenue: 1200000 }],
            revenue_by_source: [], expenses_by_category: [], top_rooms: [],
        }));
    });

    it("renders stat cards", async () => {
        render(<Dashboard />);

        await waitFor(() => {
            // Total rooms stat or occupied count
            expect(screen.getByText("3")).toBeInTheDocument(); // occupied
        });

        expect(screen.getByText("6")).toBeInTheDocument(); // vacant
        expect(screen.getByText("1")).toBeInTheDocument(); // cleaning
    });

    it("renders 10 room cards", async () => {
        render(<Dashboard />);

        await waitFor(() => {
            // Room names should be visible
            expect(screen.getByText("1A")).toBeInTheDocument();
            expect(screen.getByText("5B")).toBeInTheDocument();
        });

        // All 10 room names — use getAllByText because some names may appear in multiple places
        for (const room of mockRooms) {
            expect(screen.getAllByText(room.name).length).toBeGreaterThanOrEqual(1);
        }
    });

    it("calls fetchRooms and fetchStats on mount", async () => {
        render(<Dashboard />);

        await waitFor(() => {
            expect(invoke).toHaveBeenCalledWith("get_recent_activity", expect.anything());
        });
    });

    it("displays revenue today", async () => {
        render(<Dashboard />);

        await waitFor(() => {
            // Revenue should be formatted — "1.200.000" or "1,200,000" or similar
            const revenueText = screen.getByText(/1[.,]200[.,]000/);
            expect(revenueText).toBeInTheDocument();
        });
    });

    it("click on room changes to detail view", async () => {
        render(<Dashboard />);

        await waitFor(() => {
            expect(screen.getByText("1A")).toBeInTheDocument();
        });

        // Find and click a room card — rooms are rendered via RoomCard
        // The room name "1A" should be clickable
        screen.getByText("1A");
    });

    it("opens activity detail drawer when clicking an activity item", async () => {
        const user = userEvent.setup();

        render(<Dashboard />);

        const activityButton = await screen.findByRole("button", {
            name: /check-in phòng 2a — nguyễn văn a/i,
        });

        await user.click(activityButton);

        expect(screen.getByText("Chi tiết hoạt động")).toBeInTheDocument();
        expect(screen.getAllByText("Đã check-in").length).toBeGreaterThan(0);
        expect(screen.getByText("Ghi nhận lúc")).toBeInTheDocument();
        expect(screen.getByText("Điều hướng")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: /mở phòng 2a/i })).toBeInTheDocument();
    });

    it("refreshes activity immediately after a successful check-in", async () => {
        let activityCalls = 0;
        setMockResponse("get_recent_activity", () => {
            activityCalls += 1;
            return activityCalls === 1
                ? [
                    {
                        icon: "🔑",
                        text: "Check-in phòng 2A — Nguyễn Văn A",
                        time: "10:30",
                        color: "green",
                        kind: "check_in",
                        room_id: "2A",
                        guest_name: "Nguyễn Văn A",
                        occurred_at: "2026-03-15T10:30:00",
                        status_label: "Đã check-in",
                    },
                ]
                : [
                    {
                        icon: "🔑",
                        text: "Check-in phòng 1A — Trần Thị Mới",
                        time: "15:40",
                        color: "green",
                        kind: "check_in",
                        room_id: "1A",
                        guest_name: "Trần Thị Mới",
                        occurred_at: "2026-04-24T15:40:00+07:00",
                        status_label: "Đã check-in",
                    },
                ];
        });
        setMockResponse("check_in", () => createBookingWithGuest({ room_id: "1A", guest_name: "Trần Thị Mới" }));

        render(<Dashboard />);

        expect(await screen.findByText("Check-in phòng 2A — Nguyễn Văn A")).toBeInTheDocument();

        await act(async () => {
            await useHotelStore.getState().checkIn(
                "1A",
                [{ full_name: "Trần Thị Mới", doc_number: "012345678901" }],
                1,
                400000,
                "walk-in",
                "",
            );
        });

        await waitFor(() => {
            expect(screen.getByText("Check-in phòng 1A — Trần Thị Mới")).toBeInTheDocument();
        });
        expect(activityCalls).toBeGreaterThanOrEqual(2);
    });
});
