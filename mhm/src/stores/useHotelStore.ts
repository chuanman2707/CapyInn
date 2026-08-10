import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { createCorrelationId } from "@/lib/correlationId";
import { invokeCommand, invokeWriteCommand } from "@/lib/invokeCommand";
import {
  assertNonNegativeMoneyVnd,
  moneyValidationError,
  optionalMoneyVnd,
  type MoneyVnd,
} from "@/lib/money";
import type {
  CheckInGuestInput,
  DashboardStats,
  HotelTab,
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
  checkIn: (
    roomId: string,
    guests: CheckInGuestInput[],
    nights: number,
    paidAmount?: MoneyVnd,
    source?: string,
    notes?: string,
    // Object thay vì tham số vị trí thứ 7: hàm này đã có 6 tham số cùng kiểu
    // dữ liệu mập mờ (string | number | undefined) đứng cạnh nhau — thêm một
    // `number | null` nữa vào cuối là chỗ dễ đọc nhầm thứ tự nhất. Gói riêng
    // buộc mọi call site phải gõ tên trường ra, đọc là hiểu ngay.
    //
    // `guestCount` vào đây chứ KHÔNG thành tham số vị trí thứ 8: nó cũng là
    // `number | null`, đứng cạnh `rateOverridePerNight` thì đúng hai con số
    // dễ hoán vị nhất nằm liền nhau — một cú gọi nhầm thứ tự sẽ lấy số khách
    // làm giá phòng mà vẫn biên dịch trót lọt.
    options?: { rateOverridePerNight?: number | null; guestCount?: number | null },
  ) => Promise<void>;
  checkOut: (
    bookingId: string,
    settlementMode: CheckoutSettlementMode,
    finalTotal: MoneyVnd,
  ) => Promise<void>;
  extendStay: (bookingId: string) => Promise<void>;
  shortenStay: (bookingId: string) => Promise<void>;
  setBookingRate: (bookingId: string, ratePerNight: number) => Promise<void>;
  voidBooking: (bookingId: string, reason: string | null) => Promise<void>;
  updateBookingNotes: (bookingId: string, notes: string) => Promise<void>;
  setRoomChangeOpen: (open: boolean, bookingId?: string | null) => void;
  fetchRoomChangeOptions: (bookingId: string) => Promise<RoomChangeOptions>;
  changeRoom: (bookingId: string, newRoomId: string, keepPrice: boolean, reason?: string) => Promise<void>;
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

    checkIn: async (roomId, guests, nights, paidAmount, source, notes, options) => {
      beginAction();
      try {
        const correlationId = createCorrelationId();
        const rateOverridePerNight = options?.rateOverridePerNight ?? null;
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
              // Bỏ trống thì backend hiểu là một người. Gửi `null` chứ không
              // gửi 1 ở đây, để chỗ quyết định "trống nghĩa là mấy" chỉ có một.
              guest_count: options?.guestCount ?? null,
              // Khoá tường minh, kể cả khi không sửa giá: `null` đọc log ra
              // thấy được là "đã hỏi và giữ giá hệ thống", còn thiếu khoá thì
              // không phân biệt được với "phiên bản cũ chưa biết trường này".
              rate_override_per_night:
                rateOverridePerNight != null
                  ? assertNonNegativeMoneyVnd(rateOverridePerNight, "rateOverridePerNight")
                  : null,
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

    shortenStay: async (bookingId) => {
      beginAction();
      try {
        const correlationId = createCorrelationId();
        await invokeWriteCommand(
          "shorten_stay",
          { bookingId },
          {
            correlationId,
            monitoringContext: {
              operation: "remove_one_night",
            },
          },
        );
        await get().fetchRooms();
        await get().fetchStats();
        get().markDashboardDataChanged();
      } catch (err) {
        console.error("shorten_stay error:", err);
        throw err;
      } finally {
        endAction();
      }
    },

    setBookingRate: async (bookingId, ratePerNight) => {
      beginAction();
      try {
        const correlationId = createCorrelationId();
        await invokeWriteCommand(
          "set_booking_rate",
          { bookingId, ratePerNight: assertNonNegativeMoneyVnd(ratePerNight, "ratePerNight") },
          {
            correlationId,
            monitoringContext: {
              operation: "set_booking_rate",
            },
          },
        );
        await get().fetchRooms();
        await get().fetchStats();
        get().markDashboardDataChanged();
      } catch (err) {
        console.error("set_booking_rate error:", err);
        throw err;
      } finally {
        endAction();
      }
    },

    voidBooking: async (bookingId, reason) => {
      beginAction();
      try {
        const correlationId = createCorrelationId();
        await invokeWriteCommand(
          "void_booking",
          { req: { booking_id: bookingId, reason } },
          {
            correlationId,
            monitoringContext: {
              operation: "void_booking",
            },
          },
        );
        await get().fetchRooms();
        await get().fetchStats();
        get().markDashboardDataChanged();
      } catch (err) {
        console.error("void_booking error:", err);
        throw err;
      } finally {
        endAction();
      }
    },

    updateBookingNotes: async (bookingId, notes) => {
      beginAction();
      try {
        await invokeCommand("update_booking_notes", { bookingId, notes: notes || null });
      } catch (err) {
        console.error("update_booking_notes error:", err);
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
          rate_override_per_room: Object.fromEntries(
            Object.entries(req.rate_override_per_room).map(([roomId, rate]) => {
              const checked = assertNonNegativeMoneyVnd(rate, `rate_override_per_room.${roomId}`);
              // I1 (review Task 18): gate backend là `rate <= 0 ||
              // rate > MAX_RATE_PER_NIGHT_VND` (group_lifecycle.rs:1524).
              // `assertNonNegativeMoneyVnd` (>= 0) cho 0 đi lọt — ô giá bị
              // xoá trắng gửi đúng `Number("") === 0`. Chặn tại đây, nêu
              // đúng phòng, để lễ tân biết ngay phòng nào đang gõ sai thay
              // vì một lỗi chung từ chối cả đoàn.
              if (checked <= 0) {
                const roomLabel = get().rooms.find((r) => r.id === roomId)?.name ?? roomId;
                throw moneyValidationError(`Giá phòng ${roomLabel} không hợp lệ — phải lớn hơn 0₫`);
              }
              return [roomId, checked];
            }),
          ),
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
