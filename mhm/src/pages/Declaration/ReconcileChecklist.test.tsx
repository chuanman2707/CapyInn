import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeCommand = vi.hoisted(() => vi.fn());
const toastError = vi.hoisted(() => vi.fn());
const toastSuccess = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invokeCommand", () => ({ invokeCommand }));
vi.mock("sonner", () => ({
  toast: { error: toastError, success: toastSuccess },
}));

import ReconcileChecklist from "./ReconcileChecklist";

describe("ReconcileChecklist", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invokeCommand.mockResolvedValue("verified");
  });

  it("has no one-click completion button", () => {
    render(<ReconcileChecklist batchId="b1" expected={3} />);

    const buttons = screen.getAllByRole("button").map((b) => b.textContent ?? "");
    expect(
      buttons.some((t) =>
        /hoàn thành|xong|đã upload xong|đánh dấu đã khai/i.test(t),
      ),
    ).toBe(false);

    // con số phải được gõ vào
    expect(screen.getByLabelText(/Số hồ sơ thấy trên cổng/i)).toBeInTheDocument();
  });

  it("keeps the confirm button disabled until a number is typed", () => {
    render(<ReconcileChecklist batchId="b1" expected={3} />);

    const confirm = screen.getByRole("button", { name: /Xác nhận/i });
    expect(confirm).toBeDisabled();

    fireEvent.change(screen.getByLabelText(/Số hồ sơ thấy trên cổng/i), {
      target: { value: "3" },
    });
    expect(confirm).toBeEnabled();
  });

  it("does not pre-fill the expected count into the input", () => {
    render(<ReconcileChecklist batchId="b1" expected={3} />);

    const input = screen.getByLabelText(
      /Số hồ sơ thấy trên cổng/i,
    ) as HTMLInputElement;
    expect(input.value).toBe("");
  });

  it("ticking the reminder checkboxes alone cannot close the batch", () => {
    render(<ReconcileChecklist batchId="b1" expected={3} />);

    for (const box of screen.getAllByRole("checkbox")) {
      fireEvent.click(box);
    }

    expect(screen.getByRole("button", { name: /Xác nhận/i })).toBeDisabled();
    expect(invokeCommand).not.toHaveBeenCalled();
  });

  it("marks the batch verified when the typed count matches", async () => {
    invokeCommand.mockResolvedValue("verified");
    render(<ReconcileChecklist batchId="b1" expected={3} />);

    fireEvent.change(screen.getByLabelText(/Số hồ sơ thấy trên cổng/i), {
      target: { value: "3" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Xác nhận/i }));

    await waitFor(() => {
      expect(invokeCommand).toHaveBeenCalledWith("kbtt_reconcile", {
        batchId: "b1",
        seenCount: 3,
      });
    });
    expect(await screen.findByText(/Lô đã đối chiếu/i)).toBeInTheDocument();
  });

  it("marks the batch failed and tells the operator guests are held on this batch, not undeclared-list, when the count differs", async () => {
    invokeCommand.mockResolvedValue("failed");
    render(<ReconcileChecklist batchId="b1" expected={3} />);

    fireEvent.change(screen.getByLabelText(/Số hồ sơ thấy trên cổng/i), {
      target: { value: "0" },
    });
    fireEvent.click(screen.getByRole("button", { name: /Xác nhận/i }));

    expect(await screen.findByText(/Lô thất bại/i)).toBeInTheDocument();
    expect(
      screen.getByText(/giữ nguyên trên lô này/i),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/đưa khách về danh sách để sửa/i),
    ).toBeInTheDocument();
    await waitFor(() => {
      expect(invokeCommand).toHaveBeenCalledWith("kbtt_reconcile", {
        batchId: "b1",
        seenCount: 0,
      });
    });
  });
});
