import type { ButtonHTMLAttributes, InputHTMLAttributes, ReactNode } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn());
const invokeWriteCommand = vi.hoisted(() => vi.fn());
const createCorrelationId = vi.hoisted(() => vi.fn());
const toastSuccess = vi.hoisted(() => vi.fn());
const fetchRooms = vi.hoisted(() => vi.fn());
const setRoomChangeOpen = vi.hoisted(() => vi.fn());

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
    rooms: [
      {
        id: "R101",
        name: "R101",
        type: "standard",
        status: "booked",
      },
    ],
    fetchRooms,
    setRoomChangeOpen,
  }),
}));

vi.mock("@/components/ui/input", () => ({
  Input: (props: InputHTMLAttributes<HTMLInputElement>) => <input {...props} />,
}));

vi.mock("@/components/ui/badge", () => ({
  Badge: ({ children }: { children: ReactNode }) => <span>{children}</span>,
}));

vi.mock("@/components/ui/button", () => ({
  Button: ({
    children,
    ...props
  }: ButtonHTMLAttributes<HTMLButtonElement>) => <button {...props}>{children}</button>,
}));

vi.mock("@/components/ReservationSheet", () => ({
  default: () => null,
}));

vi.mock("@/components/RoomDrawer", () => ({
  default: () => null,
}));

vi.mock("@/components/InvoiceDialog", () => ({
  default: ({ open }: { open: boolean }) => (open ? <div>invoice-dialog</div> : null),
}));

vi.mock("sonner", () => ({
  toast: {
    success: toastSuccess,
    error: vi.fn(),
  },
}));

import { emitTestEvent, resetEventMocks } from "@test-mocks/tauri-event";
import Reservations from "./Reservations";

function formatLocalDate(date: Date): string {
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function bookedReservation() {
  const today = new Date();
  const tomorrow = new Date(today);
  tomorrow.setDate(today.getDate() + 1);

  return {
    id: "B101",
    room_id: "R101",
    guest_name: "Nguyen Van A",
    guest_phone: "0900000000",
    check_in_at: formatLocalDate(today),
    expected_checkout: formatLocalDate(tomorrow),
    scheduled_checkin: formatLocalDate(today),
    scheduled_checkout: formatLocalDate(tomorrow),
    nights: 1,
    total_price: 500000,
    paid_amount: 50000,
    status: "booked",
    source: "phone",
    deposit_amount: 50000,
  };
}

function dateOffsetFromToday(days: number): string {
  const d = new Date();
  d.setDate(d.getDate() + days);
  return formatLocalDate(d);
}

function bookingAt(overrides: Record<string, unknown>) {
  return {
    ...bookedReservation(),
    ...overrides,
  };
}

async function openBookedReservationActions(user: ReturnType<typeof userEvent.setup>) {
  render(<Reservations />);

  await waitFor(() => {
    expect(screen.getAllByText("Nguyen Van A").length).toBeGreaterThan(0);
  });

  await user.click(screen.getAllByText("Nguyen Van A")[0]);
}

describe("Reservations", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_all_bookings") return [bookedReservation()];
      return undefined;
    });
    invokeWriteCommand.mockResolvedValue(undefined);
    createCorrelationId.mockReturnValue("COR-5E6F7A8B");
  });

  it("confirms reservations through invokeWriteCommand with correlation id", async () => {
    const user = userEvent.setup();
    await openBookedReservationActions(user);

    await user.click(screen.getByRole("button", { name: /check-in/i }));

    expect(createCorrelationId).toHaveBeenCalledTimes(1);
    await waitFor(() => {
      expect(invokeWriteCommand).toHaveBeenCalledWith(
        "confirm_reservation",
        { bookingId: "B101" },
        { correlationId: "COR-5E6F7A8B" },
      );
    });
    expect(toastSuccess).toHaveBeenCalledWith("Check-in reservation thành công!");
  });

  it("cancels reservations through invokeWriteCommand with correlation id", async () => {
    const user = userEvent.setup();
    await openBookedReservationActions(user);

    await user.click(screen.getByRole("button", { name: /hủy/i }));

    expect(createCorrelationId).toHaveBeenCalledTimes(1);
    await waitFor(() => {
      expect(invokeWriteCommand).toHaveBeenCalledWith(
        "cancel_reservation",
        { bookingId: "B101" },
        { correlationId: "COR-5E6F7A8B" },
      );
    });
    expect(toastSuccess).toHaveBeenCalledWith("Đã hủy reservation. Tiền cọc được giữ lại.");
  });

  it("closes the popup when editing a reservation", async () => {
    const user = userEvent.setup();
    await openBookedReservationActions(user);

    expect(screen.getByText(/Reservation — Nguyen Van A/)).toBeTruthy();

    await user.click(screen.getByRole("button", { name: /chỉnh sửa/i }));

    expect(screen.queryByText(/Reservation — Nguyen Van A/)).toBeNull();
  });

  it("opens the room change sheet for a booked reservation", async () => {
    const user = userEvent.setup();
    await openBookedReservationActions(user);

    await user.click(screen.getByRole("button", { name: /chuyển phòng/i }));

    expect(setRoomChangeOpen).toHaveBeenCalledWith(true, "B101");
  });
});

