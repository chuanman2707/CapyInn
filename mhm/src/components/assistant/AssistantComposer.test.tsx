import { createEvent, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { AssistantComposer } from "./AssistantComposer";

describe("AssistantComposer", () => {
  it("Enter gửi, Shift+Enter xuống dòng", () => {
    const onSubmit = vi.fn();
    render(
      <AssistantComposer
        value="xin chào"
        contextLabel="Đang xem: 201"
        busy={false}
        onChange={vi.fn()}
        onSubmit={onSubmit}
      />,
    );
    const box = screen.getByRole("textbox");

    fireEvent.keyDown(box, { key: "Enter", shiftKey: true });
    expect(onSubmit).not.toHaveBeenCalled();

    fireEvent.keyDown(box, { key: "Enter" });
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it("Enter trong lúc bộ gõ đang dựng chữ thì KHÔNG gửi, và không nuốt phím", () => {
    // Lễ tân gõ Telex bấm Enter để CHỐT dấu, không phải để gửi. Bản cũ
    // (<form> + <input>) được trình duyệt che hộ vì implicit submission bỏ qua
    // cú Enter chốt IME; bắt keydown bằng tay thì phải tự kiểm.
    //
    // `fireEvent.keyDown` mặc định cho `isComposing` là false, nên phải đặt cờ
    // TƯỜNG MINH — quên thì test này xanh với cả bản không kiểm gì.
    const onSubmit = vi.fn();
    render(
      <AssistantComposer
        value="Nhận phòng cho anh Nam"
        contextLabel="x"
        busy={false}
        onChange={vi.fn()}
        onSubmit={onSubmit}
      />,
    );
    const box = screen.getByRole("textbox");

    // Và cú Enter đó phải đi tiếp tới bộ gõ. Bản "preventDefault trước rồi mới
    // xét isComposing" nuốt mất phím chốt dấu — hỏng nặng hơn lỗi đang sửa: lễ
    // tân không bỏ dấu được nữa. Nên khẳng định luôn defaultPrevented.
    const composing = createEvent.keyDown(box, { key: "Enter", isComposing: true });
    fireEvent(box, composing);
    expect(onSubmit).not.toHaveBeenCalled();
    expect(composing.defaultPrevented).toBe(false);

    // Cờ cũ của bộ gõ, cho engine chưa dựng `isComposing`. Bắn RIÊNG (isComposing
    // vẫn false) để hai nhánh không che nhau: bản chỉ kiểm một trong hai sẽ đỏ.
    fireEvent.keyDown(box, { key: "Enter", keyCode: 229 });
    expect(onSubmit).not.toHaveBeenCalled();

    // Vế dương: bộ gõ nhả chữ xong, Enter lúc này đúng là "gửi". Thiếu vế này
    // thì một bản chặn sạch mọi cú Enter cũng xanh.
    fireEvent.keyDown(box, { key: "Enter", isComposing: false });
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it("hộp cao theo nội dung, và co lại khi bớt chữ", () => {
    // jsdom KHÔNG tính layout: `scrollHeight` thật luôn là 0, nên test kiểu "gõ
    // dài thì hộp cao lên" đo trên jsdom trần sẽ xanh với cả bản không làm gì.
    // Ở đây dựng một `scrollHeight` giả mô phỏng đúng chỗ khó của trình duyệt
    // thật: nó không bao giờ trả về giá trị NHỎ HƠN chiều cao đang bị
    // `style.height` ghim. Nhờ vậy bản "đo rồi đặt nhưng quên reset về auto" —
    // hộp phình ra rồi không co lại — đỏ ở khẳng định cuối, còn bản không có
    // effect nào đỏ ở khẳng định đầu.
    //
    // Test này chứng minh THUẬT TOÁN đo-rồi-đặt là đúng. Nó KHÔNG chứng minh
    // hộp thật sự cao lên trên WebKit, cũng KHÔNG chứng minh `max-h-40` chặn ở
    // 160px — hai thứ đó cần một trình duyệt thật.
    const props = { contextLabel: "x", busy: false, onChange: vi.fn(), onSubmit: vi.fn() };
    const { rerender } = render(<AssistantComposer value="một dòng" {...props} />);
    const box = screen.getByRole("textbox") as HTMLTextAreaElement;

    let contentHeight = 20;
    Object.defineProperty(box, "scrollHeight", {
      configurable: true,
      get: () => {
        const pinned = Number.parseInt(box.style.height, 10);
        return Number.isNaN(pinned) ? contentHeight : Math.max(pinned, contentHeight);
      },
    });

    contentHeight = 80;
    rerender(<AssistantComposer value={"bốn\ndòng\nchữ\ndài"} {...props} />);
    expect(box.style.height).toBe("80px");

    contentHeight = 20;
    rerender(<AssistantComposer value="một dòng" {...props} />);
    expect(box.style.height).toBe("20px");
  });

  it("Enter không gửi khi đang bận", () => {
    // Nút gửi tắt lúc bận, nhưng phím Enter là đường thứ hai vào cùng chỗ —
    // tắt nút mà quên phím thì lễ tân vẫn bắn được lượt chồng lên lượt đang chờ.
    const onSubmit = vi.fn();
    render(
      <AssistantComposer
        value="có chữ"
        contextLabel="x"
        busy
        onChange={vi.fn()}
        onSubmit={onSubmit}
      />,
    );

    fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter" });

    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("nút gửi tắt khi ô trống hoặc đang bận", () => {
    const { rerender } = render(
      <AssistantComposer
        value=""
        contextLabel="x"
        busy={false}
        onChange={vi.fn()}
        onSubmit={vi.fn()}
      />,
    );
    expect(screen.getByRole("button", { name: "Gửi tin nhắn" })).toBeDisabled();

    rerender(
      <AssistantComposer
        value="có chữ"
        contextLabel="x"
        busy
        onChange={vi.fn()}
        onSubmit={vi.fn()}
      />,
    );
    expect(screen.getByRole("button", { name: "Gửi tin nhắn" })).toBeDisabled();

    rerender(
      <AssistantComposer
        value="có chữ"
        contextLabel="x"
        busy={false}
        onChange={vi.fn()}
        onSubmit={vi.fn()}
      />,
    );
    expect(screen.getByRole("button", { name: "Gửi tin nhắn" })).toBeEnabled();
  });

  it("nhãn ngữ cảnh nằm ở hàng công cụ, không phải dòng chữ trên ô nhập", () => {
    // Chỉ getByText thì bản cũ — dòng 11px nằm TRÊN ô nhập — cũng xanh, nên
    // khẳng định luôn vị trí: nhãn ở cùng hàng với nút gửi, và hàng đó không
    // chứa ô nhập. Đó đúng là "hàng công cụ" mà spec vẽ.
    render(
      <AssistantComposer
        value=""
        contextLabel="Đang xem: 201"
        busy={false}
        onChange={vi.fn()}
        onSubmit={vi.fn()}
      />,
    );

    const label = screen.getByText("Đang xem: 201");
    expect(label.parentElement).toContainElement(
      screen.getByRole("button", { name: "Gửi tin nhắn" }),
    );
    expect(label.parentElement).not.toContainElement(screen.getByRole("textbox"));
  });
});
