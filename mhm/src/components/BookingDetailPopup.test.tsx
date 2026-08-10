// Khách sạn chạy theo giờ Việt Nam; cố định múi giờ để kỳ vọng định dạng
// ngày giờ không đổi theo máy chạy test.
process.env.TZ = "Asia/Ho_Chi_Minh";

import type { ButtonHTMLAttributes } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

vi.mock("@/components/ui/button", () => ({
  Button: ({ children, ...props }: ButtonHTMLAttributes<HTMLButtonElement>) => (
    <button {...props}>{children}</button>
  ),
}));

// Thay bằng bản giả tối giản: bài test ở đây chỉ cần xác nhận cách
// BookingDetailPopup NỐI DÂY với VoidBookingDialog (mở khi bấm "Xóa lượt
// này", đóng cả popup khi onVoided bắn), không phải lặp lại hành vi bên
// trong VoidBookingDialog — cái đó đã có VoidBookingDialog.test.tsx lo.
// Nền `bg-black/40` không tự stopPropagation, giống hệt bản thật, để bài
// test "bấm ra nền" bắt đúng lỗi nổi bọt nếu popup quên chặn nó.
vi.mock("@/components/VoidBookingDialog", () => ({
  default: ({ onClose, onVoided }: { onClose: () => void; onVoided: () => void }) => (
    <div className="fixed inset-0 bg-black/40" onClick={onClose}>
      <button onClick={onVoided}>mock xác nhận xóa</button>
    </div>
  ),
}));

import BookingDetailPopup from "./BookingDetailPopup";
import { useAuthStore } from "@/stores/useAuthStore";
import type { BookingWithGuest } from "@/types";

function booking(overrides: Partial<BookingWithGuest> = {}): BookingWithGuest {
  return {
    id: "B-1",
    room_id: "1B",
    room_name: "Room 1B",
    guest_name: "Hoseo Kim",
    check_in_at: "2026-07-23",
    expected_checkout: "2026-07-25",
    actual_checkout: "2026-07-25T09:12:00+07:00",
    nights: 2,
    total_price: 1200000,
    paid_amount: 1200000,
    status: "checked_out",
    source: "phone",
    booking_type: null,
    deposit_amount: 0,
    scheduled_checkin: "2026-07-23",
    scheduled_checkout: "2026-07-25",
    guest_phone: "0900000000",
    group_id: null,
    ...overrides,
  } as BookingWithGuest;
}

// "Đã trả — Hoseo Kim": lượt đã trả phòng dùng chung cho các test nút xoá,
// vì `booking()` mặc định đã đúng hình dạng đó (status checked_out, có
// actual_checkout).
const checkedOutBooking = booking();