describe("Reservations timeline geometry", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invokeWriteCommand.mockResolvedValue(undefined);
    createCorrelationId.mockReturnValue("COR-5E6F7A8B");
  });

  function mockBookings(bookings: unknown[]) {
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_all_bookings") return bookings;
      return undefined;
    });
  }

  it("places a bar half a day into the check-in cell and half a day into the check-out cell", async () => {
    // Lưới bắt đầu ở hôm nay - 3. Khách nhận hôm nay - 1, trả hôm nay + 3.
    // rawStart = (-1 - -3) + 0.5 = 2.5 -> left  = 200px
    // rawEnd   = ( 3 - -3) + 0.5 = 6.5 -> width = (6.5 - 2.5) * 80 = 320px
    mockBookings([
      bookingAt({
        id: "B-GEO",
        scheduled_checkin: dateOffsetFromToday(-1),
        scheduled_checkout: dateOffsetFromToday(3),
      }),
    ]);

    render(<Reservations />);

    const bar = await screen.findByTestId("booking-bar-B-GEO");
    expect(bar.style.left).toBe("200px");
    expect(bar.style.width).toBe("320px");
  });

  it("keeps a visible bar for a booking that checks in and out on the same day", async () => {
    mockBookings([
      bookingAt({
        id: "B-SAMEDAY",
        scheduled_checkin: dateOffsetFromToday(0),
        scheduled_checkout: dateOffsetFromToday(0),
        nights: 0,
      }),
    ]);

    render(<Reservations />);

    const bar = await screen.findByTestId("booking-bar-B-SAMEDAY");
    expect(bar.style.width).toBe("40px");
  });

  it("does not render bookings that ended before the visible window", async () => {
    mockBookings([
      bookingAt({
        id: "B-PAST",
        scheduled_checkin: dateOffsetFromToday(-20),
        scheduled_checkout: dateOffsetFromToday(-10),
        status: "checked_out",
      }),
    ]);

    render(<Reservations />);

    await waitFor(() => {
      expect(screen.getByText(/Không tìm thấy|Chưa có booking|Room R101/)).toBeTruthy();
    });
    expect(screen.queryByTestId("booking-bar-B-PAST")).toBeNull();
  });

  it("does not render bookings that start after the visible window", async () => {
    mockBookings([
      bookingAt({
        id: "B-FUTURE",
        scheduled_checkin: dateOffsetFromToday(40),
        scheduled_checkout: dateOffsetFromToday(42),
      }),
    ]);

    render(<Reservations />);

    await waitFor(() => {
      expect(screen.getByText("Room R101")).toBeTruthy();
    });
    expect(screen.queryByTestId("booking-bar-B-FUTURE")).toBeNull();
  });

  it("clips a bar at the left edge and squares off the clipped corner", async () => {
    mockBookings([
      bookingAt({
        id: "B-CLIPPED",
        scheduled_checkin: dateOffsetFromToday(-10),
        scheduled_checkout: dateOffsetFromToday(2),
      }),
    ]);

    render(<Reservations />);

    const bar = await screen.findByTestId("booking-bar-B-CLIPPED");
    expect(bar.style.left).toBe("0px");
    // rawEnd = (2 - -3) + 0.5 = 5.5 -> width = 5.5 * 80 = 440px
    expect(bar.style.width).toBe("440px");
    expect(bar.querySelector(".rounded-l-none")).not.toBeNull();
  });

  // GIỚI HẠN: test này ghim CƠ CHẾ (tên lớp CSS), không ghim HÀNH VI. jsdom không
  // dựng layout và ResizeObserver của nó là mock không bao giờ bắn, nên không thể
  // kiểm một cú bấm thật rơi trúng đâu. Nó bắt được việc gỡ `inset-y-0`, gỡ
  // `flex items-center`, hay đổi chiều cao thanh; nó KHÔNG bắt được lỗi hit-test
  // thật trên trình duyệt, cũng không bắt được việc dời `relative` sang một thẻ
  // cha có chiều cao khác. Kiểm bằng tay trên ứng dụng thật vẫn là bắt buộc.
  it("stretches the booking bar hit area over the full row height", async () => {
    // Hàng cao 64px, thanh cao 42px. Nếu khung bọc chỉ cao bằng thanh thì còn
    // 11px hở trên và 11px hở dưới, và chuột rơi xuống ô ngày bên dưới — bấm
    // vào đó mở biểu mẫu đặt phòng cho một ngày đã có khách.
    mockBookings([
      bookingAt({
        id: "B-HITAREA",
        status: "active",
        scheduled_checkin: dateOffsetFromToday(-1),
        scheduled_checkout: dateOffsetFromToday(3),
      }),
    ]);

    render(<Reservations />);

    const bar = await screen.findByTestId("booking-bar-B-HITAREA");
    expect(bar.className).toContain("inset-y-0");
    expect(bar.className).not.toContain("top-1/2");
    // Khung bọc cao trọn hàng rồi thì việc căn giữa thanh 42px CHỈ còn dựa vào
    // `flex items-center`. Bỏ hai lớp này mà giữ `inset-y-0` thì vùng bấm vẫn
    // đúng nhưng thanh dán lên mép trên hàng — phải ghim cả hai.
    expect(bar.className).toContain("flex");
    expect(bar.className).toContain("items-center");
    expect(bar.querySelector(".h-\\[42px\\]")).not.toBeNull();
  });
});

