import type {
  ButtonHTMLAttributes,
  InputHTMLAttributes,
  LabelHTMLAttributes,
  ReactNode,
} from "react";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { clearMockResponses, invoke, setMockResponse } from "@test-mocks/tauri-core";

const autoAssignRooms = vi.hoisted(() => vi.fn());
const groupCheckIn = vi.hoisted(() => vi.fn());
const setGroupCheckinOpen = vi.hoisted(() => vi.fn());
/** Mutable so a test can close the sheet and observe the form reset. */
const sheetState = vi.hoisted(() => ({ open: true }));

// Two rooms, two prices, and a multi-word type name. Deliberately not tidier
// than production data: the shipped type names contain spaces, and a fixture
// without one has hidden a bug in this exact code path before.
const ROOMS = [
  {
    id: "R101",
    name: "R101",
    type: "Standard Room",
    room_type: "Standard Room",
    floor: 1,
    has_balcony: false,
    base_price: 500000,
    max_guests: 2,
    extra_person_fee: 0,
    status: "vacant",
  },
  {
    id: "R202",
    name: "R202",
    type: "Deluxe Balcony",
    room_type: "Deluxe Balcony",
    floor: 2,
    has_balcony: true,
    base_price: 800000,
    max_guests: 2,
    extra_person_fee: 0,
    status: "vacant",
  },
];

vi.mock("@/stores/useHotelStore", () => ({
  useHotelStore: () => ({
    isGroupCheckinOpen: sheetState.open,
    setGroupCheckinOpen,
    rooms: ROOMS,
    groupCheckIn,
    autoAssignRooms,
    loading: false,
  }),
}));

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

vi.mock("@/components/ui/input", () => ({
  Input: (props: InputHTMLAttributes<HTMLInputElement>) => <input {...props} />,
}));

vi.mock("@/components/ui/label", () => ({
  Label: ({ children, ...props }: LabelHTMLAttributes<HTMLLabelElement>) => (
    <label {...props}>{children}</label>
  ),
}));

vi.mock("sonner", () => ({ toast: { error: vi.fn(), success: vi.fn() } }));

import GroupCheckinSheet from "./GroupCheckinSheet";

/** Two rooms, quoted individually — not 500.000 + 800.000. */
const QUOTES: Record<string, number> = { R101: 632_500, R202: 1_012_000 };

function pricingResult(total: number) {
  return {
    pricing_type: "nightly",
    base_amount: total,
    surcharge_amount: 0,
    weekend_amount: 0,
    total,
    breakdown: [],
    capped: false,
  };
}

function previewArgs(): { roomId: string; checkIn: string; checkOut: string; guests: unknown }[] {
  return invoke.mock.calls
    .filter(([command]) => command === "calculate_room_price_preview")
    .map(([, args]) => args as never);
}

/** Clicks forward until the summary, which is where the group total is shown. */
async function advanceToSummary(user: ReturnType<typeof userEvent.setup>) {
  for (let step = 0; step < 3; step += 1) {
    if (screen.queryByTestId("group-price-total") || screen.queryByTestId("group-price-error")) {
      return;
    }
    const next = screen.queryByRole("button", { name: /Tiếp theo/i });
    if (!next) return;
    await user.click(next);
  }
}

/** Walks the wizard as far as picking both rooms, which is what starts quoting. */
async function pickBothRooms() {
  const user = userEvent.setup();
  render(<GroupCheckinSheet />);

  const textboxes = screen.getAllByRole("textbox");
  await user.type(textboxes[0], "Đoàn Hà Nội");
  await user.type(textboxes[1], "Trần Văn B");
  await user.click(screen.getByRole("button", { name: /Tiếp theo/i }));
  await user.click(screen.getByRole("button", { name: /Chọn tay/i }));
  await user.click(screen.getByRole("button", { name: /R101/ }));
  await user.click(screen.getByRole("button", { name: /R202/ }));

  // The wizard will not advance without a master room, and the master picker
  // renders below the grid with the same room names — take the later one.
  const masterCandidates = screen.getAllByRole("button", { name: /R101/ });
  await user.click(masterCandidates[masterCandidates.length - 1]);

  return user;
}

