import type { ButtonHTMLAttributes, ReactNode } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { clearMockResponses, setMockResponse } from "@test-mocks/tauri-core";
import { useHotelStore } from "@/stores/useHotelStore";
import type { Room, RoomStatus } from "@/types";

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

import CheckinSheet from "./CheckinSheet";

function room(id: string, status: RoomStatus): Room {
  return {
    id,
    name: id,
    type: "standard",
    floor: 1,
    has_balcony: false,
    base_price: 250000,
    max_guests: 2,
    extra_person_fee: 50000,
    status,
  };
}

// Ô "Phòng *" là <select> trần, nhãn không có htmlFor/id liên kết — cùng lý do
// đã ghi ở BackfillSheet.test.tsx.
function roomSelect(): HTMLSelectElement {
  const label = screen.getByText("Phòng *");
  const el = label.parentElement?.querySelector("select");
  if (!el) throw new Error('Không tìm thấy <select> cạnh nhãn "Phòng *"');
  return el;
}

describe("CheckinSheet", () => {
  beforeEach(() => {
    clearMockResponses();
    useHotelStore.setState({ isCheckinOpen: false, rooms: [] });
  });

  it("điền sẵn số đêm khi mở từ calendar", async () => {
    useHotelStore.setState({ isCheckinOpen: true });
    render(<CheckinSheet preSelectedRoomId="1A" preSelectedNights={3} />);

    // FormField (shared) chỉ đặt <label> cạnh <input>, không có htmlFor/id
    // liên kết chúng — nên getByLabelText("Số đêm") không tìm ra input này
    // dù component đã prefill đúng. Đây là lối thoát chính brief cho phép:
    // xác minh qua display value của input số đêm.
    expect(await screen.findByDisplayValue("3")).toBeInTheDocument();
  });

  // Ô lịch của một phòng ĐANG CÓ KHÁCH vẫn kéo được (hôm nay khách sắp trả),
  // và cú kéo đó truyền thẳng roomId vào đây mà không ai kiểm tra nó có nằm
  // trong danh sách phòng trống hay không. Trước nhánh này mọi caller đều đã
  // gác sẵn trên "vacant" nên tình huống không tới được.
  it("bỏ chọn phòng khi phòng truyền vào không nằm trong danh sách phòng trống", async () => {
    setMockResponse("get_rooms", () => [room("R101", "occupied")]);
    useHotelStore.setState({ isCheckinOpen: true });
    render(<CheckinSheet preSelectedRoomId="R101" />);

    await waitFor(() => {
      expect(roomSelect()).toHaveValue("");
    });

    // Ô select trống chưa chứng minh được gì: jsdom cũng hiện trống khi state
    // vẫn giữ R101 mà option đã bị lọc mất. Cho R101 trở lại danh sách — nếu
    // state chưa thực sự bị xoá, select sẽ tự nhảy về "R101" dù chủ chưa hề
    // chọn lại nó.
    act(() => {
      useHotelStore.setState({ rooms: [room("R101", "vacant")] });
    });

    await waitFor(() => {
      expect(
        Array.from(roomSelect().options).map((o) => o.value),
      ).toContain("R101");
    });
    expect(roomSelect()).toHaveValue("");
  });

  it("giữ nguyên phòng truyền vào khi phòng đó đang trống", async () => {
    setMockResponse("get_rooms", () => [room("R101", "vacant")]);
    useHotelStore.setState({ isCheckinOpen: true });
    render(<CheckinSheet preSelectedRoomId="R101" />);

    await waitFor(() => {
      expect(roomSelect()).toHaveValue("R101");
    });
  });
});
