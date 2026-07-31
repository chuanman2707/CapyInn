import type { ButtonHTMLAttributes, HTMLAttributes, ReactNode } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import {
  createAppErrorException,
  formatAppError,
  type AppError,
} from "@/lib/appError";

const invoke = vi.hoisted(() => vi.fn());
const invokeWriteCommand = vi.hoisted(() => vi.fn());
const createCorrelationId = vi.hoisted(() => vi.fn());
const toastError = vi.hoisted(() => vi.fn());
const toastSuccess = vi.hoisted(() => vi.fn());
const fetchRooms = vi.hoisted(() => vi.fn());
const openInvoice = vi.hoisted(() => vi.fn());
const closeInvoice = vi.hoisted(() => vi.fn());
const resetAvailability = vi.hoisted(() => vi.fn());
// The real Zustand store is NOT always a stable reference: fetchRooms() does
// `set({ rooms })` with a brand-new array from a fresh `invoke("get_rooms")`
// call, and it's wired up to fire on unrelated events (any "db-updated" event,
// the MCP agent booking path, etc — see useHotelStore.ts / RuntimeStateProvider).
// ORIGINAL_ROOMS is an immutable snapshot; `rooms` is what the mocked hook
// currently returns. Tests reassign `rooms` to model either case:
//   - untouched (same reference across renders), or
//   - `ORIGINAL_ROOMS.map((r) => ({ ...r }))` (new array, new room objects,
//     equal contents — exactly what a fetchRooms() refresh produces).
const ORIGINAL_ROOMS = vi.hoisted(() => [
  {
    id: "R101",
    name: "R101",
    type: "standard",
    room_type: "standard",
    floor: 1,
    has_balcony: false,
    base_price: 500000,
    max_guests: 2,
    extra_person_fee: 50000,
    status: "vacant",
  },
  {
    id: "R102",
    name: "R102",
    type: "deluxe",
    room_type: "deluxe",
    floor: 2,
    has_balcony: true,
    base_price: 800000,
    max_guests: 5,
    extra_person_fee: 80000,
    status: "vacant",
  },
]);
let rooms = ORIGINAL_ROOMS;

vi.mock("@tauri-apps/api/core", () => ({
  invoke,
}));

vi.mock("@/lib/invokeCommand", () => ({
  invokeWriteCommand,
}));

vi.mock("@/lib/correlationId", () => ({
  createCorrelationId,
}));

vi.mock("@/stores/useHotelStore", () => ({
  useHotelStore: () => ({
    rooms,
    fetchRooms,
  }),
}));

vi.mock("@/hooks/useAvailability", () => ({
  useAvailability: () => ({
    availability: null,
    loading: false,
    reset: resetAvailability,
  }),
}));

vi.mock("@/hooks/useInvoiceDialog", () => ({
  useInvoiceDialog: () => ({
    invoiceOpen: false,
    invoiceData: null,
    invoiceLoading: false,
    openInvoice,
    closeInvoice,
  }),
}));

vi.mock("@/components/ui/sheet", () => ({
  Sheet: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  SheetContent: ({ children, ...props }: HTMLAttributes<HTMLDivElement>) => (
    <div {...props}>{children}</div>
  ),
  SheetHeader: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  SheetTitle: ({ children }: { children: ReactNode }) => <h2>{children}</h2>,
}));

vi.mock("@/components/ui/button", () => ({
  Button: ({
    children,
    ...props
  }: ButtonHTMLAttributes<HTMLButtonElement>) => <button {...props}>{children}</button>,
}));

vi.mock("./InvoiceDialog", () => ({
  default: () => null,
}));

vi.mock("sonner", () => ({
  toast: {
    error: toastError,
    success: toastSuccess,
  },
}));

import ReservationSheet from "./ReservationSheet";

const createReservationError: AppError = {
  code: "BOOKING_INVALID_STATE",
  message: "Room R101 is booked on 2026-04-26. Cannot create reservation.",
  kind: "user",
  support_id: null,
};

