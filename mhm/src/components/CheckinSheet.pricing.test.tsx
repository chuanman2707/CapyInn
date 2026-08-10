import type { ButtonHTMLAttributes, ReactNode } from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { clearMockResponses, invoke, setMockResponse } from "@test-mocks/tauri-core";
import { useHotelStore } from "@/stores/useHotelStore";
import type { Room } from "@/types";

vi.mock("@/components/ui/sheet", () => ({
  Sheet: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  SheetContent: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  SheetHeader: ({ children }: { children: ReactNode }) => <div>{children}</div>,
  SheetTitle: ({ children }: { children: ReactNode }) => <h2>{children}</h2>,
}));

vi.mock("@/components/ui/button", () => ({
  Button: ({ children, ...props }: ButtonHTMLAttributes<HTMLButtonElement>) => (
    <button {...props}>{children}</button>
  ),
}));

vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

import CheckinSheet from "./CheckinSheet";

const ROOM: Room = {
  id: "R101",
  name: "R101",
  // A real, multi-word type name — the shipped values have spaces in them.
  type: "Deluxe Balcony",
  floor: 1,
  has_balcony: true,
  // Round on purpose: `base_price × 1 night` is 500.000, which is not what the
  // backend quotes below.
  base_price: 500000,
  max_guests: 2,
  extra_person_fee: 0,
  status: "vacant",
};

/** What a configured rule plus a holiday surcharge actually produces. */
const BACKEND_TOTAL = 632_500;

function pricingResult(total: number, breakdown: { label: string; amount: number }[] = []) {
  return {
    pricing_type: "nightly",
    base_amount: total,
    surcharge_amount: 0,
    weekend_amount: 0,
    total,
    breakdown,
    capped: false,
  };
}

function previewArgs(): { roomId: string; checkIn: string; checkOut: string; guests: unknown }[] {
  return invoke.mock.calls
    .filter(([command]) => command === "calculate_room_price_preview")
    .map(([, args]) => args as never);
}

