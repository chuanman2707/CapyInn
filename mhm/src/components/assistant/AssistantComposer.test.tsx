import { fireEvent, render, screen } from "@testing-library/react";
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