describe("Reservations date navigation", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_all_bookings") return [];
      return undefined;
    });
    invokeWriteCommand.mockResolvedValue(undefined);
    createCorrelationId.mockReturnValue("COR-5E6F7A8B");
  });

  it("shows the month of the visible range and updates it when paging", async () => {
    const user = userEvent.setup();
    render(<Reservations />);

    const label = await screen.findByTestId("timeline-range-label");
    const initial = label.textContent ?? "";
    expect(initial).toMatch(/THÁNG/);

    // Nhảy 8 tuần về phía trước chắc chắn ra khỏi tháng hiện tại.
    const next = screen.getByRole("button", { name: /tuần sau/i });
    for (let i = 0; i < 8; i += 1) {
      await user.click(next);
    }

    expect(screen.getByTestId("timeline-range-label").textContent).not.toBe(initial);
  });

  it("hides the today button until the range moves, then resets the range", async () => {
    const user = userEvent.setup();
    render(<Reservations />);

    await screen.findByTestId("timeline-range-label");
    const initialLabel = screen.getByTestId("timeline-range-label").textContent;
    expect(screen.queryByRole("button", { name: /hôm nay/i })).toBeNull();

    await user.click(screen.getByRole("button", { name: /tuần sau/i }));
    const todayButton = screen.getByRole("button", { name: /hôm nay/i });
    expect(todayButton).toBeTruthy();

    await user.click(todayButton);

    expect(screen.queryByRole("button", { name: /hôm nay/i })).toBeNull();
    expect(screen.getByTestId("timeline-range-label").textContent).toBe(initialLabel);
  });

  it("keeps the today marker when today is still inside the range after paging back", async () => {
    const user = userEvent.setup();
    render(<Reservations />);

    await screen.findByTestId("timeline-range-label");
    await user.click(screen.getByRole("button", { name: /tuần trước/i }));

    // Lùi 7 ngày: lưới phủ hôm nay - 10 đến hôm nay + 5, hôm nay vẫn nằm trong.
    expect(screen.getByTestId("timeline-today-marker")).toBeTruthy();
  });
});

