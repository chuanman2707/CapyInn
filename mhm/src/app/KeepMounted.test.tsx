import { useState } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

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

  // FINDING C2: state React sống qua tab switch (test trên) không đồng nghĩa
  // dữ liệu lấy từ server vẫn còn đúng — GuestList/badge/ReconcilePanel bên
  // trong Declaration chỉ tải lại khi `reloadKey` đổi, và không có gì đổi
  // reloadKey khi quay lại tab. `onActivate` là tín hiệu để cha (MainShell)
  // bơm lại reloadKey đúng lúc quay lại tab — không phải mỗi lần re-render,
  // và không phải lần đầu tiên được xem (mount đã tự tải dữ liệu của nó rồi).
  describe("onActivate", () => {
    it("không gọi khi mount lần đầu ở trạng thái active", () => {
      const onActivate = vi.fn();
      render(
        <KeepMounted active onActivate={onActivate}>
          Nội dung
        </KeepMounted>,
      );
      expect(onActivate).not.toHaveBeenCalled();
    });

    it("không gọi khi mount lần đầu ở trạng thái không active", () => {
      const onActivate = vi.fn();
      render(
        <KeepMounted active={false} onActivate={onActivate}>
          Nội dung
        </KeepMounted>,
      );
      expect(onActivate).not.toHaveBeenCalled();
    });

    // Đây chính là kịch bản thật trong app: tab mặc định là Dashboard, nên
    // Declaration luôn mount ở trạng thái KHÔNG active, rồi mới chuyển sang
    // active lần đầu khi người dùng bấm vào tab. Đây KHÔNG phải một lần
    // "quay lại" — mount ngay sau đó đã tự tải dữ liệu qua reloadKey ban đầu
    // của nó; gọi onActivate ở đây chỉ tạo một lượt tải trùng vô ích ngay sau
    // lượt đầu.
    it("KHÔNG gọi ở lần chuyển từ ẩn sang hiện ĐẦU TIÊN (đó là lần xem đầu, không phải quay lại)", () => {
      const onActivate = vi.fn();
      const { rerender } = render(
        <KeepMounted active={false} onActivate={onActivate}>
          Nội dung
        </KeepMounted>,
      );
      expect(onActivate).not.toHaveBeenCalled();

      rerender(
        <KeepMounted active onActivate={onActivate}>
          Nội dung
        </KeepMounted>,
      );
      expect(onActivate).not.toHaveBeenCalled();
    });

    it("gọi đúng một lần ở lần chuyển từ ẩn sang hiện THỨ HAI trở đi (quay lại thật)", () => {
      const onActivate = vi.fn();
      const { rerender } = render(
        <KeepMounted active={false} onActivate={onActivate}>
          Nội dung
        </KeepMounted>,
      );
      rerender(
        <KeepMounted active onActivate={onActivate}>
          Nội dung
        </KeepMounted>,
      );
      expect(onActivate).not.toHaveBeenCalled(); // lần xem đầu — chưa gọi

      rerender(
        <KeepMounted active={false} onActivate={onActivate}>
          Nội dung
        </KeepMounted>,
      );
      rerender(
        <KeepMounted active onActivate={onActivate}>
          Nội dung
        </KeepMounted>,
      );
      expect(onActivate).toHaveBeenCalledTimes(1); // quay lại thật — gọi
    });

    it("không gọi lại khi re-render mà active vẫn giữ nguyên true", () => {
      const onActivate = vi.fn();
      const { rerender } = render(
        <KeepMounted active={false} onActivate={onActivate}>
          Nội dung
        </KeepMounted>,
      );
      rerender(
        <KeepMounted active onActivate={onActivate}>
          Nội dung
        </KeepMounted>,
      );
      rerender(
        <KeepMounted active={false} onActivate={onActivate}>
          Nội dung
        </KeepMounted>,
      );
      rerender(
        <KeepMounted active onActivate={onActivate}>
          Nội dung
        </KeepMounted>,
      );
      expect(onActivate).toHaveBeenCalledTimes(1);

      // Re-render trong lúc vẫn active (vd. cha re-render vì lý do khác) —
      // KHÔNG được gọi thêm, nếu không mỗi lần cha render lại sẽ bơm lại
      // reloadKey và dội backend vô hình.
      rerender(
        <KeepMounted active onActivate={onActivate}>
          <span>đổi children để chắc chắn có re-render</span>
        </KeepMounted>,
      );
      expect(onActivate).toHaveBeenCalledTimes(1);
    });

    it("gọi lại đúng một lần mỗi khi ẩn rồi hiện lại lần nữa, kể cả khi mount thẳng ở trạng thái active", () => {
      const onActivate = vi.fn();
      const { rerender } = render(
        <KeepMounted active onActivate={onActivate}>
          Nội dung
        </KeepMounted>,
      );
      expect(onActivate).not.toHaveBeenCalled(); // mount thẳng active = lần xem đầu

      rerender(
        <KeepMounted active={false} onActivate={onActivate}>
          Nội dung
        </KeepMounted>,
      );
      rerender(
        <KeepMounted active onActivate={onActivate}>
          Nội dung
        </KeepMounted>,
      );
      expect(onActivate).toHaveBeenCalledTimes(1);

      rerender(
        <KeepMounted active={false} onActivate={onActivate}>
          Nội dung
        </KeepMounted>,
      );
      rerender(
        <KeepMounted active onActivate={onActivate}>
          Nội dung
        </KeepMounted>,
      );
      expect(onActivate).toHaveBeenCalledTimes(2);
    });
  });
});