describe("GroupCheckinSheet total price", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    clearMockResponses();
    invoke.mockClear();
    groupCheckIn.mockResolvedValue(undefined);
    autoAssignRooms.mockResolvedValue({ assignments: [] });
    setMockResponse("calculate_room_price_preview", (args) =>
      pricingResult(QUOTES[(args as { roomId: string }).roomId] ?? 0),
    );
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  /// The sheet summed `base_price × nights` per room in JavaScript, which
  /// ignores the configured rate, the weekend uplift and any holiday surcharge —
  /// once per room, so the group total was wrong by a multiple.
  it("quotes each room through the engine instead of multiplying", async () => {
    const user = await pickBothRooms();

    await waitFor(() =>
      expect(Array.from(new Set(previewArgs().map((a) => a.roomId))).sort()).toEqual([
        "R101",
        "R202",
      ]),
    );

    await advanceToSummary(user);

    // 632.500 + 1.012.000, the two engine quotes — not 500.000 + 800.000, which
    // is what multiplying the base prices by one night would have shown.
    const total = await screen.findByTestId("group-price-total");
    expect(total).toHaveTextContent("1.644.500");
    expect(total).not.toHaveTextContent("1.300.000");
  });

  /// `group_lifecycle.rs` charges with `None`, so the group is never billed the
  /// extra-person fee. Quoting a guest count would show more than it collects.
  it("asks with no guest count, exactly as the group charge does", async () => {
    await pickBothRooms();

    await waitFor(() => expect(previewArgs().length).toBeGreaterThan(0));
    for (const args of previewArgs()) {
      expect(args.guests).toBeNull();
    }
  });

  /// A walk-in group is priced from `Local::now()`; a reservation from bare
  /// dates. Asking with the wrong pair prices a different stay than the one
  /// being booked.
  it("asks about the instants a walk-in group will be charged for", async () => {
    await pickBothRooms();

    await waitFor(() => expect(previewArgs().length).toBeGreaterThan(0));

    const { checkIn, checkOut } = previewArgs()[0];
    expect(checkIn).toMatch(/[+-]\d{2}:\d{2}$/);
    expect(checkIn).not.toContain("Z");
    expect(Date.parse(checkOut) - Date.parse(checkIn)).toBe(86_400_000);
  });

  /// `toISOString()` reports the UTC day, which before 07:00 in Vietnam is
  /// yesterday. `isReservation` compares the arrival against it, so at 02:00 a
  /// walk-in group read as a *reservation* and was quoted from bare dates —
  /// while `group_lifecycle.rs`, comparing against its own local today, would
  /// charge it down the walk-in branch. Two branches, one stay.
  ///
  /// Asserting the day alone does not catch this: both branches say
  /// "2026-04-20" here. The offset is what distinguishes them.
  it("prices a 02:00 group as the walk-in the backend will charge it as", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date(2026, 3, 20, 2, 0, 0));

    await pickBothRooms();

    await vi.waitFor(() => expect(previewArgs().length).toBeGreaterThan(0));
    const { checkIn } = previewArgs()[0];
    expect(checkIn.slice(0, 10)).toBe("2026-04-20");
    expect(checkIn).toMatch(/T\d{2}:\d{2}:\d{2}[+-]\d{2}:\d{2}$/);
  });

  /// A total summed from one of two rooms is the same class of lie the
  /// multiplication was. There has to be a way to say "I do not know".
  it("says it could not price the group rather than showing a partial total", async () => {
    setMockResponse("calculate_room_price_preview", (args) => {
      if ((args as { roomId: string }).roomId === "R202") {
        throw new Error("database is locked");
      }
      return pricingResult(632_500);
    });

    const user = await pickBothRooms();
    await advanceToSummary(user);

    await screen.findByTestId("group-price-error");
    expect(screen.queryByTestId("group-price-total")).not.toBeInTheDocument();
    // Neither the multiplication nor a total summed from the one room that did
    // price successfully.
    expect(screen.queryByText(/1\.300\.000/)).not.toBeInTheDocument();
    expect(screen.queryByText(/632\.500/)).not.toBeInTheDocument();
  });
});

