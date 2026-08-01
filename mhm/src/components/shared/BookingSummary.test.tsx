import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import BookingSummary from "./BookingSummary";
import type { Booking } from "@/types";

const booking: Booking = {
  id: "B1",
  room_id: "R1",
  primary_guest_id: "G1",
  check_in_at: "2026-07-25T14:00:00+07:00",
  expected_checkout: "2026-08-03T12:00:00+07:00",
  nights: 9,
  total_price: 4_500_000,
  paid_amount: 0,
  status: "active",
  created_at: "2026-07-25T14:00:00+07:00",
};

const baseProps = {
  booking,
  onInvoice: vi.fn(),
  invoiceLoading: false,
};

describe("BookingSummary rate editing", () => {
  it("hiện giá mỗi đêm khi tổng chia hết cho số đêm", () => {
    render(<BookingSummary {...baseProps} />);

    expect(screen.getByText("Giá/đêm")).toBeInTheDocument();
    expect(screen.getByText("500.000đ")).toBeInTheDocument();
  });

  it("đổi nhãn thành Giá/đêm (TB) khi chia không hết", () => {
    render(
      <BookingSummary
        {...baseProps}
        booking={{ ...booking, total_price: 4_500_001 }}
      />,
    );

    expect(screen.getByText("Giá/đêm (TB)")).toBeInTheDocument();
  });

  it("không hiện nút sửa khi thiếu onSaveRate", () => {
    render(<BookingSummary {...baseProps} />);

    expect(screen.queryByRole("button", { name: /sửa giá/i })).toBeNull();
  });

  it("hiện preview tổng tiền khi gõ giá mới", async () => {
    render(<BookingSummary {...baseProps} onSaveRate={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: /sửa giá/i }));
    const input = screen.getByLabelText("Giá mỗi đêm");
    await userEvent.clear(input);
    await userEvent.type(input, "450000");

    expect(screen.getByText("9 đêm × 450.000đ = 4.050.000đ")).toBeInTheDocument();
  });

  it("gọi onSaveRate với con số đã nhập", async () => {
    const onSaveRate = vi.fn().mockResolvedValue(undefined);
    render(<BookingSummary {...baseProps} onSaveRate={onSaveRate} />);

    await userEvent.click(screen.getByRole("button", { name: /sửa giá/i }));
    const input = screen.getByLabelText("Giá mỗi đêm");
    await userEvent.clear(input);
    await userEvent.type(input, "450000");
    await userEvent.click(screen.getByRole("button", { name: "Lưu giá" }));

    expect(onSaveRate).toHaveBeenCalledWith(450000);
  });

  it("huỷ thì không gọi onSaveRate và đóng ô nhập", async () => {
    const onSaveRate = vi.fn();
    render(<BookingSummary {...baseProps} onSaveRate={onSaveRate} />);

    await userEvent.click(screen.getByRole("button", { name: /sửa giá/i }));
    await userEvent.click(screen.getByRole("button", { name: "Huỷ giá" }));

    expect(onSaveRate).not.toHaveBeenCalled();
    expect(screen.queryByLabelText("Giá mỗi đêm")).toBeNull();
  });

  it("khi onSaveRate reject: không phát sinh unhandled rejection, ô sửa giá vẫn mở với giá đã gõ", async () => {
    // RoomDrawer.handleSaveRate hiện toast lỗi rồi re-throw có chủ đích để
    // giữ ô sửa giá mở. Mô phỏng đúng hành vi đó ở đây: onSaveRate reject.
    // submitRate phải tự nuốt lỗi này (đã được caller xử lý xong), nếu không
    // rejection sẽ lọt ra khỏi onClick async mà không ai await, và Vitest sẽ
    // báo "Unhandled Rejection" cho chính test này.
    const onSaveRate = vi.fn().mockRejectedValue(new Error("Lỗi sửa giá"));
    render(<BookingSummary {...baseProps} onSaveRate={onSaveRate} />);

    await userEvent.click(screen.getByRole("button", { name: /sửa giá/i }));
    const input = screen.getByLabelText("Giá mỗi đêm");
    await userEvent.clear(input);
    await userEvent.type(input, "450000");
    await userEvent.click(screen.getByRole("button", { name: "Lưu giá" }));

    await waitFor(() => expect(onSaveRate).toHaveBeenCalledWith(450000));

    // Ô sửa giá vẫn còn mở, giá trị đã gõ được giữ nguyên.
    expect(screen.getByLabelText("Giá mỗi đêm")).toHaveValue("450000");
  });

  it("khoá nút Lưu khi giá không hợp lệ", async () => {
    render(<BookingSummary {...baseProps} onSaveRate={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: /sửa giá/i }));
    const input = screen.getByLabelText("Giá mỗi đêm");
    await userEvent.clear(input);
    await userEvent.type(input, "0");

    expect(screen.getByRole("button", { name: "Lưu giá" })).toBeDisabled();
  });
});
