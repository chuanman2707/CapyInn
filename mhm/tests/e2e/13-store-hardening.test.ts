import { beforeEach, describe, expect, it } from "vitest";
import { clearMockResponses, invoke, setMockResponse } from "@test-mocks/tauri-core";
import { useHotelStore } from "@/stores/useHotelStore";
import { useAuthStore } from "@/stores/useAuthStore";
import { createAllRooms, createBooking, createStats } from "../helpers/mock-data";

function deferred<T>() {
    let resolve!: (value: T) => void;
    let reject!: (reason?: unknown) => void;
    const promise = new Promise<T>((res, rej) => {
        resolve = res;
        reject = rej;
    });
    return { promise, resolve, reject };
}

describe("13 — Store Hardening", () => {
    beforeEach(() => {
        clearMockResponses();
        invoke.mockClear();
        useHotelStore.setState({
            rooms: [],
            stats: null,
            roomDetail: null,
            activeTab: "dashboard",
            housekeepingTasks: [],
            loading: false,
            isCheckinOpen: false,
            checkinRoomId: null,
            isGroupCheckinOpen: false,
            groups: [],
        });

        setMockResponse("get_rooms", () => createAllRooms());
        setMockResponse("get_dashboard_stats", () => createStats());
        useAuthStore.setState({
            user: null,
            isAuthenticated: false,
            loading: false,
            error: null,
        });
    });

    it("keeps loading true while another booking action is still pending", async () => {
        const pendingCheckIn = deferred<ReturnType<typeof createBooking>>();

        setMockResponse("check_in", () => pendingCheckIn.promise);
        setMockResponse("check_out", () => undefined);

        const checkInPromise = useHotelStore.getState().checkIn(
            "1A",
            [{ full_name: "Nguyễn Văn A", doc_number: "012345678901" }],
            1
        );

        expect(useHotelStore.getState().loading).toBe(true);

        await useHotelStore.getState().checkOut("booking-1", "actual_nights", 500000);

        expect(useHotelStore.getState().loading).toBe(true);

        pendingCheckIn.resolve(createBooking({ room_id: "1A" }));
        await checkInPromise;

        expect(useHotelStore.getState().loading).toBe(false);
    });

    it("clears stale auth state when session lookup returns null or throws", async () => {
        useAuthStore.setState({
            user: {
                id: "u1",
                name: "Admin",
                role: "admin",
                active: true,
                created_at: "2026-01-01T00:00:00Z",
            },
            isAuthenticated: true,
            loading: false,
            error: null,
        });

        setMockResponse("get_current_user", () => null);

        await useAuthStore.getState().checkSession();

        expect(useAuthStore.getState().user).toBeNull();
        expect(useAuthStore.getState().isAuthenticated).toBe(false);

        useAuthStore.setState({
            user: {
                id: "u2",
                name: "Admin 2",
                role: "admin",
                active: true,
                created_at: "2026-01-01T00:00:00Z",
            },
            isAuthenticated: true,
            loading: false,
            error: null,
        });

        setMockResponse("get_current_user", () => {
            throw new Error("boom");
        });

        await useAuthStore.getState().checkSession();

        expect(useAuthStore.getState().user).toBeNull();
        expect(useAuthStore.getState().isAuthenticated).toBe(false);
    });
});
