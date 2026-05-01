import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, waitFor } from "../helpers/render-app";
import RoomDetailPanel from "@/components/RoomDetailPanel";
import { setMockResponse, clearMockResponses, invoke } from "@test-mocks/tauri-core";
import { useHotelStore } from "@/stores/useHotelStore";
import { createBooking, createGuest, createRoomWithBooking } from "../helpers/mock-data";

const occupiedRoomDetail = createRoomWithBooking({
    room: { id: "2A", name: "2A", type: "deluxe", status: "occupied", floor: 2, has_balcony: true, base_price: 400000 },
    booking: { id: "b1", room_id: "2A", total_price: 400000, paid_amount: 400000, nights: 1, status: "active" },
    guests: [createGuest({ id: "g1", full_name: "Nguyễn Văn A", doc_number: "012345678901" })],
});

const vacantRoomDetail = createRoomWithBooking({
    room: { id: "1A", name: "1A", type: "deluxe", status: "vacant", floor: 1, has_balcony: true, base_price: 400000 },
    booking: null,
    guests: [],
});

describe("04 — Room Detail", () => {
    beforeEach(() => {
        clearMockResponses();
        invoke.mockClear();
        setMockResponse("get_settings", () => null);
        setMockResponse("get_folio_lines", () => []);
        setMockResponse("get_room_detail", () => occupiedRoomDetail);
    });

    it("renders room info for occupied room", async () => {
        render(<RoomDetailPanel mode="page" roomDetail={occupiedRoomDetail} />);

        await waitFor(() => {
            expect(screen.getByText("2A")).toBeInTheDocument();
        });
    });

    it("shows guest info when room is occupied", async () => {
        render(<RoomDetailPanel mode="page" roomDetail={occupiedRoomDetail} />);

        await waitFor(() => {
            expect(screen.getByText("Nguyễn Văn A")).toBeInTheDocument();
        });
    });

    it("shows empty state for vacant room", async () => {
        render(<RoomDetailPanel mode="page" roomDetail={vacantRoomDetail} />);

        await waitFor(() => {
            expect(screen.getByText("1A")).toBeInTheDocument();
        });

        // Should NOT show guest name
        expect(screen.queryByText("Nguyễn Văn A")).not.toBeInTheDocument();
    });

    it("calls get_stay_info_text through store", async () => {
        setMockResponse("get_stay_info_text", () => "Nguyễn Văn A — CCCD: 012345678901 — Phòng 2A");

        const result = await useHotelStore.getState().getStayInfoText("b1");

        expect(invoke).toHaveBeenCalledWith("get_stay_info_text", { bookingId: "b1" });
        expect(result).toContain("Nguyễn Văn A");
    });

    it("extend stay calls correct command", async () => {
        const extendedBooking = createBooking({ id: "b1", nights: 2 });
        setMockResponse("extend_stay", () => extendedBooking);

        await useHotelStore.getState().extendStay("b1");

        expect(invoke).toHaveBeenCalledWith("extend_stay", {
            bookingId: "b1",
            correlationId: expect.stringMatching(/^COR-[0-9A-F]{8}$/),
            idempotencyKey: expect.stringMatching(/^extend_stay:/),
        });
    });

    it("refreshes rooms and stats after extending stay", async () => {
        const refreshedRooms = [vacantRoomDetail.room];
        const refreshedStats = { total_rooms: 10, occupied: 1, vacant: 9, cleaning: 0, revenue_today: 800000 };

        setMockResponse("extend_stay", () => createBooking({ id: "b1", nights: 2 }));
        setMockResponse("get_rooms", () => refreshedRooms);
        setMockResponse("get_dashboard_stats", () => refreshedStats);

        await useHotelStore.getState().extendStay("b1");

        expect(invoke).toHaveBeenCalledWith("get_rooms");
        expect(invoke).toHaveBeenCalledWith("get_dashboard_stats");
        expect(useHotelStore.getState().rooms).toEqual(refreshedRooms);
        expect(useHotelStore.getState().stats).toEqual(refreshedStats);
    });
});
