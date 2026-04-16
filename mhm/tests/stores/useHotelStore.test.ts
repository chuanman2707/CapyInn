import { describe, expect, it, beforeEach } from "vitest";
import { useHotelStore } from "@/stores/useHotelStore";
import { invoke, clearMockResponses, setMockResponse } from "@test-mocks/tauri-core";
import { createAllRooms, createStats } from "../helpers/mock-data";

describe("useHotelStore", () => {
    beforeEach(() => {
        clearMockResponses();
        invoke.mockClear();
    });

    it("has expected initial state", () => {
        const state = useHotelStore.getInitialState();
        expect(state.rooms).toEqual([]);
        expect(state.stats).toBeNull();
        expect(state.activeTab).toBe("dashboard");
        expect(state.loading).toBe(false);
        expect(state.isCheckinOpen).toBe(false);
        expect(state.checkinRoomId).toBeNull();
        expect(state.isGroupCheckinOpen).toBe(false);
        expect(state.groups).toEqual([]);
        expect(state.housekeepingTasks).toEqual([]);
        expect(state.roomDetail).toBeNull();
    });

    it("fetchRooms updates the rooms state", async () => {
        const mockRooms = createAllRooms();
        setMockResponse("get_rooms", () => mockRooms);

        await useHotelStore.getState().fetchRooms();

        expect(invoke).toHaveBeenCalledWith("get_rooms");
        expect(useHotelStore.getState().rooms).toEqual(mockRooms);
    });

    it("fetchStats updates the stats state", async () => {
        const mockStats = createStats();
        setMockResponse("get_dashboard_stats", () => mockStats);

        await useHotelStore.getState().fetchStats();

        expect(invoke).toHaveBeenCalledWith("get_dashboard_stats");
        expect(useHotelStore.getState().stats).toEqual(mockStats);
    });

    it("checkIn executes successfully and updates data", async () => {
        // We expect checkIn to call `check_in`, `get_rooms`, and `get_dashboard_stats`
        setMockResponse("check_in", () => undefined);
        setMockResponse("get_rooms", () => createAllRooms());
        setMockResponse("get_dashboard_stats", () => createStats());

        const guests = [{ full_name: "John Doe", doc_number: "123456" }];
        await useHotelStore.getState().checkIn("1A", guests, 2, 500000, "walk-in", "test note");

        expect(invoke).toHaveBeenCalledWith("check_in", {
            req: {
                room_id: "1A",
                guests,
                nights: 2,
                source: "walk-in",
                notes: "test note",
                paid_amount: 500000,
            },
        });
        expect(invoke).toHaveBeenCalledWith("get_rooms");
        expect(invoke).toHaveBeenCalledWith("get_dashboard_stats");
        expect(useHotelStore.getState().loading).toBe(false);
        expect(useHotelStore.getState().activeTab).toBe("dashboard");
    });

    it("checkIn handles errors correctly", async () => {
        const mockError = new Error("Check-in failed");
        setMockResponse("check_in", () => {
            throw mockError;
        });

        const guests = [{ full_name: "Jane Doe", doc_number: "654321" }];

        await expect(
            useHotelStore.getState().checkIn("1A", guests, 1)
        ).rejects.toThrow("Check-in failed");

        expect(useHotelStore.getState().loading).toBe(false);
    });

    it("checkOut executes successfully and updates data", async () => {
        setMockResponse("check_out", () => undefined);
        setMockResponse("get_rooms", () => createAllRooms());
        setMockResponse("get_dashboard_stats", () => createStats());

        await useHotelStore.getState().checkOut("booking-1", 100000);

        expect(invoke).toHaveBeenCalledWith("check_out", {
            req: {
                booking_id: "booking-1",
                final_paid: 100000,
            },
        });
        expect(invoke).toHaveBeenCalledWith("get_rooms");
        expect(invoke).toHaveBeenCalledWith("get_dashboard_stats");
        expect(useHotelStore.getState().loading).toBe(false);
        expect(useHotelStore.getState().activeTab).toBe("dashboard");
    });

    it("checkOut handles errors correctly", async () => {
        const mockError = new Error("Check-out failed");
        setMockResponse("check_out", () => {
            throw mockError;
        });

        await expect(
            useHotelStore.getState().checkOut("booking-1")
        ).rejects.toThrow("Check-out failed");

        expect(useHotelStore.getState().loading).toBe(false);
    });
});
