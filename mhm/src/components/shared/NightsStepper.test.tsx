import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import NightsStepper from "./NightsStepper";

const baseProps = {
  canShorten: true,
  shortenDisabledReason: "Đêm cuối là hôm nay — dùng Check-out",
  busy: false,
  onShorten: vi.fn(),
  onExtend: vi.fn(),
};

describe("NightsStepper", () => {
  it("gọi onShorten khi bấm nút −1 đêm", async () => {
    const onShorten = vi.fn();
    render(<NightsStepper {...baseProps} onShorten={onShorten} />);

    await userEvent.click(screen.getByRole("button", { name: /−1 đêm/ }));

    expect(onShorten).toHaveBeenCalledTimes(1);
  });

  it("khoá nút −1 đêm khi không được phép rút", () => {
    render(<NightsStepper {...baseProps} canShorten={false} />);

    const shorten = screen.getByRole("button", { name: /−1 đêm/ });
    expect(shorten).toBeDisabled();
    expect(shorten).toHaveAttribute(
      "title",
      "Đêm cuối là hôm nay — dùng Check-out",
    );
  });

  it("khoá cả hai nút khi đang gửi lệnh", () => {
    render(<NightsStepper {...baseProps} busy />);

    expect(screen.getByRole("button", { name: /−1 đêm/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: /\+1 đêm/ })).toBeDisabled();
  });
});
