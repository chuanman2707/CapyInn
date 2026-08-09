import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { render, screen, waitFor } from "../helpers/render-app";
import userEvent from "@testing-library/user-event";
import App from "@/App";
import { APP_LOGO_ALT } from "@/lib/appIdentity";
import { setMockResponse, clearMockResponses, invoke } from "@test-mocks/tauri-core";
import { useAuthStore } from "@/stores/useAuthStore";
import { useHotelStore } from "@/stores/useHotelStore";
import { createAllRooms, createStats, createUser } from "../helpers/mock-data";

const mockUser = createUser({ id: "u1", name: "Admin", role: "admin" });
const mockRooms = createAllRooms();
const mockStats = createStats();

function setupAuthenticatedState() {
    useAuthStore.setState({
        user: mockUser,
        isAuthenticated: true,
        loading: false,
        error: null,
    });
    useHotelStore.setState({
        activeTab: "dashboard",
        rooms: [],
        stats: null,
        roomDetail: null,
        loading: false,
        isCheckinOpen: false,
    });

    setMockResponse("get_rooms", () => mockRooms);
    setMockResponse("get_dashboard_stats", () => mockStats);
    setMockResponse("get_current_user", () => mockUser);
    setMockResponse("get_settings", () => null);
    setMockResponse("get_recent_activity", () => []);
    setMockResponse("get_revenue_stats", () => ({ total_revenue: 0, rooms_sold: 0, occupancy_rate: 0, daily_revenue: [] }));
    setMockResponse("get_expenses", () => []);
    setMockResponse("get_all_bookings", () => []);
    setMockResponse("get_rooms_availability", () => []);
    setMockResponse("gateway_get_status", () => ({ running: false }));
}

describe("12 — Navigation & Layout", () => {
    let originalInnerWidth: number;

    beforeEach(() => {
        clearMockResponses();
        invoke.mockClear();

        // Prevent sidebar auto-collapse: JSDOM defaults innerWidth to 0 which triggers collapse
        originalInnerWidth = window.innerWidth;
        Object.defineProperty(window, "innerWidth", { value: 1400, writable: true, configurable: true });

        // Ensure sidebar starts expanded
        localStorage.setItem("sidebar-collapsed", "false");

        setupAuthenticatedState();
    });

    afterEach(() => {
        Object.defineProperty(window, "innerWidth", { value: originalInnerWidth, writable: true, configurable: true });
        localStorage.removeItem("sidebar-collapsed");
    });

    it("renders sidebar with all nav items", async () => {
        render(<App />);

        await waitFor(() => {
            expect(screen.getByText("Dashboard")).toBeInTheDocument();
        });

        expect(screen.getByAltText(APP_LOGO_ALT)).toBeInTheDocument();

        // Some labels may appear in multiple places (sidebar + page title), use getAllByText
        expect(screen.getAllByText("Reservations").length).toBeGreaterThanOrEqual(1);
        expect(screen.getAllByText("Rooms").length).toBeGreaterThanOrEqual(1);
        expect(screen.getAllByText("Guests").length).toBeGreaterThanOrEqual(1);
        expect(screen.queryByText("Housekeeping")).toBeNull();
        expect(screen.getAllByText("Analytics").length).toBeGreaterThanOrEqual(1);
        expect(screen.getAllByText("Night Audit").length).toBeGreaterThanOrEqual(1);
        expect(screen.getAllByText("Settings").length).toBeGreaterThanOrEqual(1);
    });

    it("shows current user info in sidebar", async () => {
        render(<App />);

        await waitFor(() => {
            expect(screen.getByText("Admin")).toBeInTheDocument();
        });

        expect(screen.getByText("admin")).toBeInTheDocument();
    });

    it("clicking nav item changes active page", async () => {
        render(<App />);
        const user = userEvent.setup();

        await waitFor(() => {
            expect(screen.getByText("Dashboard")).toBeInTheDocument();
        });

        // Click Rooms nav
        await user.click(screen.getByText("Rooms"));

        // Header should change to "Rooms"
        await waitFor(() => {
            expect(useHotelStore.getState().activeTab).toBe("rooms");
        });
    });

    it("sidebar collapse toggle works", async () => {
        render(<App />);
        const user = userEvent.setup();

        // Should show "Thu gọn" text
        await waitFor(() => {
            expect(screen.getByText("Thu gọn")).toBeInTheDocument();
        });

        // Click collapse button
        await user.click(screen.getByText("Thu gọn"));

        // After collapse, "Thu gọn" should not be visible
        await waitFor(() => {
            expect(screen.queryByText("Thu gọn")).not.toBeInTheDocument();
        });
    });

    it("logout button redirects to login screen", async () => {
        setMockResponse("logout", () => undefined);
        render(<App />);
        const user = userEvent.setup();

        await waitFor(() => {
            expect(screen.getByText("Admin")).toBeInTheDocument();
        });

        // Click logout button (has title="Đăng xuất")
        const logoutBtn = screen.getByTitle("Đăng xuất");
        await user.click(logoutBtn);

        // Should see login screen
        await waitFor(() => {
            expect(screen.getByText(/Nhập mã PIN/)).toBeInTheDocument();
        });
    });

    it("shows + Khách mới button in header", async () => {
        render(<App />);

        await waitFor(() => {
            expect(screen.getByText("+ Khách mới")).toBeInTheDocument();
        });
    });

    it("shows role badge (Admin)", async () => {
        render(<App />);

        await waitFor(() => {
            // "Admin" appears in multiple places (sidebar user info + role badge)
            expect(screen.getAllByText(/Admin/).length).toBeGreaterThanOrEqual(1);
        });
    });
});