describe("GroupCheckinSheet and the local day turning", () => {
  beforeEach(() => {
    clearMockResponses();
    invoke.mockClear();
    autoAssignRooms.mockReset();
    groupCheckIn.mockReset();
    setMockResponse("calculate_room_price_preview", () => pricingResult(632_500));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  /// `todayStr` used to be captured once per render with no reason to re-render,
  /// so an arrival date of "tomorrow" stayed classified as a reservation after
  /// midnight had made it *today*. The two classifications send different date
  /// shapes: bare `YYYY-MM-DD` for a reservation, an offset stamp for a walk-in.
  /// `group_lifecycle.rs` branches the same way against its own local today, so
  /// the desk and the ledger disagreed about which stay was being priced.
  it("reclassifies tomorrow's group as a walk-in once midnight makes it today", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date(2026, 3, 20, 23, 59, 0));

    const user = userEvent.setup();
    render(<GroupCheckinSheet />);

    // Ngày nhận phòng nằm ở bước 0, nên phải đặt trước khi đi tiếp. Ngày mai lúc
    // này là 21, nên đây là đặt trước: ngày trần, không có độ lệch.
    const arrival = screen.getByDisplayValue("2026-04-20") as HTMLInputElement;
    await user.clear(arrival);
    await user.type(arrival, "2026-04-21");

    const textboxes = screen.getAllByRole("textbox");
    await user.type(textboxes[0], "Đoàn Hà Nội");
    await user.type(textboxes[1], "Trần Văn B");
    await user.click(screen.getByRole("button", { name: /Tiếp theo/i }));
    await user.click(screen.getByRole("button", { name: /Chọn tay/i }));
    await user.click(screen.getByRole("button", { name: /R101/ }));
    await user.click(screen.getByRole("button", { name: /R202/ }));
    const masterCandidates = screen.getAllByRole("button", { name: /R101/ });
    await user.click(masterCandidates[masterCandidates.length - 1]);

    await vi.waitFor(() =>
      expect(previewArgs().some((a) => a.checkIn === "2026-04-21")).toBe(true),
    );

    const beforeMidnight = previewArgs().length;
    await vi.advanceTimersByTimeAsync(62_000);

    // Qua nửa đêm, 21 chính là hôm nay, nên phải chuyển sang nhánh vãng lai —
    // đúng nhánh backend sẽ tính tiền.
    await vi.waitFor(() => expect(previewArgs().length).toBeGreaterThan(beforeMidnight));
    const latest = previewArgs()[previewArgs().length - 1];
    expect(latest.checkIn).toMatch(/T\d{2}:\d{2}:\d{2}[+-]\d{2}:\d{2}$/);
    expect(latest.checkIn.slice(0, 10)).toBe("2026-04-21");
  });
});

