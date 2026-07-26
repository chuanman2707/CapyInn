import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeCommand = vi.hoisted(() => vi.fn());
const toastError = vi.hoisted(() => vi.fn());
const toastSuccess = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invokeCommand", () => ({ invokeCommand }));
vi.mock("sonner", () => ({
  toast: { error: toastError, success: toastSuccess },
}));

import ExportPanel from "./ExportPanel";

function mockCommands(overrides: Record<string, unknown> = {}) {
  invokeCommand.mockImplementation((command: string) => {
    if (command in overrides) {
      return Promise.resolve(overrides[command]);
    }
    switch (command) {
      case "kbtt_validate":
        return Promise.resolve([]);
      case "kbtt_open_export_dir":
        return Promise.resolve(undefined);
      default:
        return Promise.resolve([]);
    }
  });
}

const exportButton = () => screen.getByRole("button", { name: /Xuất file/i });

describe("ExportPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockCommands();
  });

  it("disables export while any blocking finding remains", async () => {
    mockCommands({
      kbtt_validate: [
        { code: "E08", severity: "blocking", link_id: "l1", message: "x" },
      ],
    });

    render(<ExportPanel linkIds={["l1"]} kind="NNN" />);
    fireEvent.click(screen.getByRole("button", { name: /Kiểm tra/i }));

    await waitFor(() => {
      expect(exportButton()).toBeDisabled();
    });
    expect(await screen.findByText("E08")).toBeInTheDocument();
  });

  it("allows export when only warnings remain", async () => {
    mockCommands({
      kbtt_validate: [
        { code: "W03", severity: "warning", link_id: "l1", message: "x" },
      ],
    });

    render(<ExportPanel linkIds={["l1"]} kind="VN" />);
    fireEvent.click(screen.getByRole("button", { name: /Kiểm tra/i }));

    await waitFor(() => {
      expect(exportButton()).toBeEnabled();
    });
  });

  it("keeps export disabled without ever offering a bypass", async () => {
    mockCommands({
      kbtt_validate: [
        { code: "E08", severity: "blocking", link_id: "l1", message: "x" },
      ],
    });

    render(<ExportPanel linkIds={["l1"]} kind="NNN" />);

    await waitFor(() => {
      expect(exportButton()).toBeDisabled();
    });

    // không có nút nào bỏ qua lỗi chặn
    const labels = screen.getAllByRole("button").map((b) => b.textContent ?? "");
    expect(
      labels.some((t) => /bỏ qua|xuất tạm|vẫn xuất|ép xuất|force/i.test(t)),
    ).toBe(false);

    fireEvent.click(exportButton());
    await waitFor(() => {
      expect(invokeCommand).not.toHaveBeenCalledWith(
        "kbtt_export",
        expect.anything(),
      );
    });
  });

  it("shows the reconcile checklist after a successful export", async () => {
    mockCommands({
      kbtt_export: {
        batch_id: "b1",
        file_path: "/x/y.xlsx",
        row_count: 3,
        kind: "VN",
      },
    });

    render(<ExportPanel linkIds={["l1", "l2", "l3"]} kind="VN" />);

    await waitFor(() => {
      expect(exportButton()).toBeEnabled();
    });
    fireEvent.click(exportButton());

    expect(
      await screen.findByLabelText(/Số hồ sơ thấy trên cổng/i),
    ).toBeInTheDocument();
    await waitFor(() => {
      expect(invokeCommand).toHaveBeenCalledWith("kbtt_export", {
        kind: "VN",
        linkIds: ["l1", "l2", "l3"],
      });
    });
    expect(invokeCommand).toHaveBeenCalledWith("kbtt_open_export_dir", {
      batchId: "b1",
    });
  });

  it("does nothing when there is nothing selected", async () => {
    render(<ExportPanel linkIds={[]} kind="VN" />);

    await waitFor(() => {
      expect(exportButton()).toBeDisabled();
    });
  });
});
