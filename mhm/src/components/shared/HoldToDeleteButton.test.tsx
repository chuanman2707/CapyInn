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

  it("kích hoạt đúng một lần khi giữ đủ 2 giây", () => {
    const onHoldComplete = vi.fn();
    render(<HoldToDeleteButton label="Giữ để xóa" onHoldComplete={onHoldComplete} />);

    const button = screen.getByRole("button", { name: /giữ để xóa/i });
    fireEvent.pointerDown(button);
    vi.advanceTimersByTime(2000);
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
});
