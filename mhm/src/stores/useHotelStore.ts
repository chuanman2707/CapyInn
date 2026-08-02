import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { createCorrelationId } from "@/lib/correlationId";
import { invokeCommand, invokeWriteCommand } from "@/lib/invokeCommand";
import { assertNonNegativeMoneyVnd, optionalMoneyVnd, type MoneyVnd } from "@/lib/money";
import type {
  CheckInGuestInput,
  DashboardStats,
  HotelTab,
  HousekeepingTask,
  Room,
  RoomWithBooking,
  BookingGroup,
  GroupCheckinRequest,
  GroupCheckoutRequest,
  GroupDetailResponse,
  AddGroupServiceRequest,
  GroupService,
  AutoAssignResult,
  CheckoutSettlementMode,
  GroupInvoiceData,
  RoomTypeRate,
  RoomChangeOptions,
} from "@/types";

interface HotelStore {
  rooms: Room[];
  /**
   * Giá niêm yết theo loại phòng, key là `rooms.type`.
   *
   * `null` = chưa tải được. Cố ý không có giá trị mặc định: màn hình nào không
   * đọc được giá thì hiện "—", chứ không bịa một số ra. `room.base_price` không
   * dùng làm giá dự phòng — engine bỏ qua nó ngay khi loại phòng có bảng giá.
   */
  roomTypeRates: Record<string, RoomTypeRate> | null;
  stats: DashboardStats | null;
  dashboardRefreshVersion: number;
  roomDetail: RoomWithBooking | null;
  activeTab: HotelTab;
  housekeepingTasks: HousekeepingTask[];
  loading: boolean;
  isCheckinOpen: boolean;
  checkinRoomId: string | null;
  checkinNights: number | null;
  isGroupCheckinOpen: boolean;
  groups: BookingGroup[];
  isRoomChangeOpen: boolean;
  roomChangeBookingId: string | null;

  fetchRooms: () => Promise<void>;
  /** Không bao giờ throw: thất bại thì đặt `roomTypeRates` về `null`. */
  fetchRoomTypeRates: () => Promise<void>;
  fetchStats: () => Promise<void>;
  markDashboardDataChanged: () => void;
  setTab: (tab: HotelTab) => void;
  setCheckinOpen: (open: boolean, roomId?: string | null, nights?: number | null) => void;
  checkIn: (roomId: string, guests: CheckInGuestInput[], nights: number, paidAmount?: MoneyVnd, source?: string, notes?: string) => Promise<void>;
  checkOut: (
    bookingId: string,
    settlementMode: CheckoutSettlementMode,
    finalTotal: MoneyVnd,
  ) => Promise<void>;
  extendStay: (bookingId: string) => Promise<void>;
  setRoomChangeOpen: (open: boolean, bookingId?: string | null) => void;
  fetchRoomChangeOptions: (bookingId: string) => Promise<RoomChangeOptions>;
  changeRoom: (bookingId: string, newRoomId: string, keepPrice: boolean, reason?: string) => Promise<void>;
  fetchHousekeeping: () => Promise<void>;
  updateHousekeeping: (taskId: string, status: string, note?: string) => Promise<void>;
  getStayInfoText: (bookingId: string) => Promise<string>;
  setGroupCheckinOpen: (open: boolean) => void;
  groupCheckIn: (req: GroupCheckinRequest) => Promise<void>;
  groupCheckout: (req: GroupCheckoutRequest) => Promise<void>;
  fetchGroups: (status?: string) => Promise<void>;
  getGroupDetail: (groupId: string) => Promise<GroupDetailResponse>;
  addGroupService: (req: AddGroupServiceRequest) => Promise<GroupService>;
  removeGroupService: (serviceId: string) => Promise<void>;
  autoAssignRooms: (roomCount: number, roomType?: string) => Promise<AutoAssignResult>;
  generateGroupInvoice: (groupId: string) => Promise<GroupInvoiceData>;
}

