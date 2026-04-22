import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const invokeCommand = vi.hoisted(() => vi.fn());
const createCorrelationId = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke,
}));

vi.mock("@/lib/invokeCommand", () => ({
  invokeCommand,
}));

vi.mock("@/lib/correlationId", () => ({
  createCorrelationId,
}));

import { useHotelStore } from "./useHotelStore";

describe("useHotelStore monitoring context", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    createCorrelationId.mockReturnValue("COR-1A2B3C4D");
    invokeCommand.mockResolvedValue(undefined);
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_rooms") {
        return [];
      }

      if (command === "get_dashboard_stats") {
        return {
          total_rooms: 10,
          occupied: 2,
          vacant: 8,
          cleaning: 0,
          revenue_today: 0,
        };
      }

      throw new Error(`Unhandled invoke ${command}`);
    });
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
  });

  it("passes scrubbed monitoring context for checkIn", async () => {
    await useHotelStore.getState().checkIn(
      "101",
      [
        { full_name: "Nguyen Van A", doc_number: "012345678901" },
        { full_name: "Tran Thi B", doc_number: "109876543210" },
      ],
      3,
      500000,
      "walk-in",
      "Late arrival",
    );

    expect(invokeCommand).toHaveBeenCalledWith(
      "check_in",
      {
        req: {
          room_id: "101",
          guests: [
            { full_name: "Nguyen Van A", doc_number: "012345678901" },
            { full_name: "Tran Thi B", doc_number: "109876543210" },
          ],
          nights: 3,
          source: "walk-in",
          notes: "Late arrival",
          paid_amount: 500000,
        },
      },
      {
        correlationId: "COR-1A2B3C4D",
        monitoringContext: {
          guest_count: 2,
          nights: 3,
          source: "walk-in",
          notes_present: true,
        },
      },
    );
  });

  it("normalizes omitted checkIn source to null in monitoring context", async () => {
    await useHotelStore.getState().checkIn(
      "101",
      [{ full_name: "Nguyen Van A", doc_number: "012345678901" }],
      1,
      250000,
      undefined,
      "",
    );

    expect(invokeCommand).toHaveBeenCalledWith(
      "check_in",
      {
        req: {
          room_id: "101",
          guests: [{ full_name: "Nguyen Van A", doc_number: "012345678901" }],
          nights: 1,
          source: undefined,
          notes: "",
          paid_amount: 250000,
        },
      },
      {
        correlationId: "COR-1A2B3C4D",
        monitoringContext: {
          guest_count: 1,
          nights: 1,
          source: null,
          notes_present: false,
        },
      },
    );
  });

  it("passes scrubbed monitoring context for checkOut", async () => {
    await useHotelStore.getState().checkOut("booking-1", "hourly", 400000);

    expect(invokeCommand).toHaveBeenCalledWith(
      "check_out",
      {
        req: {
          booking_id: "booking-1",
          settlement_mode: "hourly",
          final_total: 400000,
        },
      },
      {
        correlationId: "COR-1A2B3C4D",
        monitoringContext: {
          settlement_mode: "hourly",
        },
      },
    );
  });
});
