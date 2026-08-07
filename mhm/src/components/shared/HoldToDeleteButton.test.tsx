import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import HoldToDeleteButton from "./HoldToDeleteButton";

describe("HoldToDeleteButton", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("không kích hoạt khi nhả tay sớm", () => {
    const onHoldComplete = vi.fn();
    render(<HoldToDeleteButton label="Giữ để xóa" onHoldComplete={onHoldComplete} />);

    const button = screen.getByRole("button", { name: /giữ để xóa/i });
    fireEvent.pointerDown(button);
    vi.advanceTimersByTime(1200);
    fireEvent.pointerUp(button);
    vi.advanceTimersByTime(3000);

    expect(onHoldComplete).not.toHaveBeenCalled();
  });

  it("kích hoạt đúng một lần khi giữ đủ 2 giây, đúng vào mốc — chưa đủ thì chưa gọi", () => {
    const onHoldComplete = vi.fn();
    render(<HoldToDeleteButton label="Giữ để xóa" onHoldComplete={onHoldComplete} />);

    const button = screen.getByRole("button", { name: /giữ để xóa/i });
    fireEvent.pointerDown(button);

    vi.advanceTimersByTime(1999);
    expect(onHoldComplete).not.toHaveBeenCalled();

    vi.advanceTimersByTime(1);
    expect(onHoldComplete).toHaveBeenCalledTimes(1);

    // Giữ thêm sau khi đã bắn callback không được gọi thêm lần nào nữa.
    vi.advanceTimersByTime(2000);
    expect(onHoldComplete).toHaveBeenCalledTimes(1);
  });

  it("huỷ khi con trỏ rời khỏi nút giữa chừng", () => {
    const onHoldComplete = vi.fn();
    render(<HoldToDeleteButton label="Giữ để xóa" onHoldComplete={onHoldComplete} />);

    const button = screen.getByRole("button", { name: /giữ để xóa/i });
    fireEvent.pointerDown(button);
    vi.advanceTimersByTime(1000);
    fireEvent.pointerLeave(button);
    vi.advanceTimersByTime(3000);

    expect(onHoldComplete).not.toHaveBeenCalled();
  });

  it("không làm gì khi bị khoá", () => {
    const onHoldComplete = vi.fn();
    render(
      <HoldToDeleteButton label="Giữ để xóa" disabled onHoldComplete={onHoldComplete} />,
    );

    const button = screen.getByRole("button", { name: /giữ để xóa/i });
    fireEvent.pointerDown(button);
    vi.advanceTimersByTime(3000);

    expect(onHoldComplete).not.toHaveBeenCalled();
  });

  it("chỉ tính cú nhấn chuột trái — nhấn giữ chuột phải/giữa không kích hoạt", () => {
    const onHoldComplete = vi.fn();
    render(<HoldToDeleteButton label="Giữ để xóa" onHoldComplete={onHoldComplete} />);

    const button = screen.getByRole("button", { name: /giữ để xóa/i });
    // button: 2 = chuột phải (theo PointerEvent/MouseEvent.button).
    fireEvent.pointerDown(button, { button: 2 });
    vi.advanceTimersByTime(3000);

    expect(onHoldComplete).not.toHaveBeenCalled();
  });

  it("không gọi callback nếu component unmount giữa lúc đang giữ", () => {
    const onHoldComplete = vi.fn();
    const { unmount } = render(
      <HoldToDeleteButton label="Giữ để xóa" onHoldComplete={onHoldComplete} />,
    );

    const button = screen.getByRole("button", { name: /giữ để xóa/i });
    fireEvent.pointerDown(button);
    vi.advanceTimersByTime(1000);

    unmount();
    vi.advanceTimersByTime(2000);

    expect(onHoldComplete).not.toHaveBeenCalled();
  });

  it("huỷ ngay khi disabled chuyển thành true giữa lúc đang giữ", () => {
    const onHoldComplete = vi.fn();
    const { rerender } = render(
      <HoldToDeleteButton label="Giữ để xóa" disabled={false} onHoldComplete={onHoldComplete} />,
    );

    const button = screen.getByRole("button", { name: /giữ để xóa/i });
    fireEvent.pointerDown(button);
    vi.advanceTimersByTime(1000);

    // Cha khoá nút giữa chừng (Task 10: disabled={busy} khi một lệnh khác đang chạy).
    rerender(
      <HoldToDeleteButton label="Giữ để xóa" disabled={true} onHoldComplete={onHoldComplete} />,
    );

    // Thanh tiến trình phải tắt ngay — người dùng thấy lượt giữ đã bị huỷ,
    // không chỉ mỗi việc callback bị chặn ngầm.
    expect(screen.getByTestId("hold-progress")).toHaveStyle({ width: "0%" });

    vi.advanceTimersByTime(2000);

    expect(onHoldComplete).not.toHaveBeenCalled();
  });

  describe("bàn phím", () => {
    it("giữ đủ Enter thì kích hoạt", () => {
      const onHoldComplete = vi.fn();
      render(<HoldToDeleteButton label="Giữ để xóa" onHoldComplete={onHoldComplete} />);

      const button = screen.getByRole("button", { name: /giữ để xóa/i });
      fireEvent.keyDown(button, { key: "Enter" });
      vi.advanceTimersByTime(2000);

      expect(onHoldComplete).toHaveBeenCalledTimes(1);
    });

    it("giữ đủ phím cách (Space) thì kích hoạt", () => {
      const onHoldComplete = vi.fn();
      render(<HoldToDeleteButton label="Giữ để xóa" onHoldComplete={onHoldComplete} />);

      const button = screen.getByRole("button", { name: /giữ để xóa/i });
      fireEvent.keyDown(button, { key: " " });
      vi.advanceTimersByTime(2000);

      expect(onHoldComplete).toHaveBeenCalledTimes(1);
    });

    it("nhả phím sớm thì huỷ, không gọi callback", () => {
      const onHoldComplete = vi.fn();
      render(<HoldToDeleteButton label="Giữ để xóa" onHoldComplete={onHoldComplete} />);

      const button = screen.getByRole("button", { name: /giữ để xóa/i });
      fireEvent.keyDown(button, { key: "Enter" });
      vi.advanceTimersByTime(1000);
      fireEvent.keyUp(button, { key: "Enter" });
      vi.advanceTimersByTime(2000);

      expect(onHoldComplete).not.toHaveBeenCalled();
    });

    it("rời focus (blur) giữa chừng thì huỷ", () => {
      const onHoldComplete = vi.fn();
      render(<HoldToDeleteButton label="Giữ để xóa" onHoldComplete={onHoldComplete} />);

      const button = screen.getByRole("button", { name: /giữ để xóa/i });
      fireEvent.keyDown(button, { key: "Enter" });
      vi.advanceTimersByTime(1000);
      fireEvent.blur(button);
      vi.advanceTimersByTime(2000);

      expect(onHoldComplete).not.toHaveBeenCalled();
    });

    it("phím tự lặp lại (key repeat) không khởi động lại timer hay gọi callback nhiều lần", () => {
      const onHoldComplete = vi.fn();
      render(<HoldToDeleteButton label="Giữ để xóa" onHoldComplete={onHoldComplete} />);

      const button = screen.getByRole("button", { name: /giữ để xóa/i });
      fireEvent.keyDown(button, { key: "Enter" });
      vi.advanceTimersByTime(1000);
      // Hệ điều hành gửi lại nhiều sự kiện keydown trong lúc giữ phím.
      fireEvent.keyDown(button, { key: "Enter", repeat: true });
      fireEvent.keyDown(button, { key: "Enter", repeat: true });
      vi.advanceTimersByTime(1000);

      expect(onHoldComplete).toHaveBeenCalledTimes(1);
    });

    it("phím khác Enter/Space không kích hoạt gì cả", () => {
      const onHoldComplete = vi.fn();
      render(<HoldToDeleteButton label="Giữ để xóa" onHoldComplete={onHoldComplete} />);

      const button = screen.getByRole("button", { name: /giữ để xóa/i });
      fireEvent.keyDown(button, { key: "Tab" });
      vi.advanceTimersByTime(3000);

      expect(onHoldComplete).not.toHaveBeenCalled();
    });
  });
});
