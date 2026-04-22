import type {
  ButtonHTMLAttributes,
  InputHTMLAttributes,
  LabelHTMLAttributes,
  ReactNode,
} from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  createAppErrorException,
  formatAppError,
  type AppError,
} from "@/lib/appError";

const autoAssignRooms = vi.hoisted(() => vi.fn());
const groupCheckIn = vi.hoisted(() => vi.fn());
const setGroupCheckinOpen = vi.hoisted(() => vi.fn());
const toastError = vi.hoisted(() => vi.fn());
const toastSuccess = vi.hoisted(() => vi.fn());

vi.mock("@/stores/useHotelStore", () => ({
  useHotelStore: () => ({
    isGroupCheckinOpen: true,
    setGroupCheckinOpen,
    rooms: [
      {
        id: "R101",
        name: "R101",
        type: "standard",
        room_type: "standard",
        floor: 1,
        has_balcony: false,
        base_price: 500000,
        max_guests: 2,
        extra_person_fee: 0,
        status: "vacant",
      },
    ],
    groupCheckIn,
    autoAssignRooms,
    loading: false,
  }),
}));

vi.mock("@/components/ui/sheet", () => ({
  Sheet: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  SheetContent: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  SheetHeader: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  SheetTitle: ({ children }: { children: ReactNode }) => <h2>{children}</h2>,
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

vi.mock("@/components/ui/label", () => ({
  Label: ({ children, ...props }: LabelHTMLAttributes<HTMLLabelElement>) => (
    <label {...props}>{children}</label>
  ),
}));

vi.mock("sonner", () => ({
  toast: {
    error: toastError,
    success: toastSuccess,
  },
}));

import GroupCheckinSheet from "./GroupCheckinSheet";

const autoAssignUserError: AppError = {
  code: "GROUP_NOT_ENOUGH_VACANT_ROOMS",
  message: "Chỉ có 1 phòng trống, cần 3 phòng",
  kind: "user",
  support_id: null,
};

const groupCheckInUserError: AppError = {
  code: "BOOKING_INVALID_STATE",
  message: "Master guest information is required",
  kind: "user",
  support_id: null,
};

describe("GroupCheckinSheet", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    autoAssignRooms.mockRejectedValue(
      createAppErrorException(autoAssignUserError),
    );
    groupCheckIn.mockResolvedValue(undefined);
  });

  it("keeps the sheet open and formats migrated auto-assign failures", async () => {
    const user = userEvent.setup();
    render(<GroupCheckinSheet />);

    const textboxes = screen.getAllByRole("textbox");
    await user.type(textboxes[0], "Test Group");
    await user.type(textboxes[1], "Organizer");
    await user.click(screen.getByRole("button", { name: /Tiếp theo/i }));
    await user.click(screen.getByRole("button", { name: /Tự động chọn 3 phòng/i }));

    expect(autoAssignRooms).toHaveBeenCalledWith(3, undefined);
    expect(toastError).toHaveBeenCalledWith(formatAppError(autoAssignUserError));
    expect(screen.getByText(/Bước 2\/4: Chọn phòng/i)).toBeInTheDocument();
    expect(setGroupCheckinOpen).not.toHaveBeenCalledWith(false);
  });

  it("surfaces correlation IDs when group check-in fails", async () => {
    const correlationId = "COR-1A2B3C4D";
    const user = userEvent.setup();

    autoAssignRooms.mockResolvedValue({
      assignments: [
        {
          room: {
            id: "R101",
            name: "R101",
            type: "standard",
            room_type: "standard",
            floor: 1,
            has_balcony: false,
            base_price: 500000,
            max_guests: 2,
            extra_person_fee: 0,
            status: "vacant",
          },
          floor: 1,
        },
      ],
    });
    groupCheckIn.mockRejectedValue(
      createAppErrorException(groupCheckInUserError, undefined, {
        correlation_id: correlationId,
      }),
    );

    render(<GroupCheckinSheet />);

    const textboxes = screen.getAllByRole("textbox");
    await user.type(textboxes[0], "Test Group");
    await user.type(textboxes[1], "Organizer");
    await user.click(screen.getByRole("button", { name: /Tiếp theo/i }));
    await user.click(screen.getByRole("button", { name: /Tự động chọn 3 phòng/i }));
    await user.click(screen.getByRole("button", { name: /Tiếp theo/i }));
    await user.click(screen.getByRole("button", { name: /Tiếp theo/i }));
    await user.click(
      screen.getByRole("button", { name: /Hoàn tất Group Check-in/i }),
    );

    expect(groupCheckIn).toHaveBeenCalledTimes(1);
    expect(toastError).toHaveBeenCalledWith(
      formatAppError({
        ...groupCheckInUserError,
        correlation_id: correlationId,
      }),
    );
    expect(setGroupCheckinOpen).not.toHaveBeenCalledWith(false);
  });
});
