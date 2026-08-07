import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import VoidBookingDialog from "./VoidBookingDialog";
import type { VoidBookingPreview } from "@/types";

const invokeCommand = vi.fn();
vi.mock("@/lib/invokeCommand", () => ({
  invokeCommand: (...args: unknown[]) => invokeCommand(...args),
  invokeWriteCommand: (...args: unknown[]) => invokeCommand(...args),
  createIdempotencyKey: () => "test-key",
}));

// `handleVoid` chỉ báo lỗi qua toast (không có banner lỗi riêng trong hộp
// thoại) — mock để test lượt xóa thất bại xác nhận được đúng câu, không chỉ
// đoán từ việc onVoided không được gọi.
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
import { toast } from "sonner";

const basePreview: VoidBookingPreview = {
  booking_id: "B-1",
  guest_name: "Christian Höfer",
  room_id: "4A",
  previous_status: "checked_out",
  revenue_impact: 500000,
  revenue_date: "2026-08-06",
  deposit_amount: 0,
  nights_recognized: 1,
  nights_total: 1,
  is_audited: false,
  room_status_unchanged: false,
  is_group_booking: false,
};

afterEach(() => {
  invokeCommand.mockReset();
  vi.mocked(toast.success).mockReset();
  vi.mocked(toast.error).mockReset();
});

