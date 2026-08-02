import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const invokeCommand = vi.hoisted(() => vi.fn());
const invokeWriteCommand = vi.hoisted(() => vi.fn());
const createIdempotencyKey = vi.hoisted(() => vi.fn());
const createCorrelationId = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke,
}));

vi.mock("@/lib/invokeCommand", () => ({
  createIdempotencyKey,
  invokeCommand,
  invokeWriteCommand,
}));

vi.mock("@/lib/correlationId", () => ({
  createCorrelationId,
}));

import { useHotelStore } from "./useHotelStore";

describe("useHotelStore monitoring context", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.spyOn(console, "error").mockImplementation(() => {});
    createCorrelationId.mockReturnValue("COR-1A2B3C4D");
    createIdempotencyKey.mockReturnValue("group_checkin:IDEM-1");
    invokeCommand.mockResolvedValue(undefined);
    invokeWriteCommand.mockResolvedValue(undefined);
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_rooms") {
        return [];
      }

      if (command === "get_housekeeping_tasks") {
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

      if (command === "get_all_groups") {
        return [];
      }

      throw new Error(`Unhandled invoke ${command}`);
    });
    useHotelStore.setState({
      rooms: [],
      stats: null,
      dashboardRefreshVersion: 0,
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

  afterEach(() => {
    vi.restoreAllMocks();
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

    expect(invokeWriteCommand).toHaveBeenCalledWith(
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

    expect(invokeWriteCommand).toHaveBeenCalledWith(
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

    expect(invokeWriteCommand).toHaveBeenCalledWith(
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

  it("routes extendStay through invokeWriteCommand with monitoring context", async () => {
    await useHotelStore.getState().extendStay("booking-extend-1");

    expect(invokeWriteCommand).toHaveBeenCalledWith(
      "extend_stay",
      { bookingId: "booking-extend-1" },
      {
        correlationId: "COR-1A2B3C4D",
        monitoringContext: {
          operation: "add_one_night",
        },
      },
    );
    expect(invoke).not.toHaveBeenCalledWith("extend_stay", expect.anything());
  });

  it("passes an idempotency key for groupCheckIn", async () => {
    const req = {
      group_name: "Retry Group",
      organizer_name: "Organizer",
      room_ids: ["101", "102"],
      master_room_id: "101",
      guests_per_room: {},
      nights: 1,
      source: "walk-in",
      paid_amount: 100000,
    };

    await useHotelStore.getState().groupCheckIn(req);

    expect(createIdempotencyKey).not.toHaveBeenCalled();
    expect(invokeWriteCommand).toHaveBeenCalledWith(
      "group_checkin",
      {
        req,
      },
      {
        correlationId: "COR-1A2B3C4D",
      },
    );
    expect(invokeCommand).not.toHaveBeenCalledWith(
      "group_checkin",
      expect.anything(),
      expect.anything(),
    );
  });

  it("routes groupCheckout through invokeWriteCommand", async () => {
    const req = {
      group_id: "group-1",
      booking_ids: ["booking-1"],
      final_paid: 100000,
    };

    await useHotelStore.getState().groupCheckout(req);

    expect(invokeWriteCommand).toHaveBeenCalledWith(
      "group_checkout",
      { req },
      {
        correlationId: "COR-1A2B3C4D",
      },
    );
    expect(invokeCommand).not.toHaveBeenCalledWith(
      "group_checkout",
      expect.anything(),
      expect.anything(),
    );
  });

  it("routes addGroupService through invokeWriteCommand with guarded money", async () => {
    const service = {
      id: "svc-1",
      group_id: "group-1",
      name: "Laundry",
      quantity: 2,
      unit_price: 50000,
      total_amount: 100000,
      created_at: "2026-01-01T00:00:00Z",
    };
    invokeWriteCommand.mockResolvedValueOnce(service);

    const result = await useHotelStore.getState().addGroupService({
      group_id: "group-1",
      name: "Laundry",
      quantity: 2,
      unit_price: 50000,
    });

    expect(result).toBe(service);
    expect(invokeWriteCommand).toHaveBeenCalledWith("add_group_service", {
      req: {
        group_id: "group-1",
        name: "Laundry",
        quantity: 2,
        unit_price: 50000,
      },
    });
    expect(invoke).not.toHaveBeenCalledWith(
      "add_group_service",
      expect.anything(),
    );
  });

  it("routes removeGroupService through invokeWriteCommand", async () => {
    await useHotelStore.getState().removeGroupService("svc-1");

    expect(invokeWriteCommand).toHaveBeenCalledWith("remove_group_service", {
      serviceId: "svc-1",
    });
    expect(invoke).not.toHaveBeenCalledWith(
      "remove_group_service",
      expect.anything(),
    );
  });

  it("routes updateHousekeeping through invokeWriteCommand", async () => {
    await useHotelStore.getState().updateHousekeeping("task-1", "cleaning", "Started");

    expect(invokeWriteCommand).toHaveBeenCalledWith("update_housekeeping", {
      taskId: "task-1",
      newStatus: "cleaning",
      note: "Started",
    });
    expect(invoke).not.toHaveBeenCalledWith(
      "update_housekeeping",
      expect.anything(),
    );
  });

  it("rejects fractional checkIn paid_amount before invoking backend", async () => {
    await expect(
      useHotelStore.getState().checkIn(
        "101",
        [{ full_name: "Nguyen Van A", doc_number: "012345678901" }],
        1,
        100000.5,
      ),
    ).rejects.toThrow(/paid_amount/);

    expect(invokeWriteCommand).not.toHaveBeenCalledWith(
      "check_in",
      expect.anything(),
      expect.anything(),
    );
  });

  it("rejects fractional checkOut final_total before invoking backend", async () => {
    await expect(
      useHotelStore.getState().checkOut("booking-1", "hourly", 400000.5),
    ).rejects.toThrow(/final_total/);

    expect(invokeWriteCommand).not.toHaveBeenCalledWith(
      "check_out",
      expect.anything(),
      expect.anything(),
    );
  });

  it("rejects fractional group money before invoking backend", async () => {
    await expect(
      useHotelStore.getState().groupCheckIn({
        group_name: "Retry Group",
        organizer_name: "Organizer",
        room_ids: ["101", "102"],
        master_room_id: "101",
        guests_per_room: {},
        nights: 1,
        paid_amount: 100000.5,
      }),
    ).rejects.toThrow(/paid_amount/);

    await expect(
      useHotelStore.getState().groupCheckout({
        group_id: "group-1",
        booking_ids: ["booking-1"],
        final_paid: 100000.5,
      }),
    ).rejects.toThrow(/final_paid/);

    expect(invokeCommand).not.toHaveBeenCalledWith(
      "group_checkin",
      expect.anything(),
      expect.anything(),
    );
    expect(invokeCommand).not.toHaveBeenCalledWith(
      "group_checkout",
      expect.anything(),
      expect.anything(),
    );
    expect(invokeWriteCommand).not.toHaveBeenCalledWith(
      "group_checkout",
      expect.anything(),
      expect.anything(),
    );
  });

  it("rejects fractional group service unit_price before invoking backend", async () => {
    await expect(
      useHotelStore.getState().addGroupService({
        group_id: "group-1",
        name: "Laundry",
        quantity: 1,
        unit_price: 50000.5,
      }),
    ).rejects.toThrow(/unit_price/);

    expect(invoke).not.toHaveBeenCalledWith(
      "add_group_service",
      expect.anything(),
    );
    expect(invokeWriteCommand).not.toHaveBeenCalledWith(
      "add_group_service",
      expect.anything(),
    );
  });
});

describe("useHotelStore navigation side effects", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.spyOn(console, "error").mockImplementation(() => {});
    createCorrelationId.mockReturnValue("COR-1A2B3C4D");
    invokeWriteCommand.mockResolvedValue(undefined);
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_rooms") return [];
      if (command === "get_housekeeping_tasks") return [];
      if (command === "get_dashboard_stats") {
        return { total_rooms: 10, occupied: 2, vacant: 8, cleaning: 0, revenue_today: 0 };
      }
      throw new Error(`Unhandled invoke ${command}`);
    });
    useHotelStore.setState({
      rooms: [],
      stats: null,
      dashboardRefreshVersion: 0,
      activeTab: "reservations",
      loading: false,
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("stays on the current tab after check-out and still bumps the dashboard version", async () => {
    await useHotelStore.getState().checkOut("booking-1", "actual_nights", 500000);

    expect(useHotelStore.getState().activeTab).toBe("reservations");
    expect(useHotelStore.getState().dashboardRefreshVersion).toBe(1);
  });

  it("stays on the current tab after check-in", async () => {
    await useHotelStore.getState().checkIn(
      "101",
      [{ full_name: "Nguyen Van A", doc_number: "012345678901" }],
      1,
      500000,
      "walk-in",
      undefined,
    );

    expect(useHotelStore.getState().activeTab).toBe("reservations");
    expect(useHotelStore.getState().dashboardRefreshVersion).toBe(1);
  });
});

describe("useHotelStore room type rates", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useHotelStore.setState({ rooms: [], roomTypeRates: null });
  });

  it("loads the type rates alongside the rooms they annotate", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_rooms") return [{ id: "R-101", type: "Phòng Đôi" }];
      if (command === "get_room_type_rates")
        return [{ room_type: "Phòng Đôi", nightly_rate: 640000, configured: true }];
      throw new Error(`unexpected ${command}`);
    });

    await useHotelStore.getState().fetchRooms();

    // Keyed on `rooms.type`, because that is what every card looks up by.
    expect(useHotelStore.getState().roomTypeRates?.["Phòng Đôi"]?.nightly_rate).toBe(640000);
  });

  it("still shows the rooms when the rate lookup fails", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_rooms") return [{ id: "R-101", type: "Phòng Đôi" }];
      throw new Error("db locked");
    });

    // Không throw: mất giá thì màn hình phòng vẫn phải dùng được, chỉ là không
    // có số để in. `fetchRooms` được gọi sau mọi lệnh ghi nên nó không được vỡ.
    await useHotelStore.getState().fetchRooms();

    expect(useHotelStore.getState().rooms).toHaveLength(1);
    expect(useHotelStore.getState().roomTypeRates).toBeNull();
  });

  it("clears the rates it had when a later lookup fails", async () => {
    useHotelStore.setState({
      roomTypeRates: {
        "Phòng Đôi": { room_type: "Phòng Đôi", nightly_rate: 640000, configured: true },
      },
    });
    invoke.mockRejectedValue(new Error("db locked"));

    await useHotelStore.getState().fetchRoomTypeRates();

    // Giá cũ còn lại thì thẻ phòng in giá của một bảng giá đã đổi — tệ hơn "—".
    expect(useHotelStore.getState().roomTypeRates).toBeNull();
  });
});