describe("BookingDetailPopup", () => {
  it("shows reservation actions in reservation mode", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    const onEdit = vi.fn();
    const onCancel = vi.fn();

    render(
      <BookingDetailPopup
        booking={booking({ status: "booked", actual_checkout: null })}
        mode="reservation"
        onClose={vi.fn()}
        onConfirm={onConfirm}
        onEdit={onEdit}
        onCancel={onCancel}
      />,
    );

    await user.click(screen.getByRole("button", { name: /check-in/i }));
    expect(onConfirm).toHaveBeenCalledWith("B-1");

    await user.click(screen.getByRole("button", { name: /chỉnh sửa/i }));
    expect(onEdit).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("button", { name: /hủy/i }));
    expect(onCancel).toHaveBeenCalledWith("B-1");

    expect(screen.queryByRole("button", { name: /xem hóa đơn/i })).toBeNull();
  });

  it("opens the room change sheet for a booked reservation", async () => {
    const user = userEvent.setup();
    const onRoomChange = vi.fn();

    render(
      <BookingDetailPopup
        booking={booking({ status: "booked", actual_checkout: null })}
        mode="reservation"
        onClose={vi.fn()}
        onRoomChange={onRoomChange}
      />,
    );

    await user.click(screen.getByRole("button", { name: /chuyển phòng/i }));
    expect(onRoomChange).toHaveBeenCalledWith("B-1");
  });

  it("hides the room change action for a read-only (checked-out) booking", () => {
    render(
      <BookingDetailPopup
        booking={booking({ status: "checked_out" })}
        mode="readonly"
        onClose={vi.fn()}
        onViewInvoice={vi.fn()}
        onRoomChange={vi.fn()}
      />,
    );

    expect(screen.queryByRole("button", { name: /chuyển phòng/i })).toBeNull();
  });

  it("shows a read-only view with the actual checkout time for closed bookings", async () => {
    const user = userEvent.setup();
    const onViewInvoice = vi.fn();
    const onClose = vi.fn();

    // Khách vãng lai: không có lịch đặt trước, cả hai mốc đều là timestamp RFC3339.
    render(
      <BookingDetailPopup
        booking={booking({
          scheduled_checkin: null,
          scheduled_checkout: null,
          check_in_at: "2026-07-23T14:03:11.482913+07:00",
          actual_checkout: "2026-07-25T09:12:00+07:00",
        })}
        mode="readonly"
        onClose={onClose}
        onViewInvoice={onViewInvoice}
      />,
    );

    expect(screen.getByText(/Đã trả — Hoseo Kim/)).toBeTruthy();
    expect(screen.getByText("Trả phòng lúc")).toBeTruthy();
    expect(screen.getByText("Đã thanh toán")).toBeTruthy();

    expect(screen.getByText("14:03 23/07/2026")).toBeTruthy();
    expect(screen.getByText("09:12 25/07/2026")).toBeTruthy();
    expect(screen.queryByText("2026-07-23T14:03:11.482913+07:00")).toBeNull();
    expect(screen.queryByText("2026-07-25T09:12:00+07:00")).toBeNull();

    expect(screen.queryByRole("button", { name: /check-in/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /^hủy$/i })).toBeNull();

    await user.click(screen.getByRole("button", { name: /xem hóa đơn/i }));
    expect(onViewInvoice).toHaveBeenCalledWith("B-1");

    await user.click(screen.getByRole("button", { name: /đóng/i }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("falls back to the expected checkout when the actual one is missing", () => {
    render(
      <BookingDetailPopup
        booking={booking({ actual_checkout: null })}
        mode="readonly"
        onClose={vi.fn()}
        onViewInvoice={vi.fn()}
      />,
    );

    // Ngày trơn phải in ra ngày, không kèm giờ ảo của nửa đêm UTC.
    expect(screen.getByText("25/07/2026")).toBeTruthy();
    expect(screen.getByText("23/07/2026")).toBeTruthy();
    expect(screen.queryByText("2026-07-25")).toBeNull();
    expect(screen.queryByText(/00:00|07:00/)).toBeNull();
  });

  it("renders a date-only value as the correct calendar day even west of UTC", () => {
    // TZ toàn cục của file này là Asia/Ho_Chi_Minh (UTC+7, phía đông UTC), nơi
    // `new Date("YYYY-MM-DD")` (nửa đêm UTC) tình cờ vẫn hiển thị đúng ngày.
    // Đổi tạm sang múi giờ phía tây UTC để bài test thật sự bắt được lỗi lùi
    // ngày nếu ai đó quay lại dùng `new Date()` cho ngày trơn.
    const originalTz = process.env.TZ;
    process.env.TZ = "America/Los_Angeles"; // UTC-7/-8, phía tây UTC

    try {
      render(
        <BookingDetailPopup
          booking={booking({
            scheduled_checkin: "2026-07-25",
            scheduled_checkout: "2026-07-27",
          })}
          mode="reservation"
          onClose={vi.fn()}
        />,
      );

      // Ngày trơn 2026-07-25 phải vẫn hiện 25/07/2026, không lùi về 24/07/2026
      // như khi `new Date("2026-07-25")` bị hiểu là nửa đêm UTC.
      expect(screen.getByText("25/07/2026")).toBeTruthy();
      expect(screen.queryByText("24/07/2026")).toBeNull();
    } finally {
      process.env.TZ = originalTz;
    }
  });

  it("prefers the actual check-in time over the scheduled one in read-only mode", () => {
    render(
      <BookingDetailPopup
        booking={booking({
          scheduled_checkin: "2026-07-23",
          check_in_at: "2026-07-24T21:45:00+07:00",
        })}
        mode="readonly"
        onClose={vi.fn()}
        onViewInvoice={vi.fn()}
      />,
    );

    // Khách nhận phòng trễ hơn dự kiến: phải hiện mốc check-in thật sự
    // (check_in_at), không phải ngày dự kiến ban đầu (scheduled_checkin).
    expect(screen.getByText("21:45 24/07/2026")).toBeTruthy();
    expect(screen.queryByText("23/07/2026")).toBeNull();
  });

  it("disables the invoice button while an invoice request is in flight", () => {
    const onViewInvoice = vi.fn();

    render(
      <BookingDetailPopup
        booking={booking()}
        mode="readonly"
        onClose={vi.fn()}
        onViewInvoice={onViewInvoice}
        invoiceLoading
      />,
    );

    const button = screen.getByRole("button", { name: /đang tạo/i });
    expect(button).toBeDisabled();
    expect(screen.queryByRole("button", { name: /xem hóa đơn/i })).toBeNull();
  });

  it("admin thấy nút xóa ở lượt đã trả", () => {
    useAuthStore.setState({
      user: { id: "u1", name: "Chủ", role: "admin", active: true, created_at: "2026-08-07T09:00:00+07:00" },
    });
    render(
      <BookingDetailPopup booking={checkedOutBooking} mode="readonly" onClose={vi.fn()} />,
    );
    const button = screen.getByRole("button", { name: /xóa lượt này/i });
    expect(button.hasAttribute("disabled")).toBe(false);
  });

  it("lễ tân thấy nút xóa nhưng bị khoá, kèm lời giải thích", () => {
    useAuthStore.setState({
      user: { id: "u2", name: "Lễ tân", role: "receptionist", active: true, created_at: "2026-08-07T09:00:00+07:00" },
    });
    render(
      <BookingDetailPopup booking={checkedOutBooking} mode="readonly" onClose={vi.fn()} />,
    );
    expect(screen.getByRole("button", { name: /xóa lượt này/i }).hasAttribute("disabled")).toBe(
      true,
    );
    expect(screen.getByText(/chỉ chủ khách sạn xóa được/i)).toBeTruthy();
  });

  it("booking thuộc đoàn thì nút xóa bị khoá với lý do riêng", () => {
    useAuthStore.setState({
      user: { id: "u1", name: "Chủ", role: "admin", active: true, created_at: "2026-08-07T09:00:00+07:00" },
    });
    render(
      <BookingDetailPopup
        booking={{ ...checkedOutBooking, group_id: "G-1" }}
        mode="readonly"
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByRole("button", { name: /xóa lượt này/i }).hasAttribute("disabled")).toBe(
      true,
    );
    expect(screen.getByText(/thuộc đoàn/i)).toBeTruthy();
  });

  it("nút xóa cũng có mặt ở chế độ reservation, dưới hàng nút chính", () => {
    useAuthStore.setState({
      user: { id: "u1", name: "Chủ", role: "admin", active: true, created_at: "2026-08-07T09:00:00+07:00" },
    });
    render(
      <BookingDetailPopup
        booking={booking({ status: "booked", actual_checkout: null })}
        mode="reservation"
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByRole("button", { name: /xóa lượt này/i }).hasAttribute("disabled")).toBe(
      false,
    );
  });

  // Cả popup chi tiết lẫn VoidBookingDialog đều tự đóng khi bấm ra nền của
  // chính nó (`onClick={onClose}` trên div `fixed inset-0` ngoài cùng), và
  // VoidBookingDialog không tự stopPropagation ở nền của nó. Nếu
  // VoidBookingDialog được chèn thẳng vào cây DOM của popup mà không chặn,
  // bấm ra nền của hộp xác nhận xoá (định huỷ MỖI việc xoá) sẽ nổi bọt lên
  // và kích hoạt luôn `onClose` của popup bên dưới — đóng nhầm cả hai.
  it("bấm ra nền của hộp xác nhận xoá không đóng theo popup chi tiết", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    useAuthStore.setState({
      user: { id: "u1", name: "Chủ", role: "admin", active: true, created_at: "2026-08-07T09:00:00+07:00" },
    });
    const { container } = render(
      <BookingDetailPopup booking={checkedOutBooking} mode="readonly" onClose={onClose} />,
    );

    await user.click(screen.getByRole("button", { name: /xóa lượt này/i }));

    const voidBackdrop = container.querySelector('[class*="bg-black/40"]');
    expect(voidBackdrop).not.toBeNull();
    fireEvent.click(voidBackdrop!);

    expect(onClose).not.toHaveBeenCalled();
  });

  it("void thành công thì đóng luôn popup chi tiết", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    useAuthStore.setState({
      user: { id: "u1", name: "Chủ", role: "admin", active: true, created_at: "2026-08-07T09:00:00+07:00" },
    });
    render(
      <BookingDetailPopup booking={checkedOutBooking} mode="readonly" onClose={onClose} />,
    );

    await user.click(screen.getByRole("button", { name: /xóa lượt này/i }));
    await user.click(screen.getByRole("button", { name: /mock xác nhận xóa/i }));

    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
