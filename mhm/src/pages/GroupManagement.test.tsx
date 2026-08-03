import type {
  ButtonHTMLAttributes,
  HTMLAttributes,
  InputHTMLAttributes,
  ReactNode,
} from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  createAppErrorException,
  formatAppError,
  type AppError,
} from "@/lib/appError";

const fetchGroups = vi.hoisted(() => vi.fn());
const getGroupDetail = vi.hoisted(() => vi.fn());
const groupCheckout = vi.hoisted(() => vi.fn());
const addGroupService = vi.hoisted(() => vi.fn());
const removeGroupService = vi.hoisted(() => vi.fn());
const generateGroupInvoice = vi.hoisted(() => vi.fn());
const setRoomChangeOpen = vi.hoisted(() => vi.fn());
const toastError = vi.hoisted(() => vi.fn());
const toastSuccess = vi.hoisted(() => vi.fn());

vi.mock("@/stores/useHotelStore", () => ({
  useHotelStore: () => ({
    groups: [
      {
        id: "group-1",
        group_name: "Đoàn A",
        organizer_name: "Trưởng đoàn",
        organizer_phone: "0123456789",
        total_rooms: 1,
        status: "active",
        created_at: "2026-04-22T00:00:00Z",
      },
    ],
    fetchGroups,
    getGroupDetail,
    groupCheckout,
    addGroupService,
    removeGroupService,
    generateGroupInvoice,
    setRoomChangeOpen,
  }),
}));

vi.mock("@/components/ui/table", () => ({
  Table: ({ children }: { children: ReactNode }) => <table>{children}</table>,
  TableBody: ({ children }: { children: ReactNode }) => <tbody>{children}</tbody>,
  TableCell: ({ children, ...props }: HTMLAttributes<HTMLTableCellElement>) => (
    <td {...props}>{children}</td>
  ),
  TableHead: ({ children, ...props }: HTMLAttributes<HTMLTableCellElement>) => (
    <th {...props}>{children}</th>
  ),
  TableHeader: ({ children }: { children: ReactNode }) => <thead>{children}</thead>,
  TableRow: ({ children, ...props }: HTMLAttributes<HTMLTableRowElement>) => (
    <tr {...props}>{children}</tr>
  ),
}));

vi.mock("@/components/ui/badge", () => ({
  Badge: ({ children }: { children: ReactNode }) => <span>{children}</span>,
}));

vi.mock("@/components/ui/button", () => ({
  Button: ({
    children,
    ...props
  }: ButtonHTMLAttributes<HTMLButtonElement>) => <button {...props}>{children}</button>,
}));

vi.mock("@/components/ui/input", () => ({
  Input: (props: InputHTMLAttributes<HTMLInputElement>) => <input {...props} />,
}));

vi.mock("@/components/shared/EmptyState", () => ({
  default: ({ message }: { message: string }) => <div>{message}</div>,
}));

vi.mock("@/components/shared/SlideDrawer", () => ({
  default: ({
    children,
    open,
  }: {
    children: ReactNode;
    open: boolean;
  }) => (open ? <div>{children}</div> : null),
}));

vi.mock("@/components/InvoiceDialog", () => ({
  default: () => null,
}));

vi.mock("sonner", () => ({
  toast: {
    error: toastError,
    success: toastSuccess,
  },
}));

import { emitTestEvent, resetEventMocks } from "@test-mocks/tauri-event";
import GroupManagement from "./GroupManagement";

const checkoutUserError: AppError = {
  code: "BOOKING_INVALID_STATE",
  message: "Không thể checkout booking đã đóng",
  kind: "user",
  support_id: null,
};

describe("GroupManagement", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetEventMocks();
    fetchGroups.mockResolvedValue(undefined);
    getGroupDetail.mockResolvedValue({
      group: {
        id: "group-1",
        group_name: "Đoàn A",
        organizer_name: "Trưởng đoàn",
        organizer_phone: "0123456789",
        total_rooms: 1,
        status: "active",
        created_at: "2026-04-22T00:00:00Z",
      },
      bookings: [
        {
          id: "booking-1",
          room_id: "R101",
          room_name: "R101",
          guest_name: "Nguyễn Văn A",
          check_in_at: "2026-04-22T00:00:00Z",
          expected_checkout: "2026-04-23T00:00:00Z",
          actual_checkout: null,
          nights: 1,
          total_price: 500000,
          paid_amount: 0,
          status: "active",
          source: "walk-in",
          booking_type: "group",
          deposit_amount: null,
          scheduled_checkin: null,
          scheduled_checkout: null,
          guest_phone: null,
        },
      ],
      services: [],
      total_room_cost: 500000,
      total_service_cost: 0,
      grand_total: 500000,
      paid_amount: 0,
    });
    addGroupService.mockResolvedValue(undefined);
    removeGroupService.mockResolvedValue(undefined);
    generateGroupInvoice.mockResolvedValue(undefined);
    groupCheckout.mockResolvedValue(undefined);
  });

  it("surfaces correlation IDs when group checkout fails", async () => {
    const correlationId = "COR-5E6F7A8B";
    const user = userEvent.setup();

    groupCheckout.mockRejectedValue(
      createAppErrorException(checkoutUserError, undefined, {
        correlation_id: correlationId,
      }),
    );

    render(<GroupManagement />);

    await user.click(screen.getByText("Đoàn A"));

    const checkbox = await screen.findByRole("checkbox");
    await user.click(checkbox);
    await user.click(
      await screen.findByRole("button", { name: /Checkout 1 phòng/i }),
    );

    await waitFor(() => {
      expect(groupCheckout).toHaveBeenCalledWith({
        group_id: "group-1",
        booking_ids: ["booking-1"],
      });
    });

    expect(toastError).toHaveBeenCalledWith(
      formatAppError({
        ...checkoutUserError,
        correlation_id: correlationId,
      }),
    );
  });

  it("opens the room change sheet for an active booking in the group", async () => {
    const user = userEvent.setup();

    render(<GroupManagement />);

    await user.click(screen.getByText("Đoàn A"));
    await screen.findByText("Nguyễn Văn A");

    await user.click(screen.getByRole("button", { name: /chuyển phòng/i }));

    expect(setRoomChangeOpen).toHaveBeenCalledWith(true, "booking-1");
  });
});