describe("GroupCheckinSheet form reset", () => {
  beforeEach(() => {
    clearMockResponses();
    invoke.mockClear();
    sheetState.open = true;
    setMockResponse("calculate_room_price_preview", () => pricingResult(632_500));
  });

  afterEach(() => {
    sheetState.open = true;
    vi.useRealTimers();
  });

  /// The reset ran `new Date().toISOString().split("T")[0]` — the UTC day —
  /// three lines below a comment explaining why that is wrong. Before 07:00 in
  /// Vietnam the UTC day is still yesterday, so closing and reopening the sheet
  /// on the night shift put *yesterday* in the arrival field, which then reads as
  /// a backfill rather than today's walk-in.
  it("resets the arrival date to the local day, not the UTC day", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    vi.setSystemTime(new Date(2026, 3, 21, 2, 0, 0));

    const user = userEvent.setup();
    const { container, rerender } = render(<GroupCheckinSheet />);
    const dateInput = () => container.querySelector('input[type="date"]') as HTMLInputElement;

    const arrival = screen.getByDisplayValue("2026-04-21") as HTMLInputElement;
    await user.clear(arrival);
    await user.type(arrival, "2026-04-25");
    expect((screen.getByDisplayValue("2026-04-25") as HTMLInputElement).value).toBe("2026-04-25");

    sheetState.open = false;
    rerender(<GroupCheckinSheet />);
    sheetState.open = true;
    rerender(<GroupCheckinSheet />);

    await vi.waitFor(() => expect(dateInput().value).toBe("2026-04-21"));
    // 2026-04-20 là ngày UTC lúc 02:00 giờ Việt Nam — chính con số mà bản cũ đặt.
    expect(dateInput().value).not.toBe("2026-04-20");
  });
});