describe("Reservations timeline refresh", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetEventMocks();
    invokeWriteCommand.mockResolvedValue(undefined);
    createCorrelationId.mockReturnValue("COR-5E6F7A8B");
  });

  // Đường check-in kéo từ lịch bàn giao hẳn cho CheckinSheet ở MainShell —
  // không có onOpenChange nào quay về trang này, nên trước đây check-in xong
  // là lịch đứng im và ô vừa kéo trông như vẫn trống. Sự kiện "db-updated" là
  // thứ backend phát sau MỌI lệnh ghi, kể cả check_in.
  it("nạp lại bookings khi backend báo dữ liệu đổi", async () => {
    let bookings: unknown[] = [];
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_all_bookings") return bookings;
      return undefined;
    });

    render(<Reservations />);

    await waitFor(() => {
      expect(screen.getByText("Room R101")).toBeTruthy();
    });
    expect(screen.queryByTestId("booking-bar-B-CHECKIN")).toBeNull();

    // Khách vừa được check-in từ ô lịch: booking chỉ tồn tại sau lần nạp sau.
    bookings = [
      bookingAt({
        id: "B-CHECKIN",
        guest_name: "Khách vãng lai",
        status: "active",
        scheduled_checkin: dateOffsetFromToday(0),
        scheduled_checkout: dateOffsetFromToday(2),
      }),
    ];

    await emitTestEvent("db-updated", { entity: "rooms" });

    expect(await screen.findByTestId("booking-bar-B-CHECKIN")).toBeTruthy();
  });
});

describe("Reservations checked-out bookings", () => {
  function checkedOutBooking() {
    return {
      ...bookedReservation(),
      id: "B-OUT",
      guest_name: "Hoseo Kim",
      status: "checked_out",
      actual_checkout: "2026-07-25T09:12:00+07:00",
      paid_amount: 500000,
      scheduled_checkin: dateOffsetFromToday(-2),
      scheduled_checkout: dateOffsetFromToday(-1),
    };
  }

  beforeEach(() => {
    vi.clearAllMocks();
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_all_bookings") return [checkedOutBooking()];
      if (command === "get_invoice") return { id: "invoice-1" };
      return undefined;
    });
    invokeWriteCommand.mockResolvedValue(undefined);
    createCorrelationId.mockReturnValue("COR-5E6F7A8B");
  });

  it("opens a read-only popup when a checked-out bar is clicked", async () => {
    const user = userEvent.setup();
    render(<Reservations />);

    await user.click(await screen.findByTestId("booking-bar-B-OUT"));

    expect(screen.getByText(/Đã trả — Hoseo Kim/)).toBeTruthy();
    expect(screen.getByText("Trả phòng lúc")).toBeTruthy();
    expect(screen.queryByRole("button", { name: /^hủy$/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /chuyển phòng/i })).toBeNull();
  });

  it("reads the existing invoice instead of generating a new one", async () => {
    const user = userEvent.setup();
    render(<Reservations />);

    await user.click(await screen.findByTestId("booking-bar-B-OUT"));
    await user.click(screen.getByRole("button", { name: /xem hóa đơn/i }));

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledWith("get_invoice", { bookingId: "B-OUT" });
    });
    expect(invokeWriteCommand).not.toHaveBeenCalledWith(
      "generate_invoice",
      expect.anything(),
    );
    expect(await screen.findByText("invoice-dialog")).toBeTruthy();
  });
});

