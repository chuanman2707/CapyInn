import { render, screen, waitFor, within } from "@testing-library/react";
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

  it("renders all four blocks", async () => {
    render(<Declaration />);

    await waitFor(() => {
      expect(screen.getByText(/Kéo ảnh giấy tờ vào đây/i)).toBeInTheDocument();
    });
    expect(screen.getByText(/Cần khai báo/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Xuất file/i })).toBeInTheDocument();
    expect(screen.getByText(/Lịch sử lô/i)).toBeInTheDocument();
  });

  it("always shows the Excel warning", async () => {
    render(<Declaration />);

    await waitFor(() => {
      expect(
        screen.getByText(/Không mở\/sửa file này bằng Excel trước khi upload/i),
      ).toBeInTheDocument();
    });
    expect(
      screen.getByText(
        /Excel sẽ làm mất số 0 đầu của số giấy tờ và đổi định dạng ngày/i,
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/Cần sửa thì sửa trong CapyInn rồi xuất lại/i),
    ).toBeInTheDocument();
  });

  it("gives the Excel warning no dismiss control", async () => {
    const { container } = render(<Declaration />);

    await waitFor(() => {
      expect(container.querySelector("[data-excel-warning]")).not.toBeNull();
    });

    const warning = container.querySelector("[data-excel-warning]") as HTMLElement;
    expect(within(warning).queryAllByRole("button")).toHaveLength(0);
    expect(warning.querySelectorAll("button, [role='button'], input")).toHaveLength(0);
  });
});
