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

  // Đổi tên từ "cảnh báo khi kỳ ở có đêm giá khác nhau" (rà cuối trước
  // merge, M-a): fixture của test này CHƯA BAO GIỜ thử một kỳ có đêm giá
  // khác nhau — nó chỉ là một khoản giảm giá thường (400.000 thay vì
  // 433.333/đêm). Cái tên cũ khẳng định một nguyên nhân mà chính test không
  // hề kiểm — đúng lớp lỗi component thật đang mắc.
  it("cảnh báo trung tính khi tổng giá tay khác tổng engine, không suy đoán nguyên nhân", () => {
    render(
      <RateOverrideField engineTotal={1300000} nights={3} value={400000} onChange={vi.fn()} />,
    );
    const text = screen.getByTestId("rate-uneven-warning").textContent ?? "";
    expect(text).toContain("1.300.000");
    // Component không biết các đêm có đều giá hay không — nó chỉ có tổng.
    // Không được khẳng định nguyên nhân "cuối tuần/lễ" hay "đêm giá khác nhau".
    expect(text).not.toContain("cuối tuần");
    expect(text).not.toContain("lễ");
    expect(text).not.toContain("khác nhau");
  });

  // Ca thật reviewer chạy: kỳ giá PHẲNG (1.200.000/3 đêm = 400.000 đều nhau),
  // lễ tân CHỦ Ý giảm còn 350.000/đêm. Bản cũ vẫn hiện "Kỳ này có đêm giá
  // khác nhau (cuối tuần/lễ)" — sai sự thật, vì các đêm giá đều hệt nhau.
  // Frontend không có dữ liệu để phân biệt "kỳ phẳng bị giảm giá" với "kỳ
  // lệch giá đặt một mức", nên câu chữ phải trung tính cho cả hai.
  it("kỳ giá phẳng bị giảm giá cố ý vẫn cảnh báo trung tính, không bịa ra lý do cuối tuần/lễ", () => {
    render(
      <RateOverrideField engineTotal={1200000} nights={3} value={350000} onChange={vi.fn()} />,
    );
    const text = screen.getByTestId("rate-uneven-warning").textContent ?? "";
    expect(text).toContain("1.200.000");
    expect(text).toContain("1.050.000");
    expect(text).not.toContain("cuối tuần");
    expect(text).not.toContain("lễ");
    expect(text).not.toContain("khác nhau");
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

  // M-d (rà cuối trước merge): input chỉ có data-testid, không có tên khả
  // truy cập nào — trình đọc màn hình không nói được đây là ô gì.
  it("ô nhập giá và nút hiện giá có aria-label tiếng Việt", () => {
    render(
      <RateOverrideField engineTotal={1300000} nights={3} value={null} onChange={vi.fn()} />,
    );
    expect(screen.getByTestId("rate-display").getAttribute("aria-label")).toBeTruthy();

    fireEvent.click(screen.getByTestId("rate-display"));
    expect(screen.getByTestId("rate-input").getAttribute("aria-label")).toBeTruthy();
  });

  // M-1 (review Task 17): engineTotal null nghĩa là chưa có gì để prefill
  // (đang tải, hoặc preview vừa lỗi). Trước bản vá, bấm vào lúc này gọi
  // prefillRate(null, nights) = 0 và âm thầm gửi giá 0₫/đêm.
  it("không cho bấm khi chưa có giá engine để prefill (đang tải hoặc lỗi)", () => {
    const onChange = vi.fn();
    render(
      <RateOverrideField engineTotal={null} nights={3} value={null} onChange={onChange} />,
    );
    const button = screen.getByTestId("rate-display");
    expect(button).toBeDisabled();

    fireEvent.click(button);

    expect(onChange).not.toHaveBeenCalled();
    expect(screen.queryByTestId("rate-input")).toBeNull();
  });
});
