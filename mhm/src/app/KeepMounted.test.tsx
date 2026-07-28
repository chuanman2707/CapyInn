import { useState } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import KeepMounted from "./KeepMounted";

function Counter() {
  const [n, setN] = useState(0);
  return (
    <div>
      <p>count: {n}</p>
      <button onClick={() => setN((v) => v + 1)}>bump</button>
    </div>
  );
}

describe("KeepMounted", () => {
  it("chưa từng active thì không render gì cả (không tốn công mount sớm)", () => {
    render(<KeepMounted active={false}>Nội dung</KeepMounted>);
    expect(screen.queryByText("Nội dung")).toBeNull();
  });

  it("active thì render children bình thường", () => {
    render(<KeepMounted active>Nội dung</KeepMounted>);
    expect(screen.getByText("Nội dung")).toBeTruthy();
  });

  // FINDING D — đây là hành vi cốt lõi: chuyển active -> false rồi lại ->
  // true không được unmount children, nếu không state React bên trong (thẻ
  // chưa lưu, form đang mở) mất sạch không báo trước.
  it("active -> false -> true: children KHÔNG bị unmount, state React sống sót", () => {
    const { rerender } = render(
      <KeepMounted active>
        <Counter />
      </KeepMounted>,
    );

    fireEvent.click(screen.getByRole("button", { name: /bump/i }));
    fireEvent.click(screen.getByRole("button", { name: /bump/i }));
    expect(screen.getByText("count: 2")).toBeTruthy();

    rerender(
      <KeepMounted active={false}>
        <Counter />
      </KeepMounted>,
    );
    // Ẩn nhưng vẫn còn trong DOM — chưa unmount.
    expect(document.querySelector(".hidden")).not.toBeNull();
    expect(screen.getByText("count: 2")).toBeTruthy();

    rerender(
      <KeepMounted active>
        <Counter />
      </KeepMounted>,
    );
    // Quay lại active: vẫn là 2, không reset về 0 như khi unmount/remount.
    expect(screen.getByText("count: 2")).toBeTruthy();
  });
});
