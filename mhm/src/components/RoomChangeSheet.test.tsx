import type { ButtonHTMLAttributes, ReactNode } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

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

vi.mock("sonner", () => ({
  toast: {
    error: vi.fn(),
    success: vi.fn(),
  },
}));

import { useHotelStore } from "@/stores/useHotelStore";
import { createAppErrorException } from "@/lib/appError";
import type { RoomChangeOptions } from "@/types";
import { toast } from "sonner";
import { RoomChangeSheet } from "./RoomChangeSheet";

const baseOptions: RoomChangeOptions = {
  bookingId: "B1",
  currentRoomId: "5B",
  currentRoomName: "Phòng 5B",
  fromDate: "2026-08-03",
  toDate: "2026-08-07",
  nightsRemaining: 5,
  nightsStayed: 3,
  guestCount: 2,
  rooms: [
    {
      roomId: "2A",
      name: "Phòng 2A",
      roomType: "deluxe",
      floor: 2,
      maxGuests: 4,
      priceDifference: 500000,
    },
  ],
};

type FetchRoomChangeOptions = (bookingId: string) => Promise<RoomChangeOptions>;
type ChangeRoom = (
  bookingId: string,
  newRoomId: string,
  keepPrice: boolean,
  reason?: string,
) => Promise<void>;

function renderSheet(
  options: RoomChangeOptions,
  overrides: Partial<{
    fetchRoomChangeOptions: FetchRoomChangeOptions;
    changeRoom: ChangeRoom;
  }> = {},
) {
  const fetchRoomChangeOptions: FetchRoomChangeOptions =
    overrides.fetchRoomChangeOptions ?? vi.fn().mockResolvedValue(options);
  const changeRoom: ChangeRoom =
    overrides.changeRoom ?? vi.fn().mockResolvedValue(undefined);

  useHotelStore.setState({
    isRoomChangeOpen: true,
    roomChangeBookingId: options.bookingId,
    setRoomChangeOpen: vi.fn(),
    fetchRoomChangeOptions,
    changeRoom,
  });

  render(<RoomChangeSheet />);

  return { fetchRoomChangeOptions, changeRoom };
}

