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
            new_guests: 2,
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
          breakdown: [{ label: "3 night(s) x 500.000", amount: 1_500_000 }],
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
      expect(screen.getByText(/3 night\(s\)/i)).toBeInTheDocument();
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
            { label: "2 night(s) x 500.000", amount: 1_000_000 },
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
            { label: "2 night(s) x 1.000.000", amount: 2_000_000 },
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
              { label: "1 night(s) x 500.000", amount: 500_000 },
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
});
