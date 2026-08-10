import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

import GuestProfileSheet, { guestBookingStatusLabel } from "./GuestProfileSheet";

function historyWith(bookings: Array<Record<string, unknown>>) {
  return {
    guest: {
      id: "G-1",
      full_name: "Nguyễn Văn A",
      doc_number: "079099001234",
      nationality: "Việt Nam",
      date_of_birth: null,
      gender: null,
    },
    bookings,
  };
}

function bookingRow(overrides: Record<string, unknown>) {
  return {
    booking_id: "B-1",
    room_id: "101",
    check_in_at: "2026-08-01T14:00:00+07:00",
    expected_checkout: "2026-08-02T12:00:00+07:00",
    total_price: 500000,
    status: "active",
    ...overrides,
  };
}

// M (rà cuối trước merge): nhãn cũ chỉ có hai giá trị "Active"/"Completed"
// tiếng Anh, suy từ `status === "active"` — mọi status khác (checked_out,
// booked, cancelled, no_show) đều bị gộp chung vào "Completed", SAI SỰ THẬT
// cho một lượt đặt trước chưa hề diễn ra hay một lượt bị hủy.
describe("GuestProfileSheet — nhãn trạng thái lượt lưu trú", () => {
  it("lượt đang ở hiện tiếng Việt, không phải 'Active'", async () => {
    invoke.mockResolvedValueOnce(historyWith([bookingRow({ status: "active" })]));

    render(<GuestProfileSheet guestId="G-1" onClose={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText("Đang ở")).toBeTruthy();
    });
    expect(screen.queryByText("Active")).toBeNull();
  });

  it("lượt đã trả phòng hiện đúng câu tiếng Việt, không phải 'Completed'", async () => {
    invoke.mockResolvedValueOnce(historyWith([bookingRow({ status: "checked_out" })]));

    render(<GuestProfileSheet guestId="G-1" onClose={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText("Đã trả phòng")).toBeTruthy();
    });
    expect(screen.queryByText("Completed")).toBeNull();
  });

  it("lượt đặt trước chưa diễn ra không bị gọi là 'Completed'", async () => {
    invoke.mockResolvedValueOnce(historyWith([bookingRow({ status: "booked" })]));

    render(<GuestProfileSheet guestId="G-1" onClose={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText("Đặt trước")).toBeTruthy();
    });
    expect(screen.queryByText("Completed")).toBeNull();
  });

  it("lượt đã hủy không bị gọi là 'Completed'", async () => {
    invoke.mockResolvedValueOnce(historyWith([bookingRow({ status: "cancelled" })]));

    render(<GuestProfileSheet guestId="G-1" onClose={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText("Đã hủy")).toBeTruthy();
    });
    expect(screen.queryByText("Completed")).toBeNull();
  });

  // M2 (rà cuối trước merge): tham số cũ nhận `string`, nên thêm một status
  // thứ 7 vào union `BookingStatus` không hề đỏ ở đây — chỉ đỏ ở
  // Reservations.tsx (đo thật bằng `npx tsc --noEmit` sau khi thêm tạm một
  // status giả vào union). Status lạ rơi vào `default: return status`, hiện
  // nguyên chuỗi DB thô ("mystery_status") cho khách thay vì bị chặn ở biên
  // dịch. Sau khi siết tham số về `BookingStatus` + switch cạn kiệt (giống
  // hệt `assertUnreachableStatus` của Reservations.tsx), một giá trị status
  // không nằm trong union (chỉ có thể lọt vào bằng ép kiểu — chính là mô
  // phỏng "status lạ" ở đây) phải NÉM LỖI thay vì âm thầm hiện chuỗi thô.
  it("status lạ không nằm trong BookingStatus: ném lỗi thay vì hiện chuỗi DB thô", () => {
    expect(() => guestBookingStatusLabel("mystery_status" as never)).toThrow();
  });
});
