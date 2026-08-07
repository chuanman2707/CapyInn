import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import RateOverrideField from "./RateOverrideField";

describe("RateOverrideField", () => {
  it("hiện giá engine khi chưa sửa", () => {
    render(
      <RateOverrideField engineTotal={1300000} nights={3} value={null} onChange={vi.fn()} />,
    );
    expect(screen.getByTestId("rate-display").textContent).toContain("1.300.000");
    expect(screen.queryByTestId("rate-input")).toBeNull();
  });

  it("bấm vào giá thì hiện ô nhập, prefill làm tròn xuống bội số 1.000", () => {
    render(
      <RateOverrideField engineTotal={1300000} nights={3} value={null} onChange={vi.fn()} />,
    );
    fireEvent.click(screen.getByTestId("rate-display"));
    // 1.300.000 / 3 = 433.333,33 → làm tròn xuống 433.000
    expect((screen.getByTestId("rate-input") as HTMLInputElement).value).toBe("433000");
  });

  it("gõ giá mới thì báo tổng mới", () => {
    const onChange = vi.fn();
    render(
      <RateOverrideField engineTotal={1300000} nights={3} value={400000} onChange={onChange} />,
    );
    expect(screen.getByTestId("rate-override-total").textContent).toContain("1.200.000");
  });

  it("cảnh báo khi kỳ ở có đêm giá khác nhau", () => {
    render(
      <RateOverrideField engineTotal={1300000} nights={3} value={400000} onChange={vi.fn()} />,
    );
    expect(screen.getByTestId("rate-uneven-warning").textContent).toContain("1.300.000");
  });

  it("không cảnh báo khi giá các đêm đều nhau", () => {
    render(
      <RateOverrideField engineTotal={1200000} nights={3} value={400000} onChange={vi.fn()} />,
    );
    expect(screen.queryByTestId("rate-uneven-warning")).toBeNull();
  });

  it("Về giá gốc trả lại null", () => {
    const onChange = vi.fn();
    render(
      <RateOverrideField engineTotal={1300000} nights={3} value={400000} onChange={onChange} />,
    );
    fireEvent.click(screen.getByRole("button", { name: /về giá gốc/i }));
    expect(onChange).toHaveBeenCalledWith(null);
  });
});