describe("CheckinSheet total price", () => {
  beforeEach(() => {
    clearMockResponses();
    invoke.mockClear();
    useHotelStore.setState({ isCheckinOpen: true, rooms: [] });
    setMockResponse("get_rooms", () => [ROOM]);
    setMockResponse("calculate_room_price_preview", () => pricingResult(BACKEND_TOTAL));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  /// The sheet rendered `base_price × nights` computed in JavaScript, which
  /// ignores the configured nightly rate, the weekend uplift and any holiday
  /// surcharge. Staff read that number aloud and took a deposit against it.
  it("shows the price the backend will charge, not base_price times nights", async () => {
    render(<CheckinSheet preSelectedRoomId="R101" />);

    await waitFor(() => {
      expect(screen.getByTestId("stay-price-total")).toHaveTextContent("632.500");
    });
    expect(screen.getByTestId("stay-price-total")).not.toHaveTextContent("500.000");
  });

  /// A walk-in is charged from `Local::now()` for `nights` days
  /// (`stay_lifecycle.rs`). A bare date, or a `Z` stamp, prices a different stay
  /// than the one about to be written.
  it("asks about the same instants the check-in will charge for", async () => {
    render(<CheckinSheet preSelectedRoomId="R101" />);

    await waitFor(() => expect(previewArgs().length).toBeGreaterThan(0));

    const { roomId, checkIn, checkOut } = previewArgs()[0];
    expect(roomId).toBe("R101");
    for (const stamp of [checkIn, checkOut]) {
      expect(stamp).toMatch(/[+-]\d{2}:\d{2}$/);
      expect(stamp).not.toContain("Z");
    }
    // `nights` defaults to 1, and the span between the two stamps IS the price.
    expect(Date.parse(checkOut) - Date.parse(checkIn)).toBe(86_400_000);
  });

  /// Ô số khách để trống thì không khai gì cả — `check_in` là chỗ duy nhất biết
  /// "trống nghĩa là một người". Báo giá cũng hỏi bằng đúng con số ấy, nên hai
  /// bên vẫn ra cùng một kết quả.
  it("asks with no guest count while the field is blank", async () => {
    render(<CheckinSheet preSelectedRoomId="R101" />);

    await waitFor(() => expect(previewArgs().length).toBeGreaterThan(0));
    expect(previewArgs()[0].guests).toBeNull();
  });

  /// Khai 4 người thì báo giá phải hỏi bằng 4. Trước đây chỗ này cứng `null`, và
  /// đó là lý do phụ thu thêm người chưa từng hiện trên báo giá lẫn hoá đơn.
  it("asks with the headcount the desk actually entered", async () => {
    const { container } = render(<CheckinSheet preSelectedRoomId="R101" />);
    await waitFor(() => expect(previewArgs().length).toBeGreaterThan(0));

    const field = Array.from(container.querySelectorAll("input[type=number]")).find(
      (input) => (input as HTMLInputElement).value === "",
    ) as HTMLInputElement;
    fireEvent.change(field, { target: { value: "4" } });

    await waitFor(() => {
      const latest = previewArgs()[previewArgs().length - 1];
      expect(latest.guests).toBe(4);
    });
  });

  /// The local day is what `special_dates` is looked up by. `toISOString()`
  /// reports the UTC day, which before 07:00 in Vietnam is still yesterday.
  it("quotes against the local calendar day, not the UTC one", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date(2026, 3, 20, 2, 0, 0));

    render(<CheckinSheet preSelectedRoomId="R101" />);

    await vi.waitFor(() => expect(previewArgs().length).toBeGreaterThan(0));
    expect(previewArgs()[0].checkIn.slice(0, 10)).toBe("2026-04-20");
  });

  /// A wrong total shown with confidence is worse than no total. The old code
  /// always had a number to show and no way to say "I do not know".
  it("says it could not price the stay rather than falling back to a guess", async () => {
    setMockResponse("calculate_room_price_preview", () => {
      throw new Error("database is locked");
    });

    render(<CheckinSheet preSelectedRoomId="R101" />);

    await waitFor(() => expect(screen.getByTestId("stay-price-error")).toBeInTheDocument());
    expect(screen.queryByTestId("stay-price-total")).not.toBeInTheDocument();
    expect(screen.queryByText(/500\.000/)).not.toBeInTheDocument();
  });

  /// The number came from the breakdown, which was fetched and thrown away.
  /// Without it a total that no longer equals base_price × nights cannot be
  /// explained at the desk.
  it("shows the breakdown behind a total that is not a simple multiplication", async () => {
    setMockResponse("calculate_room_price_preview", () =>
      pricingResult(BACKEND_TOTAL, [
        { label: "Tiền phòng", amount: 550_000 },
        { label: "Phụ thu ngày lễ", amount: 82_500 },
      ]),
    );

    render(<CheckinSheet preSelectedRoomId="R101" />);

    await waitFor(() => expect(screen.getByTestId("stay-price-breakdown")).toBeInTheDocument());
    const breakdown = screen.getByTestId("stay-price-breakdown");
    expect(breakdown).toHaveTextContent("Phụ thu ngày lễ");
    expect(breakdown).toHaveTextContent("82.500");
  });

  /// Price is keyed on the room type, so `rooms.base_price` is not a price the
  /// system charges. Printing it in the picker put a per-room figure beside a
  /// total that no longer derives from it.
  it("does not print a per-room base price beside the quoted total", async () => {
    render(<CheckinSheet preSelectedRoomId="R101" />);

    await waitFor(() => expect(screen.getByTestId("stay-price-total")).toBeInTheDocument());

    const option = screen.getByRole("option", { name: /R101/ });
    expect(option).not.toHaveTextContent("500.000");
  });
});

// FormField (shared) chỉ đặt <label> cạnh <input>, không có htmlFor/id liên
// kết chúng (cùng lý do đã ghi ở CheckinSheet.test.tsx / BackfillSheet.test.tsx)
// — nên getByLabelText không tìm ra các ô này.
function fieldInput(labelText: string): HTMLInputElement {
  const label = screen.getByText(labelText);
  const el = label.parentElement?.querySelector("input");
  if (!el) throw new Error(`Không tìm thấy input cạnh nhãn "${labelText}"`);
  return el as HTMLInputElement;
}

function fillRequiredGuestFields() {
  fireEvent.change(fieldInput("Họ và tên *"), { target: { value: "Nguyen Van A" } });
  fireEvent.change(fieldInput("Số điện thoại *"), { target: { value: "0900000000" } });
}

function checkInArgs(): { req: Record<string, unknown> }[] {
  return invoke.mock.calls
    .filter(([command]) => command === "check_in")
    .map(([, args]) => args as never);
}

