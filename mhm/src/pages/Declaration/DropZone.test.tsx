import { act, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { emitTestEvent, resetEventMocks } from "@/__mocks__/tauri-event";
import type { ExtractedIdentity } from "@/types";

const invokeCommand = vi.hoisted(() => vi.fn());
const toastError = vi.hoisted(() => vi.fn());
const toastSuccess = vi.hoisted(() => vi.fn());
const toastInfo = vi.hoisted(() => vi.fn());

vi.mock("@/lib/invokeCommand", () => ({ invokeCommand }));
vi.mock("sonner", () => ({
  toast: { error: toastError, success: toastSuccess, info: toastInfo },
}));

import DropZone from "./DropZone";

const vnExtract: ExtractedIdentity = {
  source: "qr_cccd",
  confidence: "verified",
  identity: {
    id: "i2",
    full_name: "Nguyễn Văn A",
    dob: "1980-05-02",
    gender: "M",
    nationality_iso3: "VNM",
    doc_no: "058195006173",
    name_confirmed_by_human: false,
  },
  review_hints: [],
  crop_data_url: null,
};

async function dropOneImage() {
  await act(async () => {
    await emitTestEvent("tauri://drag-drop", { type: "drop", paths: ["/tmp/cccd.jpg"] });
  });
}

describe("DropZone", () => {
  beforeEach(() => {
    resetEventMocks();
    vi.clearAllMocks();
    invokeCommand.mockResolvedValue(vnExtract);
  });

  // Kiểm soát: hành vi vốn có phải còn nguyên khi đang là tab đang xem —
  // FINDING I2 chỉ được chặn lúc ẩn, không được chặn lúc đang hiện.
  it("active (mặc định): thả ảnh vẫn gọi kbtt_extract_from_image và thêm thẻ", async () => {
    render(<DropZone />);

    await dropOneImage();

    await waitFor(() =>
      expect(
        invokeCommand.mock.calls.some(([cmd]) => cmd === "kbtt_extract_from_image"),
      ).toBe(true),
    );
    await waitFor(() => expect(screen.getByDisplayValue("Nguyễn Văn A")).toBeInTheDocument());
  });

  it("active=true tường minh: vẫn gọi kbtt_extract_from_image bình thường", async () => {
    render(<DropZone active />);

    await dropOneImage();

    await waitFor(() =>
      expect(
        invokeCommand.mock.calls.some(([cmd]) => cmd === "kbtt_extract_from_image"),
      ).toBe(true),
    );
  });

  // FINDING I2: listener đăng ký một lần, sống suốt phiên do trang Declaration
  // không unmount khi rời tab (KeepMounted). Thả ảnh lúc trang đang ẩn (đứng ở
  // Check-in/Rooms) không được gọi extract — nếu không, thành công thì một thẻ
  // nằm im trên một trang không ai nhìn, thất bại thì toast lạc màn khác.
  it("active=false (trang đang ẩn sau tab khác): thả ảnh KHÔNG gọi kbtt_extract_from_image", async () => {
    render(<DropZone active={false} />);

    await dropOneImage();

    // Chờ một nhịp để chắc chắn không có gì âm thầm chạy nền rồi mới đếm.
    await act(async () => {
      await Promise.resolve();
    });
    expect(
      invokeCommand.mock.calls.some(([cmd]) => cmd === "kbtt_extract_from_image"),
    ).toBe(false);
    expect(screen.queryByDisplayValue("Nguyễn Văn A")).not.toBeInTheDocument();
    expect(toastError).not.toHaveBeenCalled();
  });

  // Listener đăng ký MỘT LẦN (deps rỗng) — active phải được đọc qua ref MỚI
  // NHẤT tại thời điểm thả, không phải giá trị lúc đăng ký. Test này đổi
  // active SAU khi mount, mô phỏng đúng "quay lại tab" mà không remount
  // DropZone (giống hệt thực tế vì trang sống suốt phiên).
  it("active đổi SAU khi mount (quay lại tab) vẫn được tôn trọng — không cần remount", async () => {
    const { rerender } = render(<DropZone active={false} />);

    await dropOneImage();
    await act(async () => {
      await Promise.resolve();
    });
    expect(
      invokeCommand.mock.calls.some(([cmd]) => cmd === "kbtt_extract_from_image"),
    ).toBe(false);

    rerender(<DropZone active />);
    await dropOneImage();

    await waitFor(() =>
      expect(
        invokeCommand.mock.calls.some(([cmd]) => cmd === "kbtt_extract_from_image"),
      ).toBe(true),
    );
  });

  it("đang hiện rồi bị ẩn đi (rời tab): thả ảnh sau đó không còn được xử lý", async () => {
    const { rerender } = render(<DropZone active />);
    rerender(<DropZone active={false} />);

    await dropOneImage();
    await act(async () => {
      await Promise.resolve();
    });
    expect(
      invokeCommand.mock.calls.some(([cmd]) => cmd === "kbtt_extract_from_image"),
    ).toBe(false);
  });
});