describe("GroupCheckinSheet manual rate override", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    clearMockResponses();
    invoke.mockClear();
    groupCheckIn.mockResolvedValue(undefined);
    autoAssignRooms.mockResolvedValue({ assignments: [] });
    sheetState.open = true;
    setMockResponse("calculate_room_price_preview", (args) =>
      pricingResult(QUOTES[(args as { roomId: string }).roomId] ?? 0),
    );
  });

  afterEach(() => {
    sheetState.open = true;
    vi.useRealTimers();
  });

  /// Task 15 báo giá KHÔNG chia hết cho số đêm — chọn nights=2 nhưng
  /// `QUOTES` không đổi theo số đêm — để một implementer quên nhân `nights`
  /// vào giá tay (chỉ cộng override một lần) không lọt qua được test.
  it("gửi rate_override_per_room theo từng phòng và cộng đúng vào tổng đoàn", async () => {
    const user = userEvent.setup();
    const { container } = render(<GroupCheckinSheet />);

    const textboxes = screen.getAllByRole("textbox");
    await user.type(textboxes[0], "Đoàn Hà Nội");
    await user.type(textboxes[1], "Trần Văn B");

    const nightsInput = container.querySelectorAll('input[type="number"]')[1] as HTMLInputElement;
    fireEvent.change(nightsInput, { target: { value: "2" } });

    await user.click(screen.getByRole("button", { name: /Tiếp theo/i }));
    await user.click(screen.getByRole("button", { name: /Chọn tay/i }));
    await user.click(screen.getByRole("button", { name: /R101/ }));
    await user.click(screen.getByRole("button", { name: /R202/ }));
    const masterCandidates = screen.getAllByRole("button", { name: /R101/ });
    await user.click(masterCandidates[masterCandidates.length - 1]);

    await advanceToSummary(user);
    await screen.findByTestId("group-price-total");

    fireEvent.click((await screen.findAllByTestId("rate-display"))[0]);
    fireEvent.change(screen.getAllByTestId("rate-input")[0], {
      target: { value: "400000" },
    });

    fireEvent.click(screen.getByRole("button", { name: /Hoàn tất Group Check-in/i }));

    await waitFor(() => expect(groupCheckIn).toHaveBeenCalledTimes(1));
    const req = groupCheckIn.mock.calls[0][0];
    expect(req.rate_override_per_room).toEqual({ R101: 400000 });

    // 400.000₫/đêm × 2 đêm = 800.000 cho R101, cộng giá engine 1.012.000 của
    // R202 (không đổi theo nights vì QUOTES cố định) = 1.812.000.
    expect(screen.getByTestId("group-price-total")).toHaveTextContent("1.812.000");
  });

  it("không sửa giá phòng nào thì gửi map rỗng, không phải undefined", async () => {
    const user = await pickBothRooms();
    await advanceToSummary(user);
    await screen.findByTestId("group-price-total");

    fireEvent.click(screen.getByRole("button", { name: /Hoàn tất Group Check-in/i }));

    await waitFor(() => expect(groupCheckIn).toHaveBeenCalledTimes(1));
    const req = groupCheckIn.mock.calls[0][0];
    expect(req.rate_override_per_room).toEqual({});
  });

  /// `group_checkin_tx` từ chối cả giao dịch nếu map override còn khoá không
  /// nằm trong `room_ids` — bỏ chọn một phòng đã sửa giá PHẢI dọn khoá đó
  /// ngay, không đợi tới lúc submit.
  it("bỏ chọn một phòng đã sửa giá thì khoá đó biến mất khỏi map", async () => {
    const user = await pickBothRooms();
    await advanceToSummary(user);
    await screen.findByTestId("group-price-total");

    // Sửa giá phòng R202 (không phải master, không phải phòng đầu tiên).
    const displays = await screen.findAllByTestId("rate-display");
    fireEvent.click(displays[1]);
    fireEvent.change(screen.getAllByTestId("rate-input")[0], {
      target: { value: "300000" },
    });

    // Quay lại bước chọn phòng và bỏ chọn R202. Có hai nút tên "R202" ở bước
    // này (ô lưới chọn phòng và ô "Phòng đại diện") — lấy nút lưới, đứng
    // trước trong DOM.
    await user.click(screen.getByRole("button", { name: /Quay lại/i }));
    await user.click(screen.getByRole("button", { name: /Quay lại/i }));
    await user.click(screen.getAllByRole("button", { name: /R202/ })[0]);

    await advanceToSummary(user);
    fireEvent.click(screen.getByRole("button", { name: /Hoàn tất Group Check-in/i }));

    await waitFor(() => expect(groupCheckIn).toHaveBeenCalledTimes(1));
    const req = groupCheckIn.mock.calls[0][0];
    expect(req.rate_override_per_room).toEqual({});
    expect(req.room_ids).toEqual(["R101"]);
  });

  /// Auto-assign thay HẲN `selectedRooms`, y hệt kiểu bỏ chọn một phòng ở
  /// nút lưới — cùng một hàng rào cứng phía backend, cùng phải dọn map.
  it("chạy lại auto-assign với danh sách phòng khác cũng dọn khoá override cũ", async () => {
    const user = await pickBothRooms();
    await advanceToSummary(user);
    await screen.findByTestId("group-price-total");

    // Sửa giá R202, rồi quay lại chọn lại bằng auto-assign — lần này auto-
    // assign chỉ trả về R101.
    const displays = await screen.findAllByTestId("rate-display");
    fireEvent.click(displays[1]);
    fireEvent.change(screen.getAllByTestId("rate-input")[0], {
      target: { value: "300000" },
    });

    await user.click(screen.getByRole("button", { name: /Quay lại/i }));
    await user.click(screen.getByRole("button", { name: /Quay lại/i }));

    autoAssignRooms.mockResolvedValue({
      assignments: [{ room: ROOMS[0], floor: 1 }],
    });
    await user.click(screen.getByRole("button", { name: /Auto-assign/i }));
    await user.click(screen.getByRole("button", { name: /Tự động chọn/i }));

    await advanceToSummary(user);
    fireEvent.click(screen.getByRole("button", { name: /Hoàn tất Group Check-in/i }));

    await waitFor(() => expect(groupCheckIn).toHaveBeenCalledTimes(1));
    const req = groupCheckIn.mock.calls[0][0];
    expect(req.rate_override_per_room).toEqual({});
    expect(req.room_ids).toEqual(["R101"]);
  });

  /// Code mẫu của plan lấy nguồn giá bằng `Object.keys(rateOverrides)[0]` —
  /// thứ tự khoá object, không phải thứ tự lễ tân gõ. Ở đây R101 được MỞ ra
  /// (bấm vào giá) trước nên vào map trước, nhưng lễ tân chỉ thực sự SỬA giá
  /// của R202 sau đó. Nguồn đúng phải là R202 (vừa sửa gần nhất) — nếu nút
  /// lấy theo Object.keys()[0] nó sẽ áp nhầm giá vừa-mở-chưa-gõ của R101.
  it("Áp cho tất cả phòng lấy giá của phòng vừa sửa gần nhất, không phải khoá đầu trong object", async () => {
    const user = await pickBothRooms();
    await advanceToSummary(user);
    await screen.findByTestId("group-price-total");

    const displaysBeforeAny = await screen.findAllByTestId("rate-display");
    expect(displaysBeforeAny).toHaveLength(2); // [R101, R202] theo thứ tự chọn

    // Mở giá R101 (chỉ bấm để xem/prefill, KHÔNG gõ số mới).
    fireEvent.click(displaysBeforeAny[0]);

    // Mở và thực sự sửa giá R202 — đây là lần sửa SAU CÙNG.
    const remaining = screen.getAllByTestId("rate-display");
    fireEvent.click(remaining[0]);
    const inputs = screen.getAllByTestId("rate-input");
    fireEvent.change(inputs[1], { target: { value: "700000" } });

    await user.click(screen.getByRole("button", { name: /áp cho tất cả phòng/i }));

    const afterApply = screen.getAllByTestId("rate-input");
    expect((afterApply[0] as HTMLInputElement).value).toBe("700000");
    expect((afterApply[1] as HTMLInputElement).value).toBe("700000");

    fireEvent.click(screen.getByRole("button", { name: /Hoàn tất Group Check-in/i }));

    await waitFor(() => expect(groupCheckIn).toHaveBeenCalledTimes(1));
    const req = groupCheckIn.mock.calls[0][0];
    expect(req.rate_override_per_room).toEqual({ R101: 700000, R202: 700000 });
  });

  /// Task 17 lộ đúng lỗ này: sheet không unmount, chỉ đổi prop `open`, nên
  /// giá tay của một lượt trước rò sang lượt sau nếu không reset đúng chỗ.
  /// Dùng `rerender` (không phải một `render` mới) để giữ nguyên MỘT instance
  /// component — y hệt cách Reservations.tsx chỉ đổi prop `open`, thứ đã sinh
  /// ra lỗi rò giá ở Task 17.
  it("đóng sheet không submit rồi mở lại thì không còn giá cũ", async () => {
    const user = userEvent.setup();
    const { rerender } = render(<GroupCheckinSheet />);

    const fillAndSelectBothRooms = async () => {
      const textboxes = screen.getAllByRole("textbox");
      await user.type(textboxes[0], "Đoàn Hà Nội");
      await user.type(textboxes[1], "Trần Văn B");
      await user.click(screen.getByRole("button", { name: /Tiếp theo/i }));
      await user.click(screen.getByRole("button", { name: /Chọn tay/i }));
      await user.click(screen.getByRole("button", { name: /R101/ }));
      await user.click(screen.getByRole("button", { name: /R202/ }));
      const masterCandidates = screen.getAllByRole("button", { name: /R101/ });
      await user.click(masterCandidates[masterCandidates.length - 1]);
    };

    await fillAndSelectBothRooms();
    await advanceToSummary(user);
    await screen.findByTestId("group-price-total");

    fireEvent.click((await screen.findAllByTestId("rate-display"))[0]);
    fireEvent.change(screen.getAllByTestId("rate-input")[0], {
      target: { value: "400000" },
    });
    await screen.findByTestId("rate-override-total");

    sheetState.open = false;
    rerender(<GroupCheckinSheet />); // effect `[isGroupCheckinOpen]` chạy trên mọi lần đóng
    sheetState.open = true;
    rerender(<GroupCheckinSheet />);

    await fillAndSelectBothRooms();
    await advanceToSummary(user);

    const total = await screen.findByTestId("group-price-total");
    // Chỉ còn giá engine của cả hai phòng — không còn 400.000 của lượt trước.
    expect(total).toHaveTextContent("1.644.500");
    expect(screen.queryByTestId("rate-override-total")).not.toBeInTheDocument();
    expect(screen.getAllByTestId("rate-display")).toHaveLength(2);
  });
});
