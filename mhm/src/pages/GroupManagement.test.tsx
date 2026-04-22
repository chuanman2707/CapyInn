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
});
