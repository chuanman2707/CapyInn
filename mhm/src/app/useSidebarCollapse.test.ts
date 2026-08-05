import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useSidebarCollapse } from "./useSidebarCollapse";

function setWidth(width: number) {
  Object.defineProperty(window, "innerWidth", { value: width, configurable: true });
}

describe("useSidebarCollapse", () => {
  beforeEach(() => {
    localStorage.clear();
    setWidth(1456);
  });

  it("mở trợ lý thì thanh thu lại", () => {
    const { result, rerender } = renderHook(({ open }) => useSidebarCollapse(open), {
      initialProps: { open: false },
    });
    expect(result.current.collapsed).toBe(false);

    rerender({ open: true });

    expect(result.current.collapsed).toBe(true);
  });

  it("bấm mở rộng khi trợ lý đang mở thì thanh bung ra", () => {
    const { result } = renderHook(() => useSidebarCollapse(true));

    act(() => result.current.toggleCollapse());

    expect(result.current.collapsed).toBe(false);
  });

  it("đóng trợ lý thì về đúng thiết lập của lễ tân", () => {
    localStorage.setItem("sidebar-collapsed", "true");
    const { result, rerender } = renderHook(({ open }) => useSidebarCollapse(open), {
      initialProps: { open: true },
    });
    act(() => result.current.toggleCollapse());
    expect(result.current.collapsed).toBe(false);

    rerender({ open: false });

    expect(result.current.collapsed).toBe(true);
  });

  /// Bệnh cũ: đường tự động ghi đè thiết lập của lễ tân vĩnh viễn.
  ///
  /// Rình trên `window.localStorage` chứ KHÔNG trên `Storage.prototype`:
  /// `tests/setup.ts` thay `window.localStorage` bằng một object thường
  /// (`Object.getPrototypeOf(...) === Object.prototype`), không phải một thực
  /// thể `Storage`. Rình nhầm prototype thì gián điệp không bao giờ được gọi và
  /// câu `not.toHaveBeenCalled()` xanh với mọi bản cài đặt — kể cả bản đang có
  /// đúng con bug này.
  it("không ghi localStorage lần nào trong cả ba bước trên", () => {
    // `setItem` của setup vốn đã là `vi.fn`, nên `vi.spyOn` trả lại chính nó
    // kèm nguyên sổ gọi của các test chạy trước trong file. Không xoá sổ thì
    // test này đỏ vì lịch sử của hàng xóm, không phải vì cài đặt sai.
    const setItem = vi.spyOn(window.localStorage, "setItem");
    setItem.mockClear();

    const { result, rerender } = renderHook(({ open }) => useSidebarCollapse(open), {
      initialProps: { open: false },
    });

    rerender({ open: true });
    act(() => result.current.toggleCollapse());
    rerender({ open: false });

    expect(setItem).not.toHaveBeenCalled();
  });

  it("cửa sổ hẹp: đóng trợ lý vẫn thu, vì lý do kia còn đúng", () => {
    setWidth(1150);
    const { result, rerender } = renderHook(({ open }) => useSidebarCollapse(open), {
      initialProps: { open: true },
    });

    rerender({ open: false });

    expect(result.current.collapsed).toBe(true);
  });

  it("resize không xoá được thao tác mở rộng bằng tay", () => {
    setWidth(1150);
    const { result } = renderHook(() => useSidebarCollapse(false));
    act(() => result.current.toggleCollapse());
    expect(result.current.collapsed).toBe(false);

    act(() => window.dispatchEvent(new Event("resize")));

    expect(result.current.collapsed).toBe(false);
  });

  /// Bệnh cũ, dựng đúng kịch bản người dùng gặp: thanh đang mở → kéo cửa sổ hẹp
  /// lại → phóng to lại. Lý do tự động hết thì thanh phải bung ra như cũ, và lần
  /// khởi động sau vẫn phải nhớ đúng lựa chọn ấy.
  it("thu nhỏ rồi phóng to lại thì thanh vẫn mở, kể cả sau khi khởi động lại", () => {
    const { result, unmount } = renderHook(() => useSidebarCollapse(false));
    expect(result.current.collapsed).toBe(false);

    act(() => {
      setWidth(1150);
      window.dispatchEvent(new Event("resize"));
    });
    expect(result.current.collapsed).toBe(true);

    act(() => {
      setWidth(1456);
      window.dispatchEvent(new Event("resize"));
    });
    expect(result.current.collapsed).toBe(false);

    unmount();
    const second = renderHook(() => useSidebarCollapse(false));

    expect(second.result.current.collapsed).toBe(false);
  });

  /// Mở lại trợ lý sau khi đã đóng phải thu thanh lần nữa: `userOverride` của
  /// lần trước đã hết hạn dùng, không được sống lại.
  it("mở lại trợ lý thì thanh thu lần nữa, không nhớ lần mở rộng cũ", () => {
    const { result, rerender } = renderHook(({ open }) => useSidebarCollapse(open), {
      initialProps: { open: true },
    });
    act(() => result.current.toggleCollapse());
    expect(result.current.collapsed).toBe(false);

    rerender({ open: false });
    rerender({ open: true });

    expect(result.current.collapsed).toBe(true);
  });
});