describe("CheckinSheet rate override", () => {
  beforeEach(() => {
    clearMockResponses();
    invoke.mockClear();
    useHotelStore.setState({ isCheckinOpen: true, rooms: [] });
    setMockResponse("get_rooms", () => [ROOM]);
    // 632.500 không chia hết cho 3 đêm — nếu component lỡ gửi lại giá engine
    // (hoặc giá prefill làm tròn) thay vì đúng con số lễ tân gõ, test gửi giá
    // tay bắt được ngay vì hai con số cách xa nhau.
    setMockResponse("calculate_room_price_preview", () => pricingResult(BACKEND_TOTAL));
    setMockResponse("check_in", () => undefined);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("gửi rate_override_per_night khi lễ tân sửa giá", async () => {
    render(<CheckinSheet preSelectedRoomId="R101" preSelectedNights={3} />);
    fillRequiredGuestFields();

    fireEvent.click(await screen.findByTestId("rate-display"));
    fireEvent.change(screen.getByTestId("rate-input"), { target: { value: "400000" } });
    fireEvent.click(screen.getByRole("button", { name: /hoàn tất check-in/i }));

    await waitFor(() => expect(checkInArgs().length).toBeGreaterThan(0));
    expect(checkInArgs()[0].req.rate_override_per_night).toBe(400000);
  });

  it("gửi rate_override_per_night: null khi giữ giá hệ thống", async () => {
    render(<CheckinSheet preSelectedRoomId="R101" preSelectedNights={3} />);
    fillRequiredGuestFields();

    await screen.findByTestId("rate-display");
    fireEvent.click(screen.getByRole("button", { name: /hoàn tất check-in/i }));

    await waitFor(() => expect(checkInArgs().length).toBeGreaterThan(0));
    expect(checkInArgs()[0].req.rate_override_per_night).toBeNull();
  });

  // Mục 4 của brief: đổi phòng khi giá tay đang bật là một quyết định, không
  // phải chi tiết. Chọn: BỎ giá tay khi đổi phòng — giá thương lượng gắn với
  // phòng cụ thể đang chọn, mang nguyên số đó sang phòng vừa đổi tới là im
  // lặng thu sai giá phòng mới.
  it("đổi phòng thì bỏ giá tay đã gõ, quay lại giá engine", async () => {
    const ROOM_2 = { ...ROOM, id: "R102", name: "R102" };
    setMockResponse("get_rooms", () => [ROOM, ROOM_2]);

    render(<CheckinSheet preSelectedRoomId="R101" />);
    fillRequiredGuestFields();

    fireEvent.click(await screen.findByTestId("rate-display"));
    fireEvent.change(screen.getByTestId("rate-input"), { target: { value: "400000" } });
    expect(screen.getByTestId("rate-input")).toBeInTheDocument();

    const label = screen.getByText("Phòng *");
    const select = label.parentElement?.querySelector("select") as HTMLSelectElement;
    fireEvent.change(select, { target: { value: "R102" } });

    await waitFor(() => {
      expect(screen.queryByTestId("rate-input")).not.toBeInTheDocument();
    });
    expect(await screen.findByTestId("rate-display")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /hoàn tất check-in/i }));

    await waitFor(() => expect(checkInArgs().length).toBeGreaterThan(0));
    const last = checkInArgs()[checkInArgs().length - 1];
    expect(last.req.room_id).toBe("R102");
    expect(last.req.rate_override_per_night).toBeNull();
  });

  // Mục 2 của brief: bảng chi tiết từng đêm mô tả giá engine, đặt cạnh một
  // tổng đã bị đè là in hai con số mâu thuẫn — phải ẩn đi khi có giá tay.
  it("ẩn bảng chi tiết từng đêm khi lễ tân đang gõ giá tay", async () => {
    setMockResponse("calculate_room_price_preview", () =>
      pricingResult(BACKEND_TOTAL, [
        { label: "Tiền phòng", amount: 550_000 },
        { label: "Phụ thu ngày lễ", amount: 82_500 },
      ]),
    );

    render(<CheckinSheet preSelectedRoomId="R101" />);

    await waitFor(() => expect(screen.getByTestId("stay-price-breakdown")).toBeInTheDocument());

    fireEvent.click(screen.getByTestId("rate-display"));

    expect(screen.queryByTestId("stay-price-breakdown")).not.toBeInTheDocument();
  });

  // I-2 (review Task 17): 632.500/3 đêm rồi đổi thành 4 đêm, preview bắn lại
  // và lỗi. Trước bản vá, ô giá tay (kể cả nút "Về giá gốc") biến mất hoàn
  // toàn, thay bằng "Chưa tính được giá" — lễ tân không còn thấy con số đã
  // gõ, không sửa được, không huỷ được, nhưng Hoàn tất Check-in vẫn âm thầm
  // gửi đúng con số đó (4 đêm × 400.000 = 1.600.000₫, chưa từng hiện lên).
  it("giá tay vẫn hiện và sửa được khi preview lỗi ngay sau khi đang gõ (I-2)", async () => {
    let calls = 0;
    setMockResponse("calculate_room_price_preview", () => {
      calls += 1;
      if (calls === 1) return pricingResult(BACKEND_TOTAL);
      throw new Error("IPC lỗi giả lập");
    });

    render(<CheckinSheet preSelectedRoomId="R101" preSelectedNights={3} />);
    fillRequiredGuestFields();

    fireEvent.click(await screen.findByTestId("rate-display"));
    fireEvent.change(screen.getByTestId("rate-input"), { target: { value: "400000" } });

    // Đổi số đêm: preview bắn lại, lần này lỗi.
    fireEvent.change(fieldInput("Số đêm"), { target: { value: "4" } });
    await waitFor(() => expect(calls).toBeGreaterThanOrEqual(2));

    // Ô giá tay vẫn còn đó — không rơi về "Chưa tính được giá" trong khi lễ
    // tân đang gõ dở, và vẫn bấm "Về giá gốc" được.
    expect(screen.getByTestId("rate-input")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /về giá gốc/i })).toBeInTheDocument();
    expect(screen.queryByTestId("stay-price-error")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /hoàn tất check-in/i }));

    await waitFor(() => expect(checkInArgs().length).toBeGreaterThan(0));
    // Đúng số lễ tân đã gõ (đơn vị mỗi đêm) — không phải 0, không phải giá
    // engine, không phải giá trị bịa ra vì preview lỗi.
    expect(checkInArgs()[0].req.rate_override_per_night).toBe(400000);
  });

  // Nhánh còn lại: KHÔNG có giá tay nào đang gõ khi preview lỗi thì vẫn báo
  // lỗi trơn như trước — không có gì để prefill từ một engine đang lỗi.
  it("không hiện ô giá tay khi preview lỗi mà chưa từng gõ giá nào", async () => {
    setMockResponse("calculate_room_price_preview", () => {
      throw new Error("IPC lỗi giả lập");
    });

    render(<CheckinSheet preSelectedRoomId="R101" preSelectedNights={3} />);

    await waitFor(() => expect(screen.getByTestId("stay-price-error")).toBeInTheDocument());
    expect(screen.queryByTestId("rate-input")).not.toBeInTheDocument();
    expect(screen.queryByTestId("stay-price-total")).not.toBeInTheDocument();
  });
});

