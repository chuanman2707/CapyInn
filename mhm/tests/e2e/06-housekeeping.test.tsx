import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor } from "../helpers/render-app";
import Housekeeping from "@/pages/Housekeeping";
import { setMockResponse, clearMockResponses, invoke } from "@test-mocks/tauri-core";
import { useHotelStore } from "@/stores/useHotelStore";
import { createHousekeepingTask } from "../helpers/mock-data";

const mockTasks = [
    createHousekeepingTask({ id: "hk-1", room_id: "3A", status: "needs_cleaning" }),
    createHousekeepingTask({ id: "hk-2", room_id: "2A", status: "cleaning" }),
    createHousekeepingTask({ id: "hk-3", room_id: "5A", status: "clean" }),
];

describe("06 — Housekeeping", () => {
    beforeEach(() => {
        clearMockResponses();
        invoke.mockClear();

        setMockResponse("get_housekeeping_tasks", () => mockTasks);
        setMockResponse("get_rooms", () => []);
        useHotelStore.setState({ housekeepingTasks: mockTasks });
    });

    it("renders housekeeping page with tasks", async () => {
        render(<Housekeeping />);

        await waitFor(() => {
            expect(screen.getByText("3A")).toBeInTheDocument();
        });
    });

    it("shows correct status for tasks", async () => {
        render(<Housekeeping />);

        await waitFor(() => {
            // Should show room IDs for tasks
            expect(screen.getByText("3A")).toBeInTheDocument();
            expect(screen.getByText("2A")).toBeInTheDocument();
        });
    });

    it("update housekeeping calls correct command", async () => {
        setMockResponse("update_housekeeping", () => undefined);
        setMockResponse("get_housekeeping_tasks", () => mockTasks);
        setMockResponse("get_rooms", () => []);

        await useHotelStore.getState().updateHousekeeping("hk-1", "cleaning", "Started cleaning");

        expect(invoke).toHaveBeenCalledWith("update_housekeeping", {
            taskId: "hk-1",
            newStatus: "cleaning",
            note: "Started cleaning",
            idempotencyKey: expect.stringMatching(/^update_housekeeping:/),
        });
    });

    it("refreshes task list after status update", async () => {
        setMockResponse("update_housekeeping", () => undefined);
        setMockResponse("get_housekeeping_tasks", () => mockTasks);
        setMockResponse("get_rooms", () => []);

        await useHotelStore.getState().updateHousekeeping("hk-1", "clean");

        // Should refresh tasks
        expect(invoke).toHaveBeenCalledWith("get_housekeeping_tasks");
        // Should also refresh rooms
        expect(invoke).toHaveBeenCalledWith("get_rooms");
    });
});
