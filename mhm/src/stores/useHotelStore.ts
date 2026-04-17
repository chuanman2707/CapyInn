import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
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
  GroupInvoiceData,
} from "@/types";

interface HotelStore {
  rooms: Room[];
  stats: DashboardStats | null;
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
  setTab: (tab: HotelTab) => void;
  setCheckinOpen: (open: boolean, roomId?: string | null) => void;
  checkIn: (roomId: string, guests: CheckInGuestInput[], nights: number, paidAmount?: number, source?: string, notes?: string) => Promise<void>;
  checkOut: (bookingId: string, finalPaid?: number) => Promise<void>;
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

// Helper for safe, parallel background refreshes that won't fail the primary action
const safeRefresh = async (...promises: Promise<void>[]) => {
  const results = await Promise.allSettled(promises);
  const errors = results
    .filter((r): r is PromiseRejectedResult => r.status === "rejected")
    .map((r) => r.reason);
  if (errors.length > 0) {
    console.error("Store refresh encountered errors:", errors);
  }
};

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

    setTab: (tab) => set({ activeTab: tab }),
    setCheckinOpen: (open, roomId = null) =>
      set({
        isCheckinOpen: open,
        checkinRoomId: open ? roomId : null,
      }),

    checkIn: async (roomId, guests, nights, paidAmount, source, notes) => {
      beginAction();
      try {
        await invoke("check_in", {
          req: { room_id: roomId, guests, nights, source, notes, paid_amount: paidAmount },
        });
        await safeRefresh(get().fetchRooms(), get().fetchStats());
        set({ activeTab: "dashboard" });
      } catch (err) {
        console.error("check_in error:", err);
        throw err;
      } finally {
        endAction();
      }
    },

    checkOut: async (bookingId, finalPaid) => {
      beginAction();
      try {
        await invoke("check_out", { req: { booking_id: bookingId, final_paid: finalPaid } });
        await safeRefresh(get().fetchRooms(), get().fetchStats());
        set({ activeTab: "dashboard" });
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
        await safeRefresh(get().fetchRooms(), get().fetchStats());
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
      await safeRefresh(get().fetchHousekeeping(), get().fetchRooms());
    },

    getStayInfoText: async (bookingId: string) => {
      return invoke<string>("get_stay_info_text", { bookingId });
    },

    // ── Group Booking Actions ──

    setGroupCheckinOpen: (open) => set({ isGroupCheckinOpen: open }),

    groupCheckIn: async (req) => {
      beginAction();
      try {
        await invoke("group_checkin", { req });
        await safeRefresh(get().fetchRooms(), get().fetchStats(), get().fetchGroups());
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
        await invoke("group_checkout", { req });
        await safeRefresh(get().fetchRooms(), get().fetchStats(), get().fetchGroups());
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
      return invoke<GroupDetailResponse>("get_group_detail", { groupId });
    },

    addGroupService: async (req) => {
      return invoke<GroupService>("add_group_service", { req });
    },

    removeGroupService: async (serviceId: string) => {
      await invoke("remove_group_service", { serviceId });
    },

    autoAssignRooms: async (roomCount: number, roomType?: string) => {
      return invoke<AutoAssignResult>("auto_assign_rooms", {
        req: { room_count: roomCount, room_type: roomType || null },
      });
    },

    generateGroupInvoice: async (groupId: string) => {
      return invoke<GroupInvoiceData>("generate_group_invoice", { groupId });
    },
  };
});