describe("VoidBookingDialog", () => {
  it("hiển thị đúng số tiền backend trả về, không tự tính", async () => {
    invokeCommand.mockResolvedValueOnce(basePreview);

    render(<VoidBookingDialog bookingId="B-1" onClose={vi.fn()} onVoided={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByTestId("void-revenue-impact").textContent).toContain("500.000");
    });
    expect(screen.getByTestId("void-revenue-impact").textContent).toContain("06/08/2026");
  });

  it("cảnh báo khi ngày đã chốt kiểm toán đêm", async () => {
    invokeCommand.mockResolvedValueOnce({ ...basePreview, is_audited: true });

    render(<VoidBookingDialog bookingId="B-1" onClose={vi.fn()} onVoided={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByTestId("void-audited-warning")).toBeTruthy();
    });
  });

  // `room_status_unchanged` chỉ nói "UPDATE rooms sẽ không khớp dòng nào" — hệt
  // nhau dù backend tính ra true vì phòng đang `occupied`, `booked`, hay (ca
  // dưới đây) đã `vacant` sẵn (housekeeping dọn xong trước khi ai đó phát hiện
  // lượt này nhập sai). Dialog không phân biệt được LÝ DO đằng sau cờ này, nên
  // câu chữ không được suy đoán ra một lý do cụ thể — nhất là không được nói
  // "có khách khác" cho một phòng đang trống, và cũng không được nói phòng "về
  // Trống" (nó đã trống rồi, xoá lượt không đổi gì cả).
  it("phòng đã Trống (dọn xong trước khi xoá) vẫn báo giữ nguyên trạng thái, không suy đoán có khách khác", async () => {
    invokeCommand.mockResolvedValueOnce({ ...basePreview, room_status_unchanged: true });

    render(<VoidBookingDialog bookingId="B-1" onClose={vi.fn()} onVoided={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByTestId("void-room-status-unchanged-note")).toBeTruthy();
    });
    const noteText = screen.getByTestId("void-room-status-unchanged-note").textContent ?? "";
    expect(noteText).not.toContain("khách khác");
    expect(noteText).not.toContain("Trống");
  });

  it("không hiện dòng tiền nào khi lượt đặt trước không cọc, không doanh thu", async () => {
    invokeCommand.mockResolvedValueOnce({
      ...basePreview,
      previous_status: "booked",
      revenue_impact: 0,
      deposit_amount: 0,
    });

    render(<VoidBookingDialog bookingId="B-1" onClose={vi.fn()} onVoided={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText(/Christian Höfer/)).toBeTruthy();
    });
    expect(screen.queryByTestId("void-revenue-impact")).toBeNull();
    expect(screen.queryByTestId("void-deposit-note")).toBeNull();
  });

  it("cọc hiện thành dòng riêng, không gộp vào dòng doanh thu", async () => {
    invokeCommand.mockResolvedValueOnce({
      ...basePreview,
      previous_status: "booked",
      revenue_impact: 0,
      deposit_amount: 200000,
    });

    render(<VoidBookingDialog bookingId="B-1" onClose={vi.fn()} onVoided={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByTestId("void-deposit-note").textContent).toContain("200.000");
    });
    // Cọc KHÔNG được nói là "gỡ khỏi doanh thu" — nó chưa bao giờ là doanh thu,
    // và xoá lượt không gỡ nó khỏi sổ thu.
    expect(screen.queryByTestId("void-revenue-impact")).toBeNull();
  });

  // ─── Dòng "phòng về trạng thái Trống" ───
  //
  // `void_booking_tx` (`services/booking/void_lifecycle.rs`) chỉ UPDATE rooms
  // cho status `active`/`checked_out`; nhánh `booked` là `_ => {}` — không đụng
  // bảng rooms. `room_status_unchanged` cũng chỉ được backend tính cho `checked_out`
  // (luôn false với `booked`), nên `!room_status_unchanged` một mình không đủ để
  // quyết định có nên nói "phòng sẽ về trống" hay không.

  it("không báo phòng về trống cho lượt chỉ mới đặt trước — void không đụng tới bảng rooms", async () => {
    invokeCommand.mockResolvedValueOnce({
      ...basePreview,
      previous_status: "booked",
      revenue_impact: 0,
      deposit_amount: 0,
      room_status_unchanged: false,
    });

    render(<VoidBookingDialog bookingId="B-1" onClose={vi.fn()} onVoided={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText(/Christian Höfer/)).toBeTruthy();
    });
    expect(screen.queryByTestId("void-room-vacant-note")).toBeNull();
  });

  it("báo phòng về trống khi lượt đã trả phòng và phòng chưa bị bán lại, không tự thêm cảnh báo nào khác", async () => {
    invokeCommand.mockResolvedValueOnce(basePreview);

    render(<VoidBookingDialog bookingId="B-1" onClose={vi.fn()} onVoided={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByTestId("void-room-vacant-note")).toBeTruthy();
    });
    // basePreview không audited, không thuộc đoàn, phòng không bị bán lại,
    // không cọc — bốn dòng điều kiện còn lại (mỗi dòng chỉ khoá được ở test
    // riêng của nó khi BẬT) đều phải vắng mặt ở đây, khi TẮT.
    expect(screen.queryByTestId("void-group-booking-warning")).toBeNull();
    expect(screen.queryByTestId("void-audited-warning")).toBeNull();
    expect(screen.queryByTestId("void-room-status-unchanged-note")).toBeNull();
    expect(screen.queryByTestId("void-deposit-note")).toBeNull();
  });

  it("khi cờ giữ-nguyên-trạng-thái bật thì không nói thêm 'sẽ về trống' — hai dòng nói hai điều trái nhau", async () => {
    invokeCommand.mockResolvedValueOnce({ ...basePreview, room_status_unchanged: true });

    render(<VoidBookingDialog bookingId="B-1" onClose={vi.fn()} onVoided={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByTestId("void-room-status-unchanged-note")).toBeTruthy();
    });
    expect(screen.queryByTestId("void-room-vacant-note")).toBeNull();
  });

  // ─── Chú thích số đêm đã ghi nhận ───
  //
  // Chỉ có ý nghĩa khi lượt còn đang ở (`active`) — đang tích luỹ dần. Lượt đã
  // trả phòng luôn ghi nhận đủ 100% số đêm nên nói "đã ghi nhận N/N đêm" không
  // sai nhưng thừa; test dưới khoá cả hai chiều để đổi điều kiện là thấy ngay.

  it("hiện số đêm đã ghi nhận khi lượt đang ở (active)", async () => {
    invokeCommand.mockResolvedValueOnce({
      ...basePreview,
      previous_status: "active",
      revenue_impact: 200000,
      nights_recognized: 2,
      nights_total: 5,
    });

    render(<VoidBookingDialog bookingId="B-1" onClose={vi.fn()} onVoided={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByTestId("void-revenue-impact").textContent).toContain("2/5 đêm");
    });
    // active cũng ép phòng về trống ngay khi voided (nhánh active của
    // void_booking_tx), không chỉ riêng checked_out.
    expect(screen.getByTestId("void-room-vacant-note")).toBeTruthy();
  });

  it("không hiện số đêm đã ghi nhận khi lượt đã trả phòng", async () => {
    invokeCommand.mockResolvedValueOnce(basePreview);

    render(<VoidBookingDialog bookingId="B-1" onClose={vi.fn()} onVoided={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByTestId("void-revenue-impact")).toBeTruthy();
    });
    expect(screen.getByTestId("void-revenue-impact").textContent).not.toContain("đêm");
  });

  // ─── Lượt thuộc đoàn ───
  //
  // `void_booking_tx` từ chối thẳng mọi booking có `group_id` ("Lượt này thuộc
  // đoàn — chưa hỗ trợ xóa từng phòng"), không điều kiện. Không khoá nút ở đây
  // thì người dùng giữ đủ 2 giây chỉ để nhận một lỗi mà preview đã biết trước.

  it("chặn nút xóa và cảnh báo khi lượt thuộc một đoàn", async () => {
    invokeCommand.mockResolvedValueOnce({ ...basePreview, is_group_booking: true });

    render(<VoidBookingDialog bookingId="B-1" onClose={vi.fn()} onVoided={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByTestId("void-group-booking-warning")).toBeTruthy();
    });
    expect(screen.getByRole("button", { name: /không thể xóa/i })).toBeDisabled();
  });

  // ─── Tải xem trước thất bại ───

  it("hiện đúng câu backend trả về khi tải xem trước thất bại, không phải lỗi chung chung", async () => {
    invokeCommand.mockRejectedValueOnce({
      code: "BOOKING_NOT_FOUND",
      message: "Không tìm thấy lượt này — vui lòng tải lại trang",
      kind: "user",
      support_id: null,
    });

    render(<VoidBookingDialog bookingId="B-1" onClose={vi.fn()} onVoided={vi.fn()} />);

    await waitFor(() => {
      expect(screen.getByText("Không tìm thấy lượt này — vui lòng tải lại trang")).toBeTruthy();
    });
  });

  // ─── Giữ nút tới khi hoàn tất ───
  //
  // Thời gian giữ thật (2s) của HoldToDeleteButton đã có bộ test riêng (Task
  // 9); ở đây chỉ xác nhận phần việc của VoidBookingDialog: gọi voidBooking
  // đúng lý do đang chọn, khoá nút trong lúc đợi, và xử lý cả hai kết cục.

  it(
    "giữ đủ 2 giây: gọi voidBooking với lý do đang chọn, khóa nút trong lúc chờ, gọi onVoided khi xong",
    async () => {
      invokeCommand.mockResolvedValueOnce(basePreview);
      let resolveVoid!: (value: unknown) => void;
      invokeCommand.mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            resolveVoid = resolve;
          }),
      );

      const onVoided = vi.fn();
      render(<VoidBookingDialog bookingId="B-1" onClose={vi.fn()} onVoided={onVoided} />);

      await waitFor(() => {
        expect(screen.getByTestId("void-revenue-impact")).toBeTruthy();
      });

      fireEvent.pointerDown(screen.getByRole("button", { name: /Giữ 2 giây để xóa/ }));

      await waitFor(
        () => {
          expect(screen.getByRole("button", { name: /Đang xóa/ })).toBeDisabled();
        },
        { timeout: 3000 },
      );
      expect(onVoided).not.toHaveBeenCalled();

      resolveVoid({
        ok: true,
        booking_id: "B-1",
        room_id: "4A",
        previous_status: "checked_out",
        voided_at: "2026-08-06T10:00:00+07:00",
      });

      await waitFor(() => {
        expect(onVoided).toHaveBeenCalledTimes(1);
      });
      expect(screen.getByRole("button", { name: /Giữ 2 giây để xóa/ })).not.toBeDisabled();

      expect(invokeCommand).toHaveBeenNthCalledWith(
        2,
        "void_booking",
        expect.objectContaining({
          req: { booking_id: "B-1", reason: "Bấm nhầm" },
        }),
        expect.anything(),
      );
    },
    8000,
  );

  it(
    "lệnh xóa thất bại sau khi giữ đủ: báo lỗi qua toast, không gọi onVoided, mở khóa nút để thử lại",
    async () => {
      invokeCommand.mockResolvedValueOnce(basePreview);
      invokeCommand.mockRejectedValueOnce({
        code: "CONFLICT_INVALID_STATE_TRANSITION",
        message: "Lượt vừa thay đổi bởi thao tác khác — vui lòng tải lại trang",
        kind: "user",
        support_id: null,
      });

      const onVoided = vi.fn();
      render(<VoidBookingDialog bookingId="B-1" onClose={vi.fn()} onVoided={onVoided} />);

      await waitFor(() => {
        expect(screen.getByTestId("void-revenue-impact")).toBeTruthy();
      });

      fireEvent.pointerDown(screen.getByRole("button", { name: /Giữ 2 giây để xóa/ }));

      await waitFor(
        () => {
          expect(toast.error).toHaveBeenCalledWith(
            "Lỗi xóa lượt: Lượt vừa thay đổi bởi thao tác khác — vui lòng tải lại trang",
          );
        },
        { timeout: 3000 },
      );

      expect(onVoided).not.toHaveBeenCalled();
      expect(screen.getByRole("button", { name: /Giữ 2 giây để xóa/ })).not.toBeDisabled();
    },
    8000,
  );
});