describe("RoomChangeSheet", () => {
  beforeEach(() => {
    useHotelStore.setState({ isRoomChangeOpen: false, roomChangeBookingId: null });
    vi.clearAllMocks();
  });

  it("nói rõ những đêm đã ở vẫn thuộc phòng cũ", async () => {
    renderSheet(baseOptions);
    expect(
      await screen.findByText(/3 đêm trước vẫn tính phòng 5B/i),
    ).toBeInTheDocument();
  });

  it("không hiện dòng đêm cũ khi đặt phòng chưa bắt đầu (nightsStayed = 0)", async () => {
    renderSheet({ ...baseOptions, nightsStayed: 0 });
    await screen.findByText("Phòng 2A");
    expect(screen.queryByText(/vẫn tính phòng/i)).not.toBeInTheDocument();
  });

  it("hiện chênh lệch sau khi chọn phòng", async () => {
    renderSheet(baseOptions);
    await userEvent.click(await screen.findByText("Phòng 2A"));
    // Chọn dòng có hậu tố "so với phòng cũ" — dòng phòng trong danh sách cũng
    // hiện chênh lệch (hậu tố "cả kỳ") nên chỉ khớp regex trần sẽ khớp cả hai.
    expect(screen.getByText(/\+500\.000đ so với phòng cũ/)).toBeInTheDocument();
  });

  it("dòng phòng hiện chênh lệch giá, không phải basePrice", async () => {
    renderSheet(baseOptions);
    expect(await screen.findByText(/\+500\.000đ/)).toBeInTheDocument();
  });

  // `priceDifference` là chênh lệch cho TOÀN BỘ số đêm còn lại. Dòng này trước
  // đây đọc là "…đ/đêm", nên một con số trần rất dễ bị đọc thành giá mỗi đêm và
  // báo sai cho khách.
  it("dòng phòng nói rõ chênh lệch là cho cả kỳ, không phải mỗi đêm", async () => {
    renderSheet(baseOptions);
    expect(await screen.findByText(/\+500\.000đ cả kỳ/)).toBeInTheDocument();
    expect(screen.queryByText(/500\.000đ\/đêm/)).not.toBeInTheDocument();
  });

  it("dòng phòng hiện dấu trừ khi phòng rẻ hơn phòng hiện tại", async () => {
    renderSheet({
      ...baseOptions,
      rooms: [
        {
          roomId: "3C",
          name: "Phòng 3C",
          roomType: "standard",
          floor: 3,
          maxGuests: 2,
          priceDifference: -50000,
        },
      ],
    });
    expect(await screen.findByText(/−50\.000đ cả kỳ/)).toBeInTheDocument();
  });

  it("dòng phòng hiện 'không đổi tiền' khi chênh lệch bằng 0", async () => {
    renderSheet({
      ...baseOptions,
      rooms: [
        {
          roomId: "4D",
          name: "Phòng 4D",
          roomType: "deluxe",
          floor: 4,
          maxGuests: 2,
          priceDifference: 0,
        },
      ],
    });
    await screen.findByText("Phòng 4D");
    expect(screen.getByText(/không đổi tiền/i)).toBeInTheDocument();
  });

  it("nút xác nhận hiện dấu trừ khi chọn phòng rẻ hơn", async () => {
    renderSheet({
      ...baseOptions,
      rooms: [
        {
          roomId: "3C",
          name: "Phòng 3C",
          roomType: "standard",
          floor: 3,
          maxGuests: 2,
          priceDifference: -50000,
        },
      ],
    });
    await userEvent.click(await screen.findByText("Phòng 3C"));
    const confirmButton = screen.getByRole("button", { name: /chuyển sang phòng 3c/i });
    expect(confirmButton).toHaveTextContent(/50\.000/);
    expect(confirmButton).toHaveTextContent(/trừ/);
  });

  it("nút xác nhận hiện 'không đổi tiền' khi phòng chọn không chênh lệch", async () => {
    renderSheet({
      ...baseOptions,
      rooms: [
        {
          roomId: "4D",
          name: "Phòng 4D",
          roomType: "deluxe",
          floor: 4,
          maxGuests: 2,
          priceDifference: 0,
        },
      ],
    });
    await userEvent.click(await screen.findByText("Phòng 4D"));
    const confirmButton = screen.getByRole("button", { name: /chuyển sang phòng 4d/i });
    expect(confirmButton).toHaveTextContent(/không đổi tiền/i);
  });

  it("gửi keepPrice true khi chọn giữ nguyên giá cũ", async () => {
    const changeRoom = vi.fn().mockResolvedValue(undefined);
    renderSheet(baseOptions, { changeRoom });

    await userEvent.click(await screen.findByText("Phòng 2A"));
    await userEvent.click(screen.getByLabelText(/giữ nguyên giá cũ/i));
    await userEvent.click(screen.getByRole("button", { name: /chuyển sang/i }));

    await waitFor(() =>
      expect(changeRoom).toHaveBeenCalledWith("B1", "2A", true, undefined),
    );
  });

  it("nút xác nhận nêu rõ phòng và tiền, không phải chỉ 'Xác nhận'", async () => {
    renderSheet(baseOptions);
    await userEvent.click(await screen.findByText("Phòng 2A"));
    const confirmButton = screen.getByRole("button", { name: /chuyển sang phòng 2a/i });
    expect(confirmButton).toHaveTextContent(/500\.000/);
    expect(confirmButton.textContent?.trim().toLowerCase()).not.toBe("xác nhận");
  });

  it("báo hết phòng thay vì hiện danh sách rỗng", async () => {
    renderSheet({ ...baseOptions, rooms: [] });
    expect(
      await screen.findByText(/Không phòng nào trống suốt 5 đêm còn lại/i),
    ).toBeInTheDocument();
  });

  it("khi đổi phòng thất bại: báo lỗi dễ đọc, nạp lại danh sách, bỏ chọn cũ", async () => {
    const fetchRoomChangeOptions = vi
      .fn()
      .mockResolvedValueOnce(baseOptions)
      .mockResolvedValueOnce({
        ...baseOptions,
        rooms: [
          {
            roomId: "3C",
            name: "Phòng 3C",
            roomType: "standard",
            floor: 3,
            maxGuests: 2,
            priceDifference: -100000,
          },
        ],
      });
    const changeRoom = vi.fn().mockRejectedValue(
      createAppErrorException({
        code: "BOOKING_INVALID_STATE",
        message: "Phòng 2A vừa được người khác chọn.",
        kind: "user",
        support_id: null,
      }),
    );

    renderSheet(baseOptions, { fetchRoomChangeOptions, changeRoom });

    await userEvent.click(await screen.findByText("Phòng 2A"));
    await userEvent.click(screen.getByRole("button", { name: /chuyển sang/i }));

    await waitFor(() => expect(toast.error).toHaveBeenCalled());
    const [message] = (toast.error as ReturnType<typeof vi.fn>).mock.calls[0];
    expect(message).toContain("Phòng 2A vừa được người khác chọn.");
    expect(message).not.toMatch(/\[object Object\]/);

    expect(fetchRoomChangeOptions).toHaveBeenCalledTimes(2);
    await screen.findByText("Phòng 3C");
    expect(screen.queryByText("Phòng 2A")).not.toBeInTheDocument();
  });
});