describe("GroupManagement room change gating", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    fetchGroups.mockResolvedValue(undefined);
    getGroupDetail.mockResolvedValue({
      group: {
        id: "group-1",
        group_name: "Đoàn A",
        organizer_name: "Trưởng đoàn",
        organizer_phone: "0123456789",
        total_rooms: 1,
        status: "completed",
        created_at: "2026-04-22T00:00:00Z",
      },
      bookings: [
        {
          id: "booking-2",
          room_id: "R102",
          room_name: "R102",
          guest_name: "Trần Thị B",
          check_in_at: "2026-04-20T00:00:00Z",
          expected_checkout: "2026-04-22T00:00:00Z",
          actual_checkout: "2026-04-22T08:00:00Z",
          nights: 2,
          total_price: 800000,
          paid_amount: 800000,
          status: "checked_out",
          source: "walk-in",
          booking_type: "group",
          deposit_amount: null,
          scheduled_checkin: null,
          scheduled_checkout: null,
          guest_phone: null,
        },
      ],
      services: [],
      total_room_cost: 800000,
      total_service_cost: 0,
      grand_total: 800000,
      paid_amount: 800000,
    });
  });

  it("hides the room change action for a checked-out booking", async () => {
    const user = userEvent.setup();

    render(<GroupManagement />);

    await user.click(screen.getByText("Đoàn A"));
    await screen.findByText("Trần Thị B");

    expect(screen.queryByRole("button", { name: /chuyển phòng/i })).toBeNull();
  });
});

describe("GroupManagement refresh after a room change", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetEventMocks();
    fetchGroups.mockResolvedValue(undefined);
    groupCheckout.mockResolvedValue(undefined);
  });

  function groupDetailWith(roomName: string, totalPrice: number) {
    return {
      group: {
        id: "group-1",
        group_name: "Đoàn A",
        organizer_name: "Trưởng đoàn",
        organizer_phone: "0123456789",
        total_rooms: 1,
        status: "active",
        created_at: "2026-04-22T00:00:00Z",
      },
      bookings: [
        {
          id: "booking-1",
          room_id: roomName,
          room_name: roomName,
          guest_name: "Nguyễn Văn A",
          check_in_at: "2026-04-22T00:00:00Z",
          expected_checkout: "2026-04-23T00:00:00Z",
          actual_checkout: null,
          nights: 1,
          total_price: totalPrice,
          paid_amount: 0,
          status: "active",
          source: "walk-in",
          booking_type: "group",
          deposit_amount: null,
          scheduled_checkin: null,
          scheduled_checkout: null,
          guest_phone: null,
        },
      ],
      services: [],
      total_room_cost: totalPrice,
      total_service_cost: 0,
      grand_total: totalPrice,
      paid_amount: 0,
    };
  }

  // `detail` là state cục bộ và RoomChangeSheet không có callback quay về đây,
  // nên nếu trang không nghe "db-updated" thì sau khi chuyển R102 → R105
  // (+500.000đ) dòng vẫn ghi R102 với giá cũ và `grand_total` vẫn thiếu khoản
  // chênh — lễ tân đọc con số đó lên là báo sai tiền cho khách.
  it("cập nhật phòng và tổng tiền sau khi backend báo dữ liệu đổi", async () => {
    const user = userEvent.setup();
    getGroupDetail.mockResolvedValue(groupDetailWith("R102", 500000));

    render(<GroupManagement />);
    await user.click(screen.getByText("Đoàn A"));
    await screen.findByText("R102");

    // Khách vừa được chuyển sang R105 và bị tính thêm 500.000đ.
    getGroupDetail.mockResolvedValue(groupDetailWith("R105", 1000000));
    await emitTestEvent("db-updated", { entity: "bookings" });

    await waitFor(() => {
      expect(screen.getByText("R105")).toBeTruthy();
    });
    expect(screen.queryByText("R102")).toBeNull();
    expect(getGroupDetail).toHaveBeenCalledTimes(2);
  });

  // Nuốt lỗi ở chỗ nghe này là quay lại đúng triệu chứng nó sinh ra để chống:
  // bảng đứng yên với phòng cũ và tổng cũ mà lễ tân không biết là đã hỏng.
  it("báo lỗi khi nạp lại thất bại thay vì im lặng để bảng cũ trên màn hình", async () => {
    const user = userEvent.setup();
    getGroupDetail.mockResolvedValue(groupDetailWith("R102", 500000));

    render(<GroupManagement />);
    await user.click(screen.getByText("Đoàn A"));
    await screen.findByText("R102");

    getGroupDetail.mockRejectedValue(new Error("mất kết nối cơ sở dữ liệu"));
    await emitTestEvent("db-updated", { entity: "bookings" });

    await waitFor(() => {
      expect(toastError).toHaveBeenCalled();
    });
    expect(screen.getByText("R102")).toBeTruthy();
  });
});
