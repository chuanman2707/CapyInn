import { describe, it, expect, beforeEach } from "vitest";
import { setMockResponse, clearMockResponses, invoke } from "@test-mocks/tauri-core";
import { useHotelStore } from "@/stores/useHotelStore";
import { createAllRooms, createStats } from "../helpers/mock-data";

const mockRooms = createAllRooms();
const mockStats = createStats();

describe("05 — Check-out Flow", () => {
    beforeEach(() => {
        clearMockResponses();
        invoke.mockClear();
        useHotelStore.setState({
            rooms: mockRooms,
            stats: mockStats,
            activeTab: "dashboard",
            roomDetail: null,
            housekeepingTasks: [],
            loading: false,
            isCheckinOpen: false,
        });

        setMockResponse("get_rooms", () => mockRooms);
        setMockResponse("get_dashboard_stats", () => mockStats);
    });

    it("check_out calls correct invoke command", async () => {
        setMockResponse("check_out", () => undefined);

        await useHotelStore.getState().checkOut("booking-1", "hourly", 400000);

        const checkOutCall = invoke.mock.calls.find(([command]) => command === "check_out");
        expect(checkOutCall).toBeDefined();

        expect(checkOutCall?.[1]).toMatchObject({
            req: { booking_id: "booking-1", settlement_mode: "hourly", final_total: 400000 },
            correlationId: expect.stringMatching(/^COR-[0-9A-F]{8}$/),
        });
        expect(checkOutCall?.[1]).not.toHaveProperty("monitoringContext");
    });

    it("refreshes rooms and stats after checkout", async () => {
        setMockResponse("check_out", () => undefined);

        await useHotelStore.getState().checkOut("booking-1", "actual_nights", 500000);

        // Should refresh data
        expect(invoke).toHaveBeenCalledWith("get_rooms");
        expect(invoke).toHaveBeenCalledWith("get_dashboard_stats");
    });

    it("navigates to dashboard after checkout", async () => {
        setMockResponse("check_out", () => undefined);

        useHotelStore.setState({ activeTab: "rooms" });
        await useHotelStore.getState().checkOut("booking-1", "actual_nights", 500000);

        expect(useHotelStore.getState().activeTab).toBe("dashboard");
    });

    it("handles checkout error", async () => {
        setMockResponse("check_out", () => {
            throw {
                code: "BOOKING_NOT_FOUND",
                message: "Booking not found",
                kind: "user",
                support_id: null,
            };
        });

        const promise = useHotelStore
            .getState()
            .checkOut("nonexistent", "actual_nights", 500000);

        await expect(promise).rejects.toMatchObject({
            name: "AppError",
            code: "BOOKING_NOT_FOUND",
            message: "Booking not found",
            kind: "user",
            support_id: null,
            correlation_id: expect.stringMatching(/^COR-[0-9A-F]{8}$/),
        });

        const checkOutCall = invoke.mock.calls.find(([command]) => command === "check_out");
        const correlationId = checkOutCall?.[1]?.correlationId;
        expect(correlationId).toMatch(/^COR-[0-9A-F]{8}$/);
        expect(checkOutCall?.[1]).not.toHaveProperty("monitoringContext");

        await promise.catch((error) => {
            expect(error.correlation_id).toBe(correlationId);
        });

        expect(useHotelStore.getState().loading).toBe(false);
    });

    it("checkout sends the requested settlement payload", async () => {
        setMockResponse("check_out", () => undefined);

        await useHotelStore.getState().checkOut("booking-1", "booked_nights", 2500000);

        const checkOutCall = invoke.mock.calls.find(([command]) => command === "check_out");
        expect(checkOutCall?.[1]).toMatchObject({
            req: { booking_id: "booking-1", settlement_mode: "booked_nights", final_total: 2500000 },
            correlationId: expect.stringMatching(/^COR-[0-9A-F]{8}$/),
        });
        expect(checkOutCall?.[1]).not.toHaveProperty("monitoringContext");
    });
});