describe("ReservationSheet", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invoke.mockResolvedValue(undefined);
    invokeWriteCommand.mockResolvedValue(undefined);
    createCorrelationId.mockReturnValue("COR-5E6F7A8B");
    rooms = ORIGINAL_ROOMS;
  });

  it("uses invokeWriteCommand with scrubbed monitoring context for the create flow", async () => {
    const user = userEvent.setup();
    render(<ReservationSheet open onOpenChange={vi.fn()} />);

    await waitFor(() => {
      expect(fetchRooms).toHaveBeenCalledTimes(1);
    });

    const [roomSelect, sourceSelect] = screen.getAllByRole("combobox");
    const depositInput = screen.getByPlaceholderText("0");

    fireEvent.change(roomSelect, {
      target: { value: "R101" },
    });
    await user.type(screen.getByPlaceholderText("Họ và tên *"), "Nguyen Van A");
    fireEvent.change(screen.getByLabelText(/ngày đến/i), {
      target: { value: "2026-08-06" },
    });
    fireEvent.change(screen.getByLabelText(/ngày đi/i), {
      target: { value: "2026-08-08" },
    });
    fireEvent.change(depositInput, {
      target: { value: "250000" },
    });
    fireEvent.change(sourceSelect, {
      target: { value: "zalo" },
    });
    await user.type(
      screen.getByPlaceholderText("Ghi chú thêm..."),
      "Khách thích tầng cao",
    );

    await user.click(screen.getByRole("button", { name: /đặt phòng/i }));

    expect(createCorrelationId).toHaveBeenCalledTimes(1);
    await waitFor(() => {
      expect(invokeWriteCommand).toHaveBeenCalledWith(
        "create_reservation",
        {
          req: {
            room_id: "R101",
            guest_name: "Nguyen Van A",
            guest_phone: null,
            guest_doc_number: null,
            check_in_date: expect.any(String),
            check_out_date: expect.any(String),
            nights: 2,
            guests: 2,
            deposit_amount: 250000,
            source: "zalo",
            notes: "Khách thích tầng cao",
          },
        },
        {
          correlationId: "COR-5E6F7A8B",
          monitoringContext: {
            nights: 2,
            deposit_present: true,
            source: "zalo",
            notes_present: true,
          },
        },
      );
    });
    expect(invoke).not.toHaveBeenCalledWith("create_reservation", expect.anything());
  });

  it("uses invokeWriteCommand with correlation and monitoring context for the modify flow", async () => {
    const user = userEvent.setup();
    render(
      <ReservationSheet
        open
        onOpenChange={vi.fn()}
        editBooking={{
          id: "B101",
          room_id: "R101",
          guest_name: "Nguyen Van A",
          guest_phone: "0900000000",
          check_in_at: "2026-04-20",
          expected_checkout: "2026-04-22",
          scheduled_checkin: "2026-04-20",
          scheduled_checkout: "2026-04-22",
          nights: 2,
          // Cố ý KHÁC max_guests của R101 (2) — nếu guard `isEditMode` trên
          // effect nạp lại số khách theo phòng (guestsLoadedForRoomId) bị gỡ,
          // effect đó sẽ nạp đè guests = 2 (max_guests của R101), và bài test
          // này sẽ bắt được ngay vì 2 khác 4. Trùng giá trị với max_guests sẽ
          // che mất một guard bị gỡ mất — xem finding 4 của lượt review.
          guests: 4,
          total_price: 1000000,
          source: "phone",
          deposit_amount: 50000,
        }}
      />,
    );

    await waitFor(() => {
      expect(fetchRooms).toHaveBeenCalledTimes(1);
    });

    fireEvent.change(screen.getByLabelText(/ngày đi/i), {
      target: { value: "2026-04-23" },
    });

    await user.click(screen.getByRole("button", { name: /lưu thay đổi/i }));

    expect(createCorrelationId).toHaveBeenCalledTimes(1);
    await waitFor(() => {
      expect(invokeWriteCommand).toHaveBeenCalledWith(
        "modify_reservation",
        {
          req: {
            booking_id: "B101",
            new_check_in_date: "2026-04-20",
            new_check_out_date: "2026-04-23",
            new_nights: 3,
            new_guests: 4,
          },
        },
        {
          correlationId: "COR-5E6F7A8B",
          monitoringContext: {
            nights: 3,
            deposit_present: false,
            source: null,
            notes_present: false,
          },
        },
      );
    });
    expect(invoke).not.toHaveBeenCalledWith("modify_reservation", expect.anything());
  });

  it("sửa một đặt phòng cũ (guests NULL) mà không đụng ô số khách thì gửi new_guests: null, giữ nguyên giá cũ", async () => {
    // Trước bản vá: prefill effect làm `setGuests(editBooking.guests ?? 2)`,
    // nên guests NULL biến thành số 2 trên form, và submit gửi thẳng số đó —
    // một đặt phòng cũ chưa từng có phụ thu bỗng bị tính thêm người. Backend
    // đọc `new_guests: null` là "giữ nguyên số đã lưu"
    // (modify_reservation_tx: `req.new_guests.or(reservation.guests)`), nên
    // gửi null mới đúng là "không đụng gì" khi người dùng không đụng ô này.
    const user = userEvent.setup();
    render(
      <ReservationSheet
        open
        onOpenChange={vi.fn()}
        editBooking={{
          id: "B-LEGACY",
          room_id: "R101",
          guest_name: "Nguyen Van Legacy",
          guest_phone: "0900000000",
          check_in_at: "2026-04-20",
          expected_checkout: "2026-04-22",
          scheduled_checkin: "2026-04-20",
          scheduled_checkout: "2026-04-22",
          nights: 2,
          guests: null,
          total_price: 1000000,
          source: "phone",
          deposit_amount: 50000,
        }}
      />,
    );

    await waitFor(() => {
      expect(fetchRooms).toHaveBeenCalledTimes(1);
    });

    // Chỉ đổi ngày — không đụng vào ô số khách.
    fireEvent.change(screen.getByLabelText(/ngày đi/i), {
      target: { value: "2026-04-23" },
    });

    await user.click(screen.getByRole("button", { name: /lưu thay đổi/i }));

    await waitFor(() => {
      expect(invokeWriteCommand).toHaveBeenCalledWith(
        "modify_reservation",
        expect.objectContaining({
          req: expect.objectContaining({
            booking_id: "B-LEGACY",
            new_guests: null,
          }),
        }),
        expect.anything(),
      );
    });
  });

  it("sửa một đặt phòng cũ (guests NULL) và CÓ đụng ô số khách thì gửi đúng số vừa gõ", async () => {
    const user = userEvent.setup();
    render(
      <ReservationSheet
        open
        onOpenChange={vi.fn()}
        editBooking={{
          id: "B-LEGACY-2",
          room_id: "R101",
          guest_name: "Nguyen Van Legacy",
          guest_phone: "0900000000",
          check_in_at: "2026-04-20",
          expected_checkout: "2026-04-22",
          scheduled_checkin: "2026-04-20",
          scheduled_checkout: "2026-04-22",
          nights: 2,
          guests: null,
          total_price: 1000000,
          source: "phone",
          deposit_amount: 50000,
        }}
      />,
    );

    await waitFor(() => {
      expect(fetchRooms).toHaveBeenCalledTimes(1);
    });

    fireEvent.change(screen.getByLabelText(/số khách/i), {
      target: { value: "3" },
    });

    await user.click(screen.getByRole("button", { name: /lưu thay đổi/i }));

    await waitFor(() => {
      expect(invokeWriteCommand).toHaveBeenCalledWith(
        "modify_reservation",
        expect.objectContaining({
          req: expect.objectContaining({
            booking_id: "B-LEGACY-2",
            new_guests: 3,
          }),
        }),
        expect.anything(),
      );
    });
  });

  it("formats create flow failures with formatAppError", async () => {
    const user = userEvent.setup();
    const error = createAppErrorException(createReservationError, undefined, {
      correlation_id: "COR-5E6F7A8B",
    });
    invokeWriteCommand.mockRejectedValue(error);

    render(<ReservationSheet open onOpenChange={vi.fn()} />);

    await waitFor(() => {
      expect(fetchRooms).toHaveBeenCalledTimes(1);
    });

    const [roomSelect] = screen.getAllByRole("combobox");

    fireEvent.change(roomSelect, {
      target: { value: "R101" },
    });
    await user.type(screen.getByPlaceholderText("Họ và tên *"), "Nguyen Van A");
    await user.click(screen.getByRole("button", { name: /đặt phòng/i }));

    await waitFor(() => {
      expect(invokeWriteCommand).toHaveBeenCalledWith(
        "create_reservation",
        expect.objectContaining({
          req: expect.objectContaining({
            room_id: "R101",
            guest_name: "Nguyen Van A",
          }),
        }),
        {
          correlationId: "COR-5E6F7A8B",
          monitoringContext: {
            nights: 1,
            deposit_present: false,
            source: "phone",
            notes_present: false,
          },
        },
      );
    });
    expect(toastError).toHaveBeenCalledWith(formatAppError(error));
  });

  it("rejects fractional deposit before invoking create reservation", async () => {
    const user = userEvent.setup();
    render(<ReservationSheet open onOpenChange={vi.fn()} />);

    await waitFor(() => {
      expect(fetchRooms).toHaveBeenCalledTimes(1);
    });

    const [roomSelect] = screen.getAllByRole("combobox");
    const depositInput = screen.getByPlaceholderText("0");

    fireEvent.change(roomSelect, {
      target: { value: "R101" },
    });
    await user.type(screen.getByPlaceholderText("Họ và tên *"), "Nguyen Van A");
    fireEvent.change(depositInput, {
      target: { value: "250000.5" },
    });

    await user.click(screen.getByRole("button", { name: /đặt phòng/i }));

    expect(invokeWriteCommand).not.toHaveBeenCalledWith(
      "create_reservation",
      expect.anything(),
      expect.anything(),
    );
    expect(toastError).toHaveBeenCalledWith(
      "deposit_amount must be a safe integer VND value",
    );
  });

  it("để người dùng chọn ngày đi và không còn ô số đêm", async () => {
    // Dòng tổng tiền giờ đến từ engine (calculate_room_price_preview) chứ
    // không còn tự nhân trong component — nạp một breakdown giả phản ánh
    // đúng số đêm đã chọn, để chứng minh ngày tự suy ra số đêm mà không cần
    // ô nhập riêng.
    invoke.mockImplementation(async (command: string) => {
      if (command === "calculate_room_price_preview") {
        return {
          pricing_type: "nightly",
          base_amount: 1_500_000,
          surcharge_amount: 0,
          weekend_amount: 0,
          total: 1_500_000,
          capped: false,
          breakdown: [{ label: "3 đêm x 500.000", amount: 1_500_000 }],
        };
      }
      return { available: true, conflicts: [], max_nights: null };
    });

    render(<ReservationSheet open onOpenChange={vi.fn()} />);

    expect(screen.queryByLabelText(/số đêm/i)).not.toBeInTheDocument();

    const checkOut = screen.getByLabelText(/ngày đi/i) as HTMLInputElement;
    expect(checkOut.readOnly).toBe(false);

    // Dòng tổng tiền chỉ hiện khi đã chọn phòng — đó là chỗ số đêm hiện ra.
    fireEvent.change(screen.getByLabelText(/^phòng$/i), { target: { value: "R101" } });
    fireEvent.change(screen.getByLabelText(/ngày đến/i), {
      target: { value: "2026-08-06" },
    });
    fireEvent.change(checkOut, { target: { value: "2026-08-09" } });

    await waitFor(() => {
      expect(screen.getByText(/3 đêm/i)).toBeInTheDocument();
    });
  });

  it("khoá nút đặt phòng khi ngày đi không sau ngày đến", async () => {
    render(<ReservationSheet open onOpenChange={vi.fn()} />);

    // Điền đủ phòng và tên khách trước, để nút bị khoá *chỉ vì* ngày sai —
    // không điền thì nút vốn đã khoá và test không chứng minh được gì.
    fireEvent.change(screen.getByLabelText(/^phòng$/i), { target: { value: "R101" } });
    fireEvent.change(screen.getByPlaceholderText(/họ và tên/i), {
      target: { value: "Nguyễn Nhật Huy" },
    });
    fireEvent.change(screen.getByLabelText(/ngày đến/i), {
      target: { value: "2026-08-06" },
    });

    const submit = screen.getByRole("button", { name: /đặt phòng/i });
    await waitFor(() => expect(submit).toBeEnabled());

    fireEvent.change(screen.getByLabelText(/ngày đi/i), {
      target: { value: "2026-08-06" },
    });

    await waitFor(() => {
      expect(screen.getByText(/ngày đi phải sau ngày đến/i)).toBeInTheDocument();
    });
    expect(submit).toBeDisabled();
  });

  it("đặt ô ngày đi max = 90 đêm sau ngày đến", async () => {
    render(<ReservationSheet open onOpenChange={vi.fn()} />);

    fireEvent.change(screen.getByLabelText(/ngày đến/i), {
      target: { value: "2026-08-06" },
    });

    const checkOut = screen.getByLabelText(/ngày đi/i) as HTMLInputElement;
    // 90 đêm sau 2026-08-06.
    expect(checkOut.max).toBe("2026-11-04");
  });

  it("từ chối đặt phòng vượt quá 90 đêm dù ô ngày đi cho gõ tay bỏ qua max", async () => {
    // Ô ngày đi cho gõ tay trực tiếp (không còn suy từ ô số đêm có max={90}
    // như trước), nên thuộc tính `max` trên input không chặn được input giả
    // lập qua fireEvent — đúng như một lỗi gõ năm thật (2036 thay vì 2026)
    // sẽ đi vòng qua trình duyệt. handleSubmit phải tự chặn.
    const user = userEvent.setup();
    render(<ReservationSheet open onOpenChange={vi.fn()} />);

    fireEvent.change(screen.getByLabelText(/^phòng$/i), { target: { value: "R101" } });
    await user.type(screen.getByPlaceholderText(/họ và tên/i), "Nguyen Van A");
    fireEvent.change(screen.getByLabelText(/ngày đến/i), {
      target: { value: "2026-08-08" },
    });
    fireEvent.change(screen.getByLabelText(/ngày đi/i), {
      target: { value: "2036-08-08" },
    });

    await user.click(screen.getByRole("button", { name: /đặt phòng/i }));

    expect(toastError).toHaveBeenCalledWith(
      expect.stringMatching(/90 đêm/),
    );
    expect(invokeWriteCommand).not.toHaveBeenCalledWith(
      "create_reservation",
      expect.anything(),
      expect.anything(),
    );
  });

  it("nạp số khách theo phòng và gửi nó khi đặt phòng", async () => {
    render(<ReservationSheet open onOpenChange={vi.fn()} />);

    fireEvent.change(screen.getByLabelText(/^phòng$/i), { target: { value: "R101" } });

    const guests = screen.getByLabelText(/số khách/i) as HTMLInputElement;
    await waitFor(() => expect(guests.value).toBe("2"));

    fireEvent.change(guests, { target: { value: "4" } });
    fireEvent.change(screen.getByPlaceholderText(/họ và tên/i), {
      target: { value: "Nguyễn Nhật Huy" },
    });
    fireEvent.click(screen.getByRole("button", { name: /đặt phòng/i }));

    await waitFor(() => {
      expect(invokeWriteCommand).toHaveBeenCalledWith(
        "create_reservation",
        expect.objectContaining({
          req: expect.objectContaining({ guests: 4 }),
        }),
        expect.anything(),
      );
    });
  });

  it("đổi sang phòng khác thì nạp lại số khách theo phòng mới, dù đã gõ tay số khác", async () => {
    render(<ReservationSheet open onOpenChange={vi.fn()} />);

    fireEvent.change(screen.getByLabelText(/^phòng$/i), { target: { value: "R101" } });

    const guests = screen.getByLabelText(/số khách/i) as HTMLInputElement;
    await waitFor(() => expect(guests.value).toBe("2"));

    // Người dùng gõ tay một số khách khác với max_guests của phòng đang chọn.
    fireEvent.change(guests, { target: { value: "9" } });
    expect(guests.value).toBe("9");

    // Đổi sang phòng khác — đây LÀ một lựa chọn phòng mới của người dùng,
    // nên số khách phải được nạp lại theo phòng mới (ghi đè số đã gõ tay).
    fireEvent.change(screen.getByLabelText(/^phòng$/i), { target: { value: "R102" } });

    await waitFor(() => expect(guests.value).toBe("5"));
  });

  it("hiện tổng tiền do engine trả về, kèm dòng phụ thu", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "calculate_room_price_preview") {
        return {
          pricing_type: "nightly",
          base_amount: 1_000_000,
          surcharge_amount: 0,
          weekend_amount: 0,
          total: 1_200_000,
          capped: false,
          breakdown: [
            { label: "2 đêm x 500.000", amount: 1_000_000 },
            { label: "Phụ thu 2 khách", amount: 200_000 },
          ],
        };
      }
      return { available: true, conflicts: [], max_nights: null };
    });

    render(<ReservationSheet open onOpenChange={vi.fn()} />);
    fireEvent.change(screen.getByLabelText(/^phòng$/i), { target: { value: "R101" } });
    fireEvent.change(screen.getByLabelText(/số khách/i), { target: { value: "4" } });

    await waitFor(() => {
      expect(screen.getByText("Phụ thu 2 khách")).toBeInTheDocument();
      expect(screen.getByText("1.200.000₫")).toBeInTheDocument();
    });
  });

  it("hiện đúng preview.total do engine trả về, không phải tổng cộng dồn breakdown khi bị cap", async () => {
    // total (1.000.000) cố tình khác xa tổng breakdown (2.000.000 + 3.000.000
    // = 5.000.000) — mô phỏng một trường hợp bị cap/rounding ở engine. Nếu
    // component lỡ tự cộng dồn breakdown thay vì dùng preview.total, test này
    // phải bắt được ngay vì hai con số cách nhau rất xa.
    invoke.mockImplementation(async (command: string) => {
      if (command === "calculate_room_price_preview") {
        return {
          pricing_type: "nightly",
          base_amount: 2_000_000,
          surcharge_amount: 3_000_000,
          weekend_amount: 0,
          total: 1_000_000,
          capped: true,
          breakdown: [
            { label: "2 đêm x 1.000.000", amount: 2_000_000 },
            { label: "Phụ thu 3 khách", amount: 3_000_000 },
          ],
        };
      }
      return { available: true, conflicts: [], max_nights: null };
    });

    render(<ReservationSheet open onOpenChange={vi.fn()} />);
    fireEvent.change(screen.getByLabelText(/^phòng$/i), { target: { value: "R101" } });
    fireEvent.change(screen.getByLabelText(/ngày đến/i), {
      target: { value: "2026-08-06" },
    });
    fireEvent.change(screen.getByLabelText(/ngày đi/i), {
      target: { value: "2026-08-08" },
    });

    await waitFor(() => {
      expect(screen.getByText("1.000.000₫")).toBeInTheDocument();
    });
    expect(screen.queryByText("5.000.000₫")).not.toBeInTheDocument();
  });

  it("báo lỗi rõ ràng khi tính giá thất bại, và không còn hiện giá cũ bên cạnh thông báo lỗi", async () => {
    let calls = 0;
    invoke.mockImplementation(async (command: string) => {
      if (command === "calculate_room_price_preview") {
        calls += 1;
        if (calls === 1) {
          return {
            pricing_type: "nightly",
            base_amount: 500_000,
            surcharge_amount: 100_000,
            weekend_amount: 0,
            total: 600_000,
            capped: false,
            breakdown: [
              { label: "1 đêm x 500.000", amount: 500_000 },
              { label: "Phụ thu 1 khách", amount: 100_000 },
            ],
          };
        }
        throw new Error("IPC lỗi giả lập");
      }
      return { available: true, conflicts: [], max_nights: null };
    });

    render(<ReservationSheet open onOpenChange={vi.fn()} />);
    fireEvent.change(screen.getByLabelText(/^phòng$/i), { target: { value: "R101" } });
    fireEvent.change(screen.getByLabelText(/ngày đến/i), {
      target: { value: "2026-08-06" },
    });
    fireEvent.change(screen.getByLabelText(/ngày đi/i), {
      target: { value: "2026-08-07" },
    });

    // Lượt gọi đầu tiên thành công — giá hiện lên bình thường.
    await waitFor(() => {
      expect(screen.getByText("600.000₫")).toBeInTheDocument();
    });

    // Đổi số khách để kích hoạt một lượt gọi mới, lượt này thất bại.
    fireEvent.change(screen.getByLabelText(/số khách/i), { target: { value: "4" } });

    await waitFor(() => {
      expect(screen.getByText(/không tính được giá/i)).toBeInTheDocument();
    });
    expect(screen.queryByText("600.000₫")).not.toBeInTheDocument();
    expect(screen.queryByText("Tổng")).not.toBeInTheDocument();
  });

  it("một fetchRooms() không liên quan (mảng rooms mới, cùng dữ liệu) không được xoá số khách đã gõ tay", async () => {
    const { rerender } = render(<ReservationSheet open onOpenChange={vi.fn()} />);

    fireEvent.change(screen.getByLabelText(/^phòng$/i), { target: { value: "R101" } });

    const guests = screen.getByLabelText(/số khách/i) as HTMLInputElement;
    await waitFor(() => expect(guests.value).toBe("2"));

    // Người dùng gõ tay một số khách khác với max_guests của phòng đang chọn.
    fireEvent.change(guests, { target: { value: "9" } });
    expect(guests.value).toBe("9");

    // Mô phỏng đúng những gì fetchRooms() thật làm khi một sự kiện không liên
    // quan xảy ra ở nơi khác (dọn phòng khác, checkout phòng khác, AI agent
    // đặt một phòng khác...): store trả về một mảng rooms MỚI (identity mới)
    // nhưng cùng nội dung — roomId đang chọn không đổi.
    rooms = ORIGINAL_ROOMS.map((r) => ({ ...r }));
    rerender(<ReservationSheet open onOpenChange={vi.fn()} />);

    const guestsAfterRefresh = screen.getByLabelText(/số khách/i) as HTMLInputElement;
    expect(guestsAfterRefresh.value).toBe("9");
  });

  it("điền sẵn ngày và số đêm khi mở từ calendar (prefillDates)", async () => {
    // Không còn ô "số đêm" riêng (đã bỏ, xem test "để người dùng chọn ngày
    // đi..." phía trên) — số đêm giờ luôn suy ra từ checkIn/checkOut qua
    // nightsBetween(). Nạp breakdown giả phản ánh đúng 3 đêm để chứng minh
    // nights được tính đúng từ cặp ngày prefill, không phải qua ô nhập riêng.
    invoke.mockImplementation(async (command: string) => {
      if (command === "calculate_room_price_preview") {
        return {
          pricing_type: "nightly",
          base_amount: 1_500_000,
          surcharge_amount: 0,
          weekend_amount: 0,
          total: 1_500_000,
          capped: false,
          breakdown: [{ label: "3 đêm x 500.000", amount: 1_500_000 }],
        };
      }
      return { available: true, conflicts: [], max_nights: null };
    });

    render(
      <ReservationSheet
        open
        onOpenChange={vi.fn()}
        preSelectedRoomId="R101"
        prefillDates={{ checkIn: "2026-08-02", checkOut: "2026-08-05" }}
      />,
    );

    const checkIn = screen.getByLabelText(/ngày đến/i) as HTMLInputElement;
    const checkOut = screen.getByLabelText(/ngày đi/i) as HTMLInputElement;
    await waitFor(() => {
      expect(checkIn.value).toBe("2026-08-02");
      expect(checkOut.value).toBe("2026-08-05");
    });
  });

  it("editBooking thắng prefillDates khi component nhận cả hai (edit mode ưu tiên)", async () => {
    render(
      <ReservationSheet
        open
        onOpenChange={vi.fn()}
        prefillDates={{ checkIn: "2026-08-02", checkOut: "2026-08-05" }}
        editBooking={{
          id: "B999",
          room_id: "R101",
          guest_name: "Existing Guest",
          guest_phone: "0900000000",
          check_in_at: "2026-04-20",
          expected_checkout: "2026-04-22",
          scheduled_checkin: "2026-04-20",
          scheduled_checkout: "2026-04-22",
          nights: 2,
          guests: 2,
          total_price: 1000000,
          source: "phone",
          deposit_amount: 50000,
        }}
      />,
    );

    await waitFor(() => {
      expect(fetchRooms).toHaveBeenCalledTimes(1);
    });

    const checkIn = screen.getByLabelText(/ngày đến/i) as HTMLInputElement;
    const checkOut = screen.getByLabelText(/ngày đi/i) as HTMLInputElement;
    expect(checkIn.value).toBe("2026-04-20");
    expect(checkOut.value).toBe("2026-04-22");
  });

  it("một re-render không liên quan với prefillDates object mới (cùng nội dung) không được xoá ngày người dùng vừa sửa", async () => {
    // prefillDates đến từ kéo-thả trên lịch (Task 8) và được component cha
    // dựng inline trong JSX (`prefillDates={... ? { checkIn, checkOut } : undefined}`),
    // nên mỗi lần cha re-render vì lý do bất kỳ sẽ tạo ra một object mới có
    // cùng nội dung. Nếu effect phụ thuộc vào *reference* của prefillDates,
    // nó sẽ chạy lại, rơi vào nhánh `else if (prefillDates)`, và ghi đè ngày
    // người dùng vừa gõ tay trở lại giá trị prefill ban đầu.
    const { rerender } = render(
      <ReservationSheet
        open
        onOpenChange={vi.fn()}
        preSelectedRoomId="R101"
        prefillDates={{ checkIn: "2026-08-02", checkOut: "2026-08-05" }}
      />,
    );

    const checkIn = screen.getByLabelText(/ngày đến/i) as HTMLInputElement;
    await waitFor(() => {
      expect(checkIn.value).toBe("2026-08-02");
    });

    // Người dùng tự sửa ngày đến sau khi sheet đã mở.
    fireEvent.change(checkIn, { target: { value: "2026-08-10" } });
    expect(checkIn.value).toBe("2026-08-10");

    // Re-render với một object prefillDates MỚI nhưng cùng nội dung — đúng
    // những gì component cha (calendar kéo-thả) tạo ra ở mỗi lần render.
    rerender(
      <ReservationSheet
        open
        onOpenChange={vi.fn()}
        preSelectedRoomId="R101"
        prefillDates={{ checkIn: "2026-08-02", checkOut: "2026-08-05" }}
      />,
    );

    const checkInAfterRerender = screen.getByLabelText(/ngày đến/i) as HTMLInputElement;
    expect(checkInAfterRerender.value).toBe("2026-08-10");
  });

  // Ca chủ khách sạn báo: phòng 5A đang có khách, khách trả ngày 3/8. Đặt
  // trước cho 12–13/8 thì 5A không hề xuất hiện trong ô chọn phòng — nó là
  // phòng duy nhất `occupied`, và cũng là phòng duy nhất biến mất.
  //
  // Trạng thái phòng nói về HÔM NAY; đặt phòng trước hỏi về một khoảng ngày
  // khác hẳn. Cửa chặn đúng chiều thời gian là check_availability (theo
  // khoảng ngày) và chốt chặn trong transaction ở create_reservation_tx —
  // danh sách phòng không được đoán thay chúng.
  it.each(["occupied", "cleaning"])(
    "vẫn liệt kê phòng đang %s để đặt trước cho ngày phòng đó đã trống",
    async (status) => {
      rooms = [ORIGINAL_ROOMS[0], { ...ORIGINAL_ROOMS[1], status }];
      render(<ReservationSheet open onOpenChange={vi.fn()} />);

      await waitFor(() => {
        expect(fetchRooms).toHaveBeenCalledTimes(1);
      });

      expect(
        Array.from(
          (screen.getByLabelText(/^Phòng$/) as HTMLSelectElement).options,
        ).map((o) => o.value),
      ).toContain("R102");
    },
  );

  // Ô lịch TƯƠNG LAI của một phòng đang có khách vẫn kéo được, và cú kéo
  // truyền thẳng roomId đó vào đây. Phòng phải ở nguyên trong ô chọn: xoá đi
  // là biến một cú kéo hợp lệ thành một form trống không rõ lý do.
  it("giữ nguyên phòng kéo từ lịch dù phòng đó đang có khách", async () => {
    rooms = [
      ORIGINAL_ROOMS[0],
      { ...ORIGINAL_ROOMS[1], status: "occupied" },
    ];
    render(
      <ReservationSheet
        open
        onOpenChange={vi.fn()}
        preSelectedRoomId="R102"
        prefillDates={{ checkIn: "2026-08-12", checkOut: "2026-08-13" }}
      />,
    );

    const roomSelect = screen.getByLabelText(/^Phòng$/) as HTMLSelectElement;
    await waitFor(() => {
      expect(roomSelect).toHaveValue("R102");
    });

    // Nút gửi phải bật lên được: với phòng còn nguyên và tên khách đã điền,
    // không còn gì chặn chủ khách sạn đặt phòng cho 12–13/8.
    fireEvent.change(screen.getByPlaceholderText(/họ và tên/i), {
      target: { value: "Nguyễn Nhật Huy" },
    });
    expect(screen.getByRole("button", { name: /Đặt phòng/ })).toBeEnabled();
  });

  // Vế còn lại của cùng effect: một roomId KHÔNG phải phòng nào cả (phòng bị
  // xoá trong Cài đặt khi sheet đang mở) vẫn phải bị bỏ chọn — ô select hiện
  // trống trong khi state giữ một id truthy sẽ bật nút gửi, vì nút chỉ gác
  // trên `!roomId`.
  it("bỏ chọn roomId không còn tồn tại trong danh sách phòng", async () => {
    render(
      <ReservationSheet open onOpenChange={vi.fn()} preSelectedRoomId="R999" />,
    );

    await waitFor(() => {
      expect(screen.getByLabelText(/^Phòng$/)).toHaveValue("");
    });
    fireEvent.change(screen.getByPlaceholderText(/họ và tên/i), {
      target: { value: "Nguyễn Nhật Huy" },
    });
    expect(screen.getByRole("button", { name: /Đặt phòng/ })).toBeDisabled();
  });

  // Chế độ sửa: phòng lấy từ chính booking đang sửa, ô select bị disable, và
  // phòng đó rất có thể không còn vacant/booked — nó đang có khách. Ô select
  // hiện trống ở cả hai đời code (không có option khớp), nên chỉ soi DOM là
  // không phân biệt được; đường lưu thay đổi mới là thứ chứng minh state còn
  // nguyên: `handleSubmit` chặn ngay ở `!roomId`.
  it("không bỏ chọn phòng của booking đang sửa dù phòng đó đang có khách", async () => {
    rooms = [
      ORIGINAL_ROOMS[0],
      { ...ORIGINAL_ROOMS[1], status: "occupied" },
    ];
    const user = userEvent.setup();
    render(
      <ReservationSheet
        open
        onOpenChange={vi.fn()}
        editBooking={{
          id: "B-EDIT",
          room_id: "R102",
          guest_name: "Nguyen Van A",
          guest_phone: "0900000000",
          check_in_at: "2026-08-02T14:00:00+07:00",
          expected_checkout: "2026-08-05T12:00:00+07:00",
          scheduled_checkin: "2026-08-02",
          scheduled_checkout: "2026-08-05",
          nights: 3,
          guests: 2,
          total_price: 1000000,
          deposit_amount: 0,
          source: "phone",
        }}
      />,
    );

    await waitFor(() => {
      expect(fetchRooms).toHaveBeenCalledTimes(1);
    });

    await user.click(screen.getByRole("button", { name: /lưu thay đổi/i }));

    await waitFor(() => {
      expect(invokeWriteCommand).toHaveBeenCalledWith(
        "modify_reservation",
        expect.objectContaining({
          req: expect.objectContaining({ booking_id: "B-EDIT" }),
        }),
        expect.anything(),
      );
    });
    expect(toastError).not.toHaveBeenCalled();
  });
});
