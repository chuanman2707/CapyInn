import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { createCorrelationId } from "@/lib/correlationId";
import { createIdempotencyKey, invokeCommand } from "@/lib/invokeCommand";
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
} from "@/types";

interface HotelStore {
  rooms: Room[];
  stats: DashboardStats | null;
  dashboardRefreshVersion: number;
  roomDetail: RoomWithBooking | null;
  activeTab: HotelTab;
  housekeepingTasks: HousekeepingTask[];
  loading: boolean;
  isCheckinOpen: boolean;
  checkinRoomId: string | null;
  isGroupCheckinOpen: boolean;
  groups: BookingGroup[];

  fetchRooms: () => Promise<void>;
  fetchStats: () => Promise<void>;
  markDashboardDataChanged: () => void;
  setTab: (tab: HotelTab) => void;
  setCheckinOpen: (open: boolean, roomId?: string | null) => void;
  checkIn: (roomId: string, guests: CheckInGuestInput[], nights: number, paidAmount?: number, source?: string, notes?: string) => Promise<void>;
  checkOut: (
    bookingId: string,
    settlementMode: CheckoutSettlementMode,
    finalTotal: number,
  ) => Promise<void>;
  extendStay: (bookingId: string) => Promise<void>;
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

    fetchRooms: async () => {
      const rooms = await invoke<Room[]>("get_rooms");
      set({ rooms });
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
    setCheckinOpen: (open, roomId = null) =>
      set({
        isCheckinOpen: open,
        checkinRoomId: open ? roomId : null,
      }),

    checkIn: async (roomId, guests, nights, paidAmount, source, notes) => {
      beginAction();
      try {
        const correlationId = createCorrelationId();
        await invokeCommand(
          "check_in",
          {
            req: { room_id: roomId, guests, nights, source, notes, paid_amount: paidAmount },
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
          activeTab: "dashboard",
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
        await invokeCommand(
          "check_out",
          {
            req: {
              booking_id: bookingId,
              settlement_mode: settlementMode,
              final_total: finalTotal,
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
          activeTab: "dashboard",
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
        await invoke("extend_stay", { bookingId });
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

    fetchHousekeeping: async () => {
      const tasks = await invoke<HousekeepingTask[]>("get_housekeeping_tasks");
      set({ housekeepingTasks: tasks });
    },

    updateHousekeeping: async (taskId, status, note) => {
      await invoke("update_housekeeping", { taskId, newStatus: status, note });
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
        await invokeCommand(
          "group_checkin",
          { req, idempotencyKey: createIdempotencyKey("group_checkin") },
          { correlationId },
        );
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
        await invokeCommand("group_checkout", { req }, { correlationId });
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
      return invoke<GroupService>("add_group_service", { req });
    },

    removeGroupService: async (serviceId: string) => {
      await invoke("remove_group_service", { serviceId });
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