// Ca thật, ngày 31/07/2026: cột `bookings.guests` thiếu trên máy chủ khách sạn
// (migration v22 bị bỏ qua vì schema_version đã là 23), nên `get_all_bookings`
// đổ lỗi "no such column: b.guests". `.catch(() => setBookings([]))` nuốt trọn
// nó và trang hiện "Chưa có booking nào" — chủ đọc ra là mất sạch dữ liệu, đi
// tìm bản backup, trong khi 25 booking vẫn nằm nguyên trong database.
// C1 (rà cuối trước merge): backend đã bịt 8 đường đọc SQL còn sót lượt đã
// xoá, nhưng frontend lọc bar bằng danh sách ĐEN (`status !== "cancelled"`) —
// đúng lớp lỗi "quên rà nhánh status" đã cắn ba lần trên nhánh này. Một lượt
// "voided" lọt qua tầng SQL (index cũ, cache, hay chính bug đang sửa) vẫn phải
// biến mất khỏi lịch ở tầng này — phòng thủ theo chiều sâu.
describe("Reservations voided bookings", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetEventMocks();
    invokeWriteCommand.mockResolvedValue(undefined);
    createCorrelationId.mockReturnValue("COR-5E6F7A8B");
  });

  it("không vẽ thanh cho lượt đã xóa (voided), dù nó lọt qua tới tận đây", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_all_bookings") {
        return [bookingAt({ id: "B-VOIDED", status: "voided" })];
      }
      return undefined;
    });

    render(<Reservations />);

    await waitFor(() => {
      expect(screen.getByText("Room R101")).toBeTruthy();
    });
    expect(screen.queryByTestId("booking-bar-B-VOIDED")).toBeNull();
  });

  it("không đếm lượt đã xóa vào tổng số booking hiển thị", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_all_bookings") {
        return [
          bookingAt({ id: "B-VOIDED", status: "voided" }),
          bookingAt({ id: "B-OK", status: "booked" }),
        ];
      }
      return undefined;
    });

    render(<Reservations />);

    await waitFor(() => {
      expect(screen.getByTestId("booking-bar-B-OK")).toBeTruthy();
    });
    expect(screen.getByTestId("total-booking-count").textContent).toBe("1");
  });

  // M6 (rà cuối trước merge): `totalCount` lọc bằng danh sách ĐEN
  // (`status !== "voided"`), trong khi bar trên lịch lọc bằng danh sách
  // TRẮNG `VISIBLE_BOOKING_STATUSES` (không có "cancelled") — lệch nhau
  // trong cùng một commit. Lượt đã hủy không có bar nào trên lịch (không
  // nằm trong VISIBLE_BOOKING_STATUSES) nên không được góp vào "Tổng" —
  // "Tổng" ở đây mô tả các lượt còn đang hiện diện trên lịch, không phải
  // toàn bộ lịch sử booking đã từng tạo ra.
  it("không đếm lượt đã hủy (cancelled) vào tổng số booking hiển thị, giống hệt lượt đã xóa", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_all_bookings") {
        return [
          bookingAt({ id: "B-CANCELLED", status: "cancelled" }),
          bookingAt({ id: "B-OK", status: "booked" }),
        ];
      }
      return undefined;
    });

    render(<Reservations />);

    await waitFor(() => {
      expect(screen.getByTestId("booking-bar-B-OK")).toBeTruthy();
    });
    expect(screen.getByTestId("total-booking-count").textContent).toBe("1");
  });
});

describe("Reservations load errors", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    resetEventMocks();
    invokeWriteCommand.mockResolvedValue(undefined);
    createCorrelationId.mockReturnValue("COR-5E6F7A8B");
  });

  function failBookingsWith(message: string) {
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_all_bookings") throw new Error(message);
      return undefined;
    });
  }

  it("hiện lỗi thật thay vì giả vờ là chưa có booking nào", async () => {
    failBookingsWith("in prepare, no such column: b.guests");

    render(<Reservations />);

    expect(await screen.findByText(/no such column: b\.guests/i)).toBeTruthy();
    // Câu này là thứ đã đánh lừa chủ khách sạn. Nó chỉ được phép xuất hiện khi
    // đọc THÀNH CÔNG và kết quả rỗng thật.
    expect(screen.queryByText(/Chưa có booking nào/i)).toBeNull();
  });

  it("giữ nguyên booking đang hiện khi một lần nạp lại thất bại", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_all_bookings") return [bookedReservation()];
      return undefined;
    });

    render(<Reservations />);
    expect(await screen.findByTestId("booking-bar-B101")).toBeTruthy();

    // Lần nạp lại sau một lệnh ghi bị hỏng. Xoá sạch lịch vì một sự cố tạm
    // thời là tự tay biến nó thành cảnh mất dữ liệu.
    failBookingsWith("database is locked");
    await emitTestEvent("db-updated", { entity: "bookings" });

    expect(await screen.findByText(/database is locked/i)).toBeTruthy();
    expect(screen.getByTestId("booking-bar-B101")).toBeTruthy();
  });

  it("cho thử lại, và gỡ thông báo lỗi khi đọc lại được", async () => {
    const user = userEvent.setup();
    failBookingsWith("database is locked");

    render(<Reservations />);
    expect(await screen.findByText(/database is locked/i)).toBeTruthy();

    invoke.mockImplementation(async (command: string) => {
      if (command === "get_all_bookings") return [bookedReservation()];
      return undefined;
    });
    await user.click(screen.getByRole("button", { name: /thử lại/i }));

    expect(await screen.findByTestId("booking-bar-B101")).toBeTruthy();
    expect(screen.queryByText(/database is locked/i)).toBeNull();
  });
});

