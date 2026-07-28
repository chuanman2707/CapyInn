import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import type { DeclarationBatch } from "@/types";

const invokeCommand = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invokeCommand", () => ({ invokeCommand }));
vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

import ReconcilePanel from "./ReconcilePanel";

function batch(over: Partial<DeclarationBatch>): DeclarationBatch {
  return {
    id: "b1", kind: "VN", file_path: "/x/TBLT.xlsx", row_count: 3,
    status: "exported", verified_count: null, verified_at: null,
    created_at: "2026-07-27T10:00:00+07:00",
    ...over,
  };
}

function mockBatches(batches: DeclarationBatch[]) {
  invokeCommand.mockImplementation((cmd: string) =>
    cmd === "kbtt_list_batches" ? Promise.resolve(batches) : Promise.resolve(null),
  );
}

describe("ReconcilePanel", () => {
  it("thẻ mọc lại từ DB khi mở app — lô exported còn đó là còn thẻ", async () => {
    mockBatches([batch({}), batch({ id: "b2", status: "verified" })]);
    render(<ReconcilePanel reloadKey={0} onSettled={() => {}} />);

    await waitFor(() => expect(screen.getByText(/đối chiếu/i)).toBeTruthy());
    // Chỉ lô chưa xong có thẻ; lô verified thì không.
    expect(screen.getAllByText(/khách việt nam/i)).toHaveLength(1);
    expect(screen.getByText(/vì sao phải đếm tay/i)).toBeTruthy();
  });

  it("gõ đúng số → kbtt_reconcile, báo xanh, gọi onSettled", async () => {
    mockBatches([batch({})]);
    invokeCommand.mockImplementation((cmd: string) => {
      if (cmd === "kbtt_list_batches") return Promise.resolve([batch({})]);
      if (cmd === "kbtt_reconcile") return Promise.resolve("verified");
      return Promise.resolve(null);
    });
    const onSettled = vi.fn();
    render(<ReconcilePanel reloadKey={0} onSettled={onSettled} />);
    await waitFor(() => expect(screen.getByLabelText(/cổng hiện/i)).toBeTruthy());

    fireEvent.change(screen.getByLabelText(/cổng hiện/i), { target: { value: "3" } });
    fireEvent.click(screen.getByRole("button", { name: /chốt/i }));

    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith("kbtt_reconcile", {
        batchId: "b1",
        seenCount: 3,
      }),
    );
    expect(onSettled).toHaveBeenCalled();
  });

  it("lô failed: thẻ đỏ có nút đưa khách về danh sách → kbtt_reopen_batch", async () => {
    mockBatches([batch({ status: "failed", verified_count: 0 })]);
    const onSettled = vi.fn();
    render(<ReconcilePanel reloadKey={0} onSettled={onSettled} />);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /đưa khách về danh sách/i })).toBeTruthy(),
    );

    fireEvent.click(screen.getByRole("button", { name: /đưa khách về danh sách/i }));
    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith("kbtt_reopen_batch", { batchId: "b1" }),
    );
    expect(onSettled).toHaveBeenCalled();
  });

  it("bấm Mở thư mục → kbtt_open_export_dir đúng batch id", async () => {
    mockBatches([batch({})]);
    render(<ReconcilePanel reloadKey={0} onSettled={() => {}} />);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /mở thư mục/i })).toBeTruthy(),
    );

    fireEvent.click(screen.getByRole("button", { name: /mở thư mục/i }));
    await waitFor(() =>
      expect(invokeCommand).toHaveBeenCalledWith("kbtt_open_export_dir", { batchId: "b1" }),
    );
  });

  it("phản ứng không bị ảnh hưởng nếu phản hồi cũ đến sau phản hồi mới", async () => {
    let resolveFirstRequest: ((data: DeclarationBatch[]) => void) = () => {};
    let resolveSecondRequest: ((data: DeclarationBatch[]) => void) = () => {};

    const firstRequestPromise = new Promise<DeclarationBatch[]>((resolve) => {
      resolveFirstRequest = resolve;
    });
    const secondRequestPromise = new Promise<DeclarationBatch[]>((resolve) => {
      resolveSecondRequest = resolve;
    });

    let requestCount = 0;
    invokeCommand.mockImplementation((cmd: string) => {
      if (cmd === "kbtt_list_batches") {
        requestCount++;
        if (requestCount === 1) return firstRequestPromise;
        if (requestCount === 2) return secondRequestPromise;
      }
      return Promise.resolve(null);
    });

    const { rerender } = render(<ReconcilePanel reloadKey={0} onSettled={() => {}} />);
    rerender(<ReconcilePanel reloadKey={1} onSettled={() => {}} />);

    // Phản hồi MỚI về trước: lô "b-new" (khác lô cũ) vẫn còn dở.
    resolveSecondRequest([batch({ id: "b-new", file_path: "/x/NEW.xlsx" })]);
    // Phản hồi CŨ về sau: lô "b-old" — phải bị bỏ qua, không được đè lên state mới.
    resolveFirstRequest([batch({ id: "b-old", file_path: "/x/OLD.xlsx" })]);

    await waitFor(() => expect(screen.getByText("NEW.xlsx")).toBeTruthy());
    expect(screen.queryByText("OLD.xlsx")).toBeFalsy();
  });

  it("không còn lô dở thì panel biến mất", async () => {
    mockBatches([batch({ id: "b9", status: "verified" })]);
    const { container } = render(<ReconcilePanel reloadKey={0} onSettled={() => {}} />);
    await waitFor(() => expect(invokeCommand).toHaveBeenCalled());
    expect(container.textContent).not.toMatch(/đối chiếu/i);
  });
});
