import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { AssistantEmptyState, SUGGESTIONS } from "./AssistantEmptyState";

describe("AssistantEmptyState", () => {
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
