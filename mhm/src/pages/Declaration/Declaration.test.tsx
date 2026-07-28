import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeCommand = vi.hoisted(() => vi.fn());
const toastError = vi.hoisted(() => vi.fn());
const toastSuccess = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invokeCommand", () => ({ invokeCommand }));
vi.mock("sonner", () => ({
  toast: { error: toastError, success: toastSuccess },
}));

import Declaration from "./index";

describe("Declaration page", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invokeCommand.mockImplementation((command: string) => {
      switch (command) {
        case "kbtt_pending_rows":
        case "kbtt_list_stays":
        case "kbtt_validate":
        case "kbtt_list_batches":
          return Promise.resolve([]);
        case "kbtt_undeclared_count":
          return Promise.resolve(0);
        default:
          return Promise.resolve([]);
      }
    });
  });

  it("trang rỗng: danh sách trống và nút xuất mờ", async () => {
    invokeCommand.mockImplementation((cmd: string) => {
      if (cmd === "kbtt_pending_rows") return Promise.resolve([]);
      if (cmd === "kbtt_list_stays") return Promise.resolve([]);
      if (cmd === "kbtt_validate") return Promise.resolve([]);
      if (cmd === "kbtt_list_batches") return Promise.resolve([]);
      return Promise.resolve(null);
    });
    render(<Declaration />);

    await waitFor(() =>
      expect(screen.getByText(/chưa khai báo \(0\)/i)).toBeTruthy(),
    );
    const button = screen.getByRole("button", { name: /xuất file/i });
    expect((button as HTMLButtonElement).disabled).toBe(true);
  });
});