export const useHotelStore = create<HotelStore>((set, get) => {
  let pendingActions = 0;

  const beginAction = () => {
    pendingActions += 1;
    set({ loading: true });
  };

  const endAction = () => {
    pendingActions = Math.max(0, pendingActions - 1);
    set({ loading: pendingActions > 0 });
  };

  return {
    rooms: [],
    roomTypeRates: null,
    stats: null,
    dashboardRefreshVersion: 0,
    roomDetail: null,
    activeTab: "dashboard",
    housekeepingTasks: [],
    loading: false,
    isCheckinOpen: false,
    checkinRoomId: null,
    checkinNights: null,
    isGroupCheckinOpen: false,
    groups: [],
    isRoomChangeOpen: false,
    roomChangeBookingId: null,

    fetchRooms: async () => {
      const rooms = await invoke<Room[]>("get_rooms");
      set({ rooms });
      // Cùng một lệnh tải: mọi màn hình hiện giá cạnh phòng đều đọc hai thứ này
      // cùng lúc, và `fetchRooms` đã được gọi sau mọi lệnh ghi. Giá tải lỗi thì
      // danh sách phòng vẫn hiện — nên `fetchRoomTypeRates` không throw.
      await get().fetchRoomTypeRates();
    },

    fetchRoomTypeRates: async () => {
      try {
        const rates = await invoke<RoomTypeRate[]>("get_room_type_rates");
        set({
          roomTypeRates: Object.fromEntries(rates.map((rate) => [rate.room_type, rate])),
        });
      } catch {
        set({ roomTypeRates: null });
      }
    },

    fetchStats: async () => {
      const stats = await invoke<DashboardStats>("get_dashboard_stats");
      set({ stats });
    },

    markDashboardDataChanged: () =>
      set((state) => ({
        dashboardRefreshVersion: state.dashboardRefreshVersion + 1,
      })),

    setTab: (tab) => set({ activeTab: tab }),
    setCheckinOpen: (open, roomId = null, nights = null) =>
      set({
        isCheckinOpen: open,
        checkinRoomId: open ? roomId : null,
        checkinNights: open ? nights : null,
      }),

    checkIn: async (roomId, guests, nights, paidAmount, source, notes) => {
      beginAction();
      try {
        const correlationId = createCorrelationId();
        await invokeWriteCommand(
          "check_in",
          {
            req: {
              room_id: roomId,
              guests,
              nights,
              source,
              notes,
              paid_amount: optionalMoneyVnd(paidAmount, "paid_amount"),
            },
          },
          {
            correlationId,
            monitoringContext: {
              guest_count: guests.length,
              nights,
              source: source ?? null,
              notes_present: Boolean(notes?.trim()),
            },
          },
        );
        await get().fetchRooms();
        await get().fetchStats();
        set((state) => ({
          dashboardRefreshVersion: state.dashboardRefreshVersion + 1,
        }));
      } catch (err) {
        console.error("check_in error:", err);
        throw err;
      } finally {
        endAction();
      }
    },

    checkOut: async (bookingId, settlementMode, finalTotal) => {
      beginAction();
      try {
        const correlationId = createCorrelationId();
        await invokeWriteCommand(
          "check_out",
          {
            req: {
              booking_id: bookingId,
              settlement_mode: settlementMode,
              final_total: assertNonNegativeMoneyVnd(finalTotal, "final_total"),
            },
          },
          {
            correlationId,
            monitoringContext: {
              settlement_mode: settlementMode,
            },
          },
        );
        await get().fetchRooms();
        await get().fetchStats();
        set((state) => ({
          dashboardRefreshVersion: state.dashboardRefreshVersion + 1,
        }));
      } catch (err) {
        console.error("check_out error:", err);
        throw err;
      } finally {
        endAction();
      }
    },

    extendStay: async (bookingId) => {
      beginAction();
      try {
        const correlationId = createCorrelationId();
        await invokeWriteCommand(
          "extend_stay",
          { bookingId },
          {
            correlationId,
            monitoringContext: {
              operation: "add_one_night",
            },
          },
        );
        await get().fetchRooms();
        await get().fetchStats();
        get().markDashboardDataChanged();
      } catch (err) {
        console.error("extend_stay error:", err);
        throw err;
      } finally {
        endAction();
      }
    },

    setRoomChangeOpen: (open, bookingId = null) =>
      set({ isRoomChangeOpen: open, roomChangeBookingId: open ? bookingId : null }),

    fetchRoomChangeOptions: async (bookingId) =>
      invoke<RoomChangeOptions>("get_room_change_options", { bookingId }),

    changeRoom: async (bookingId, newRoomId, keepPrice, reason) => {
      beginAction();
      try {
        const correlationId = createCorrelationId();
        await invokeWriteCommand(
          "change_room",
          { bookingId, newRoomId, keepPrice, reason: reason ?? null },
          {
            correlationId,
            monitoringContext: { operation: "change_room" },
          },
        );
        await get().fetchRooms();
        await get().fetchStats();
        get().markDashboardDataChanged();
      } catch (err) {
        console.error("change_room error:", err);
        throw err;
      } finally {
        endAction();
      }
    },

    fetchHousekeeping: async () => {
      const tasks = await invoke<HousekeepingTask[]>("get_housekeeping_tasks");
      set({ housekeepingTasks: tasks });
    },

    updateHousekeeping: async (taskId, status, note) => {
      await invokeWriteCommand("update_housekeeping", { taskId, newStatus: status, note });
      await get().fetchHousekeeping();
      await get().fetchRooms();
    },

    getStayInfoText: async (bookingId: string) => {
      return invoke<string>("get_stay_info_text", { bookingId });
    },

    // ── Group Booking Actions ──

    setGroupCheckinOpen: (open) => set({ isGroupCheckinOpen: open }),

    groupCheckIn: async (req) => {
      beginAction();
      try {
        const correlationId = createCorrelationId();
        const guardedReq: GroupCheckinRequest = {
          ...req,
          paid_amount: optionalMoneyVnd(req.paid_amount, "paid_amount"),
        };
        await invokeWriteCommand("group_checkin", { req: guardedReq }, { correlationId });
        await get().fetchRooms();
        await get().fetchStats();
        await get().fetchGroups();
        get().markDashboardDataChanged();
        set({ isGroupCheckinOpen: false });
      } catch (err) {
        console.error("group_checkin error:", err);
        throw err;
      } finally {
        endAction();
      }
    },

    groupCheckout: async (req) => {
      beginAction();
      try {
        const correlationId = createCorrelationId();
        const guardedReq: GroupCheckoutRequest = {
          ...req,
          final_paid: optionalMoneyVnd(req.final_paid, "final_paid"),
        };
        await invokeWriteCommand("group_checkout", { req: guardedReq }, { correlationId });
        await get().fetchRooms();
        await get().fetchStats();
        await get().fetchGroups();
        get().markDashboardDataChanged();
      } catch (err) {
        console.error("group_checkout error:", err);
        throw err;
      } finally {
        endAction();
      }
    },

    fetchGroups: async (status?: string) => {
      const groups = await invoke<BookingGroup[]>("get_all_groups", { status: status || null });
      set({ groups });
    },

    getGroupDetail: async (groupId: string) => {
      return invokeCommand<GroupDetailResponse>("get_group_detail", { groupId });
    },

    addGroupService: async (req) => {
      return invokeWriteCommand<GroupService>("add_group_service", {
        req: {
          ...req,
          unit_price: assertNonNegativeMoneyVnd(req.unit_price, "unit_price"),
        },
      });
    },

    removeGroupService: async (serviceId: string) => {
      await invokeWriteCommand("remove_group_service", { serviceId });
    },

    autoAssignRooms: async (roomCount: number, roomType?: string) => {
      return invokeCommand<AutoAssignResult>("auto_assign_rooms", {
        req: { room_count: roomCount, room_type: roomType || null },
      });
    },

    generateGroupInvoice: async (groupId: string) => {
      return invoke<GroupInvoiceData>("generate_group_invoice", { groupId });
    },
  };
});