describe("useHotelStore room change", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.spyOn(console, "error").mockImplementation(() => {});
    createCorrelationId.mockReturnValue("COR-1A2B3C4D");
    invokeWriteCommand.mockResolvedValue(undefined);
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_rooms") return [];
      if (command === "get_dashboard_stats") {
        return { total_rooms: 10, occupied: 2, vacant: 8, cleaning: 0, revenue_today: 0 };
      }
      throw new Error(`Unhandled invoke ${command}`);
    });
    useHotelStore.setState({
      rooms: [],
      stats: null,
      dashboardRefreshVersion: 0,
      loading: false,
      isRoomChangeOpen: false,
      roomChangeBookingId: null,
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("gửi change_room kèm đủ tham số rồi nạp lại phòng", async () => {
    await useHotelStore.getState().changeRoom("B1", "2B", false, "máy lạnh hỏng");

    expect(invokeWriteCommand).toHaveBeenCalledWith(
      "change_room",
      expect.objectContaining({
        bookingId: "B1",
        newRoomId: "2B",
        keepPrice: false,
        reason: "máy lạnh hỏng",
      }),
      expect.objectContaining({
        correlationId: "COR-1A2B3C4D",
      }),
    );
    expect(invoke).toHaveBeenCalledWith("get_rooms");
  });

  it("setRoomChangeOpen mở sheet cho đúng booking", () => {
    useHotelStore.getState().setRoomChangeOpen(true, "B1");
    expect(useHotelStore.getState().isRoomChangeOpen).toBe(true);
    expect(useHotelStore.getState().roomChangeBookingId).toBe("B1");

    useHotelStore.getState().setRoomChangeOpen(false);
    expect(useHotelStore.getState().isRoomChangeOpen).toBe(false);
    expect(useHotelStore.getState().roomChangeBookingId).toBeNull();
  });

  it("fetchRoomChangeOptions đọc get_room_change_options và giữ nguyên priceDifference âm", async () => {
    const options = {
      bookingId: "B1",
      currentRoomId: "1A",
      currentRoomName: "Phòng 1A",
      fromDate: "2026-08-02",
      toDate: "2026-08-05",
      nightsRemaining: 3,
      nightsStayed: 0,
      guestCount: 2,
      rooms: [
        {
          roomId: "2B",
          name: "Phòng 2B",
          roomType: "Standard",
          floor: 2,
          maxGuests: 2,
          priceDifference: -150000,
        },
      ],
    };
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_room_change_options") return options;
      throw new Error(`Unhandled invoke ${command}`);
    });

    const result = await useHotelStore.getState().fetchRoomChangeOptions("B1");

    expect(invoke).toHaveBeenCalledWith("get_room_change_options", { bookingId: "B1" });
    expect(result).toEqual(options);
    expect(result.rooms[0].priceDifference).toBe(-150000);
  });
});