describe("CheckinSheet and the local day", () => {
  beforeEach(() => {
    clearMockResponses();
    invoke.mockClear();
    useHotelStore.setState({ isCheckinOpen: true, rooms: [] });
    setMockResponse("get_rooms", () => [ROOM]);
    setMockResponse("calculate_room_price_preview", () => pricingResult(BACKEND_TOTAL));
    setMockResponse("check_availability", () => ({ available: true, conflicts: [] }));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  function availabilityArgs(): { fromDate: string; toDate: string }[] {
    return invoke.mock.calls
      .filter(([command]) => command === "check_availability")
      .map(([, args]) => args as never);
  }

  /// At 02:00 in Vietnam the UTC day is still yesterday, so a `toISOString()`
  /// window asked whether the room was free *the day before* the guest is
  /// standing at the desk.
  it("checks availability for the local day, not the UTC day", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-04-21T02:00:00+07:00"));

    render(<CheckinSheet preSelectedRoomId="R101" />);

    await vi.waitFor(() => expect(availabilityArgs().length).toBeGreaterThan(0));
    expect(availabilityArgs()[0].fromDate).toBe("2026-04-21");
    expect(availabilityArgs()[0].toDate).toBe("2026-04-22");
  });

  /// A sheet opened before midnight and submitted after it quoted yesterday
  /// while `check_in` charged from today's `Local::now()` — different
  /// `special_dates` row, different weekend uplift. The night shift is a shift,
  /// not an edge case.
  it("re-quotes when the local day turns under an open sheet", async () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date("2026-04-21T23:59:00+07:00"));

    render(<CheckinSheet preSelectedRoomId="R101" />);

    await vi.waitFor(() => expect(previewArgs().length).toBeGreaterThan(0));
    expect(previewArgs()[0].checkIn).toContain("2026-04-21");

    const before = previewArgs().length;
    await vi.advanceTimersByTimeAsync(62_000);

    await vi.waitFor(() => expect(previewArgs().length).toBeGreaterThan(before));
    const latest = previewArgs()[previewArgs().length - 1];
    expect(latest.checkIn).toContain("2026-04-22");
    // Vẫn là mốc địa phương có độ lệch, không phải ngày trần hay `Z`.
    expect(latest.checkIn).toMatch(/[+-]\d{2}:\d{2}$/);
  });
});