describe("Reservations column width", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    createCorrelationId.mockReturnValue("COR-5E6F7A8B");
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  // GIỚI HẠN: test này ghim PHÉP TÍNH, không ghim ĐÍCH ĐO. Stub bề rộng gắn
  // trên `HTMLElement.prototype` nên MỌI phần tử — kể cả một phần tử sai —
  // đều trả lời cùng một con số; test không có cách nào phân biệt được
  // `timelineRef` đang đo đúng khung lịch hay lỡ đo trúng một phần tử khác.
  // Dời `ref={timelineRef}` từ khung lịch sang ô nhãn "Rooms" 140px vẫn qua
  // được mọi test ở đây — trong trình duyệt thật phép đo đó ra
  // `floor((140 - 140) / 16) = 0`, rơi về 80px, cả tính năng coi như chết —
  // mà không một dòng nào đỏ. Chỉ một lần đổi cỡ cửa sổ bằng tay trên ứng
  // dụng thật mới bắt được lỗi này.
  it("stretches day columns to fill the measured timeline width", async () => {
    // 1780 - 140 (cột tên phòng) = 1640, chia 16 ngày = 102.5 -> 102 sau khi
    // làm tròn xuống. Phần dư 8px chấp nhận được; làm tròn lên sẽ tràn ra
    // ngoài và đẻ ra thanh cuộn ngang không cần thiết.
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
      width: 1780,
      height: 600,
      top: 0,
      left: 0,
      right: 1780,
      bottom: 600,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    } as DOMRect);

    invoke.mockImplementation(async (command: string) => {
      if (command === "get_all_bookings") {
        return [
          bookingAt({
            id: "B-WIDE",
            scheduled_checkin: dateOffsetFromToday(-1),
            scheduled_checkout: dateOffsetFromToday(3),
          }),
        ];
      }
      return undefined;
    });

    render(<Reservations />);

    // Ghim nửa còn lại của cùng một bất biến: cột ngày phải rộng đúng bằng
    // colWidth đo được, không chỉ thanh booking vẽ trên nó. Bar và cell tính
    // từ cùng một colWidth nhưng qua hai đường code khác nhau (ba vòng lặp
    // DAYS.map dựng cell, getBookingBars dựng bar) — chỉ ghim bar thì một
    // cell bị revert về hằng số cũ (w-[80px]) vẫn lọt qua test, trong khi bar
    // vẫn vẽ ở 102px/ngày trên cột 80px thật: đúng nghĩa "booking hiện sai
    // ngày".
    expect(screen.getByTestId("cell-R101-0").style.width).toBe("102px");

    const bar = await screen.findByTestId("booking-bar-B-WIDE");
    // rawStart = (-1 - -3) + 0.5 = 2.5 -> left = 2.5 * 102 = 255px
    expect(bar.style.left).toBe("255px");
    // rawEnd = (3 - -3) + 0.5 = 6.5 -> width = 4 * 102 = 408px
    expect(bar.style.width).toBe("408px");
  });

  it("falls back to the 80px minimum when the timeline has no measurable width", async () => {
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_all_bookings") {
        return [
          bookingAt({
            id: "B-NARROW",
            scheduled_checkin: dateOffsetFromToday(-1),
            scheduled_checkout: dateOffsetFromToday(3),
          }),
        ];
      }
      return undefined;
    });

    render(<Reservations />);

    const bar = await screen.findByTestId("booking-bar-B-NARROW");
    expect(bar.style.left).toBe("200px");
    expect(bar.style.width).toBe("320px");
  });
});
