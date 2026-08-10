import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { AssistantEmptyState, SUGGESTIONS } from "./AssistantEmptyState";

describe("AssistantEmptyState", () => {
  /// Ba câu spec chốt "đúng nguyên văn" (dòng 152-156), gõ lại ở đây.
  ///
  /// Test dưới đây so nút đã vẽ với `SUGGESTIONS` — tức so mã với **chính hằng
  /// số nó import**. Đo được: đổi `SUGGESTIONS[2]` thành "Giúp tôi nhận phòng"
  /// để lại **88/88 xanh**. Nó canh được "component vẽ đủ ba nút", nhưng không
  /// canh được "ba câu đúng là ba câu spec chốt" — mà đó mới là thứ spec chốt.
  ///
  /// Khuôn lấy từ chính nhánh này ở phía Rust
  /// (`the_ceilings_are_the_numbers_the_spec_pinned`, commit `4eace69`): hằng số
  /// spec ghim thì phải có một chỗ **gõ lại con số/câu chữ**, không đọc từ mã
  /// sản xuất. Trước bản này kỷ luật ấy chỉ áp một nửa nhánh.
  ///
  /// Câu ba đặc biệt đáng ghim: nó cố ý **không** dựng được thẻ ngay
  /// (`draft.rs` đòi `room_id`, danh sách khách không rỗng, `nights >= 1`), nên
  /// trợ lý sẽ hỏi lại. Sửa nó thành một câu nghe "hoàn chỉnh" hơn là đổi một
  /// lựa chọn thiết kế đã cân nhắc, không phải sửa chính tả.
  const SPEC_SUGGESTIONS = [
    "Tối nay còn phòng nào trống?",
    "Hôm nay những phòng nào phải trả?",
    "Nhận phòng giúp tôi",
  ];

  it("ba câu gợi ý đúng nguyên văn spec", () => {
    expect([...SUGGESTIONS]).toEqual(SPEC_SUGGESTIONS);
  });

  it("bấm gợi ý là GỬI câu đó, không phải điền vào ô nhập", async () => {
    const onPick = vi.fn();
    render(<AssistantEmptyState onPick={onPick} />);

    await userEvent.click(screen.getByRole("button", { name: SUGGESTIONS[0] }));

    expect(onPick).toHaveBeenCalledWith(SUGGESTIONS[0]);
  });

  it("vẽ đúng ba gợi ý, không câu nào chứa số phòng cụ thể", () => {
    // Khẳng định trên NÚT ĐÃ VẼ chứ không chỉ trên hằng số: hằng có ba câu mà
    // component chỉ vẽ hai (hoặc vẽ chuỗi cứng khác) thì test vẫn phải đỏ.
    render(<AssistantEmptyState onPick={vi.fn()} />);

    const buttons = screen.getAllByRole("button");
    expect(buttons.map((button) => button.textContent)).toEqual([...SUGGESTIONS]);
    expect(buttons).toHaveLength(3);
    for (const button of buttons) {
      expect(button.textContent).not.toMatch(/\d{3}/);
    }
  });
});
