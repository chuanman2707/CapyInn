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

  // Sáu test trên chỉ đọc giá trị input được prefill hoặc truyền sẵn qua prop
  // — không test nào thật sự gõ vào ô input. Ba test dưới đây bơm sự kiện
  // change trực tiếp lên rate-input để phủ chính đường mà người dùng gõ giá,
  // gồm cả dòng bảo toàn tiền nguyên (Math.trunc) mà sáu test trên không chạm tới.

  it("gõ số thập phân thì cắt về số nguyên trước khi báo lên onChange", () => {
    const onChange = vi.fn();
    render(
      <RateOverrideField engineTotal={1300000} nights={3} value={400000} onChange={onChange} />,
    );
    fireEvent.change(screen.getByTestId("rate-input"), {
      target: { value: "500000.7" },
    });
    // Tiền là số nguyên VND — phần thập phân phải bị cắt trước khi ra khỏi component.
    expect(onChange).toHaveBeenCalledWith(500000);
  });

  it("xoá trắng ô nhập thì báo 0, không phải null (null chỉ dành cho nút Về giá gốc)", () => {
    const onChange = vi.fn();
    render(
      <RateOverrideField engineTotal={1300000} nights={3} value={400000} onChange={onChange} />,
    );
    fireEvent.change(screen.getByTestId("rate-input"), { target: { value: "" } });
    expect(onChange).toHaveBeenCalledWith(0);
  });

  it("gõ số âm thì báo nguyên số âm lên — component không tự chặn, backend sẽ từ chối", () => {
    const onChange = vi.fn();
    render(
      <RateOverrideField engineTotal={1300000} nights={3} value={400000} onChange={onChange} />,
    );
    fireEvent.change(screen.getByTestId("rate-input"), { target: { value: "-500" } });
    expect(onChange).toHaveBeenCalledWith(-500);
  });
});
