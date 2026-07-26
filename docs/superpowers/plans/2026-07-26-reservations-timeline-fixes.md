# Reservations Timeline Fixes Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Sửa bốn lỗi UI/UX của lưới timeline trong tab Reservations — hình học thanh bar theo nửa ngày, booking quá khứ hiện sai vị trí, label tháng đứng yên, không xem được khách đã trả phòng, và bị nhảy tab sau khi check-out.

**Architecture:** Bốn nhóm thay đổi độc lập nhau. (1) `getBookingBars` trong `Reservations.tsx` đổi từ đơn vị ô nguyên sang ngày thập phân và lọc ngoài-vùng trước khi clamp. (2) Cụm điều hướng ngày tính label từ chính dải ngày đang hiển thị, thêm nút "Hôm nay", dời lên toolbar. (3) Popup chi tiết booking tách ra component riêng có hai chế độ, cộng một đường đọc hóa đơn chỉ-đọc trong `useInvoiceDialog`. (4) Store `useHotelStore` thôi ép đổi tab sau check-in/check-out.

**Tech Stack:** React 18 + TypeScript, Zustand, Tailwind, Vitest + Testing Library, Tauri v2 (backend Rust — plan này không đụng backend).

## Global Constraints

- Thư mục làm việc của mọi lệnh: `/Users/binhan/HotelManager/mhm`. Git repo root là `/Users/binhan/HotelManager` (thư mục cha), nên đường dẫn trong lệnh `git add` có tiền tố `mhm/`.
- Lưới timeline hiển thị đúng **16 ngày**, mỗi cột rộng đúng **80px**. Hai hằng số này xuất hiện nhiều chỗ; đặt tên `VISIBLE_DAYS = 16` và `COL_WIDTH = 80` ở đầu `Reservations.tsx` và dùng lại, không viết số trực tiếp trong code mới.
- Chuỗi hiển thị cho người dùng viết bằng tiếng Việt có dấu, khớp giọng văn hiện có trong file.
- Không đụng vào backend Rust (`src-tauri/`). Hai lệnh `get_invoice` và `generate_invoice` đã tồn tại và đã đăng ký.
- Không sửa `RoomDetailPanel.tsx` (code chết, ngoài phạm vi).
- File `src/stores/useHotelStore.test.ts` đang có thay đổi chưa commit trong working tree (test định tuyến `group_checkin`). Task 6 chỉ **thêm** một `describe` mới ở cuối file, không được chạm vào các test có sẵn và không được `git checkout` file này.
- Tiền là số nguyên VND (`MoneyVnd`). Không đưa số thực vào bất kỳ giá trị tiền nào.
- Chạy test: `npm test -- <path>` (vitest run). Kiểm tra kiểu: `npx tsc --noEmit`.

---

### Task 1: Hình học thanh bar theo nửa ngày và lọc booking ngoài vùng

Đổi `getBookingBars` từ đơn vị ô nguyên sang ngày thập phân, offset +0.5 ở cả hai đầu, và chuyển bước lọc ngoài-vùng lên **trước** bước clamp — đây chính là chỗ khiến booking quá khứ dồn về cột đầu lưới.

**Files:**
- Modify: `mhm/src/pages/Reservations.tsx` (kiểu `BookingBar` dòng 18-24; hằng số quanh dòng 26; hàm `getBookingBars` dòng 142-165; JSX render bar dòng 287-312)
- Test: `mhm/src/pages/Reservations.test.tsx`

**Interfaces:**
- Consumes: không có (task đầu tiên).
- Produces:
  - `const VISIBLE_DAYS = 16` và `const COL_WIDTH = 80` — Task 2 dùng lại.
  - Kiểu `BookingBar = BookingWithGuest & { left: number; width: number; clippedLeft: boolean; clippedRight: boolean; color: string; statusLabel: string; isBooked: boolean }` — `left` và `width` tính sẵn theo px.
  - Mỗi thanh bar render kèm `data-testid={`booking-bar-${bar.id}`}` — Task 5 dùng để bấm vào bar.

- [ ] **Step 1: Viết test thất bại cho hình học bar và lọc ngoài vùng**

Mở `mhm/src/pages/Reservations.test.tsx`. Ngay dưới hàm `bookedReservation()` (kết thúc ở dòng 98), thêm hai hàm dựng dữ liệu:

```tsx
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
```

Sau đó thêm khối `describe` mới vào cuối file (sau `describe("Reservations", ...)` đóng ở dòng 154):

```tsx
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
});
```

- [ ] **Step 2: Chạy test để xác nhận nó thất bại**

Run: `npm test -- src/pages/Reservations.test.tsx`
Expected: FAIL — năm test mới đều báo `Unable to find an element by: [data-testid="booking-bar-..."]` (hoặc test "does not render" thất bại vì bar quá khứ vẫn hiện). Hai test cũ (`confirms reservations...`, `cancels reservations...`) vẫn PASS.

- [ ] **Step 3: Thêm hằng số và đổi kiểu BookingBar**

Trong `mhm/src/pages/Reservations.tsx`, thay khối dòng 18-26:

```tsx
type BookingBar = BookingWithGuest & {
    left: number;
    width: number;
    clippedLeft: boolean;
    clippedRight: boolean;
    color: string;
    statusLabel: string;
    isBooked: boolean;
};

const DAY_MS = 24 * 60 * 60 * 1000;
const VISIBLE_DAYS = 16;
const COL_WIDTH = 80;
/** Nhận phòng buổi chiều, trả phòng buổi sáng: bar lệch nửa ô ở cả hai đầu. */
const HALF_DAY = 0.5;
/** Khách nhận và trả cùng ngày vẫn phải nhìn thấy được. */
const MIN_BAR_DAYS = 0.5;
```

Trong `getDateRange` (dòng 45-58), đổi `length: 16` thành `length: VISIBLE_DAYS`.

- [ ] **Step 4: Viết lại getBookingBars**

Thay toàn bộ thân hàm `getBookingBars` (dòng 142-165) bằng:

```tsx
    function getBookingBars(roomId: string): BookingBar[] {
        return visibleBookings
            .filter(b => b.room_id === roomId && b.status !== "cancelled")
            .flatMap((b): BookingBar[] => {
                const checkIn = parseDate(b.scheduled_checkin || b.check_in_at);
                const checkOut = parseDate(b.scheduled_checkout || b.expected_checkout);
                const startDay = DAYS[0].dateObj;

                const rawStart = differenceInCalendarDays(checkIn, startDay) + HALF_DAY;
                const rawEnd = Math.max(
                    differenceInCalendarDays(checkOut, startDay) + HALF_DAY,
                    rawStart + MIN_BAR_DAYS,
                );

                // Lọc trước khi clamp — clamp trước sẽ kéo mọi booking quá khứ về cột 0.
                if (rawStart >= VISIBLE_DAYS || rawEnd <= 0) return [];

                const visStart = Math.max(0, rawStart);
                const visEnd = Math.min(VISIBLE_DAYS, rawEnd);

                return [{
                    ...b,
                    left: visStart * COL_WIDTH,
                    width: (visEnd - visStart) * COL_WIDTH,
                    clippedLeft: rawStart < 0,
                    clippedRight: rawEnd > VISIBLE_DAYS,
                    color: getBookingBarColor(b.status),
                    statusLabel: getStatusLabel(b.status),
                    isBooked: b.status === "booked",
                }];
            })
    }
```

- [ ] **Step 5: Cập nhật JSX render bar**

Thay khối `bars.map(...)` (dòng 287-312) bằng:

```tsx
                                            {bars.map((bar) => (
                                                <div
                                                    key={bar.id}
                                                    data-testid={`booking-bar-${bar.id}`}
                                                    className="absolute top-1/2 -translate-y-1/2 px-0.5 z-10 cursor-pointer"
                                                    style={{ left: `${bar.left}px`, width: `${bar.width}px` }}
                                                    onClick={() => {
                                                        if (bar.isBooked) setSelectedBooking(bar);
                                                        else if (bar.status === "active") setDrawerRoomId(bar.room_id);
                                                    }}
                                                >
                                                    <div className={`h-[42px] w-full ${bar.color} border rounded-xl ${bar.clippedLeft ? "rounded-l-none" : ""} ${bar.clippedRight ? "rounded-r-none" : ""} px-3 flex flex-col justify-center hover:shadow-md hover:-translate-y-0.5 transition-all`}>
                                                        <span className="font-semibold text-xs truncate">{bar.guest_name}</span>
                                                        <div className="flex items-center gap-1.5 mt-0.5">
                                                            <span className="text-[9px] opacity-70">{bar.source || "walk-in"}</span>
                                                            <Badge className={`text-[8px] px-1 py-0 h-3.5 rounded border-0 ${bar.isBooked
                                                                ? "bg-blue-200 text-blue-800"
                                                                : bar.status === "active"
                                                                    ? "bg-emerald-200 text-emerald-800"
                                                                    : "bg-slate-200 text-slate-600"
                                                                }`}>
                                                                {bar.statusLabel}
                                                            </Badge>
                                                        </div>
                                                    </div>
                                                </div>
                                            ))}
```

Còn hai chỗ nữa vẫn dùng số 80 trực tiếp — vạch dọc "hôm nay" ở dòng 284 (`* 80 + 40`). Đổi thành:

```tsx
                                                <div className="absolute top-0 bottom-0 w-[2px] bg-brand-primary/60 z-20" style={{ left: `${DAYS.findIndex(d => d.isToday) * COL_WIDTH + COL_WIDTH / 2}px` }} />
```

- [ ] **Step 6: Chạy test để xác nhận đã pass**

Run: `npm test -- src/pages/Reservations.test.tsx`
Expected: PASS — cả 7 test (2 cũ + 5 mới).

- [ ] **Step 7: Kiểm tra kiểu**

Run: `npx tsc --noEmit`
Expected: không có lỗi. Nếu báo `Property 'startCol' does not exist`, nghĩa là còn chỗ nào đó trong JSX chưa đổi sang `left`/`width`.

- [ ] **Step 8: Commit**

```bash
git add mhm/src/pages/Reservations.tsx mhm/src/pages/Reservations.test.tsx
git commit -m "fix(reservations): half-day bar geometry and drop past bookings from grid"
```

---

### Task 2: Label dải ngày, nút "Hôm nay", và highlight hôm nay khi cuộn

Bỏ state `currentMonth` đứng yên, tính label từ dải ngày đang hiển thị. Thêm nút "Hôm nay". Sửa `isToday` để còn đúng khi đã cuộn tuần. Dời cụm điều hướng lên toolbar.

**Files:**
- Modify: `mhm/src/pages/Reservations.tsx` (`getDateRange` dòng 45-58; state `currentMonth` dòng 89; toolbar dòng 197-231; ô góc lưới dòng 238-245)
- Test: `mhm/src/pages/Reservations.test.tsx`

**Interfaces:**
- Consumes: `VISIBLE_DAYS`, `COL_WIDTH` từ Task 1.
- Produces: `formatRangeLabel(days: ReturnType<typeof getDateRange>): string` — hàm thuần cấp module, không dùng ở task khác nhưng phải tồn tại để test gọi gián tiếp qua UI.

- [ ] **Step 1: Viết test thất bại**

Thêm vào cuối `mhm/src/pages/Reservations.test.tsx` một `describe` mới:

```tsx
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
    expect(screen.queryByRole("button", { name: /hôm nay/i })).toBeNull();

    await user.click(screen.getByRole("button", { name: /tuần sau/i }));
    const todayButton = screen.getByRole("button", { name: /hôm nay/i });
    const shifted = screen.getByTestId("timeline-range-label").textContent;

    await user.click(todayButton);

    expect(screen.queryByRole("button", { name: /hôm nay/i })).toBeNull();
    expect(screen.getByTestId("timeline-range-label").textContent).not.toBe(shifted);
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
```

- [ ] **Step 2: Chạy test để xác nhận nó thất bại**

Run: `npm test -- src/pages/Reservations.test.tsx -t "date navigation"`
Expected: FAIL — `Unable to find an element by: [data-testid="timeline-range-label"]`.

- [ ] **Step 3: Sửa isToday trong getDateRange**

Trong `mhm/src/pages/Reservations.tsx`, thay hàm `getDateRange` (dòng 45-58) bằng:

```tsx
function getDateRange(offset: number) {
    const today = startOfLocalDay(new Date());
    const todayKey = formatLocalDate(today);
    return Array.from({ length: VISIBLE_DAYS }, (_, i) => {
        const d = new Date(today);
        d.setDate(today.getDate() + i - 3 + offset);
        const fullDate = formatLocalDate(d);
        return {
            day: d.toLocaleDateString("vi-VN", { weekday: "short" }).replace(".", ""),
            date: d.getDate(),
            fullDate,
            isToday: fullDate === todayKey,
            dateObj: d,
        };
    });
}
```

- [ ] **Step 4: Thêm hàm formatRangeLabel**

Ngay dưới `getDateRange`, thêm:

```tsx
type TimelineDay = ReturnType<typeof getDateRange>[number];

function formatRangeLabel(days: TimelineDay[]): string {
    const first = days[0].dateObj;
    const last = days[days.length - 1].dateObj;
    const firstMonth = first.getMonth() + 1;
    const lastMonth = last.getMonth() + 1;
    const firstYear = first.getFullYear();
    const lastYear = last.getFullYear();

    if (firstYear !== lastYear) {
        return `${firstMonth}/${firstYear} – ${lastMonth}/${lastYear}`;
    }

    if (firstMonth !== lastMonth) {
        return `THÁNG ${firstMonth}–${lastMonth} / ${firstYear}`;
    }

    return `THÁNG ${firstMonth} NĂM ${firstYear}`;
}
```

- [ ] **Step 5: Bỏ state currentMonth**

Xóa dòng 89:

```tsx
    const [currentMonth] = useState(new Date().toLocaleDateString("vi-VN", { month: "long", year: "numeric" }));
```

Ngay dưới `const DAYS = getDateRange(dateOffset);` thêm:

```tsx
    const rangeLabel = formatRangeLabel(DAYS);
```

Nếu `useState` không còn chỗ dùng nào khác trong file thì bỏ nó khỏi dòng import 1 — kiểm tra bằng `grep -n "useState" src/pages/Reservations.tsx` (còn nhiều state khác nên nhiều khả năng vẫn giữ).

- [ ] **Step 6: Dời cụm điều hướng lên toolbar**

Trong khối toolbar, thay thẻ mở của nhóm bên trái (dòng 198) và chèn cụm điều hướng vào **sau** nhóm badge, tức là thay dòng 213 `<div className="flex items-center gap-3">` (nhóm bên phải) bằng:

```tsx
                <div className="flex items-center gap-3">
                    <div className="flex items-center gap-1.5 pr-3 border-r border-slate-100">
                        <button
                            aria-label="Tuần trước"
                            className="text-slate-400 hover:text-slate-600 cursor-pointer p-1"
                            onClick={() => setDateOffset(o => o - 7)}
                        >
                            <ChevronLeft size={16} />
                        </button>
                        <span
                            data-testid="timeline-range-label"
                            className="text-xs font-bold text-slate-600 uppercase whitespace-nowrap min-w-[150px] text-center"
                        >
                            {rangeLabel}
                        </span>
                        <button
                            aria-label="Tuần sau"
                            className="text-slate-400 hover:text-slate-600 cursor-pointer p-1"
                            onClick={() => setDateOffset(o => o + 7)}
                        >
                            <ChevronRight size={16} />
                        </button>
                        {dateOffset !== 0 && (
                            <Button
                                size="sm"
                                variant="outline"
                                className="h-8 px-3 rounded-lg text-xs cursor-pointer"
                                onClick={() => setDateOffset(0)}
                            >
                                Hôm nay
                            </Button>
                        )}
                    </div>
```

Giữ nguyên phần ô tìm kiếm và nút "Đặt phòng" bên dưới, và giữ nguyên thẻ `</div>` đóng nhóm.

- [ ] **Step 7: Dọn ô góc lưới**

Thay khối ô góc (dòng 238-245) bằng:

```tsx
                    <div className="w-[140px] shrink-0 border-r border-slate-100 bg-white shadow-[2px_0_10px_rgba(0,0,0,0.02)] sticky left-0 z-20 flex items-center px-4">
                        <span className="text-xs font-semibold text-slate-500">Rooms</span>
                    </div>
```

- [ ] **Step 8: Đánh dấu vạch "hôm nay" cho test**

Trong khối vạch dọc (đã sửa ở Task 1 Step 5), thêm `data-testid`:

```tsx
                                            {DAYS.some(d => d.isToday) && (
                                                <div data-testid="timeline-today-marker" className="absolute top-0 bottom-0 w-[2px] bg-brand-primary/60 z-20" style={{ left: `${DAYS.findIndex(d => d.isToday) * COL_WIDTH + COL_WIDTH / 2}px` }} />
                                            )}
```

Lưu ý: vạch này render một lần cho **mỗi phòng**, nên test dùng `getAllByTestId` nếu có nhiều phòng. Mock store trong test chỉ có một phòng `R101` nên `getByTestId` chạy đúng.

- [ ] **Step 9: Chạy test**

Run: `npm test -- src/pages/Reservations.test.tsx`
Expected: PASS — toàn bộ 10 test.

- [ ] **Step 10: Kiểm tra kiểu**

Run: `npx tsc --noEmit`
Expected: không lỗi. Nếu báo `'ChevronLeft' is declared but its value is never read` thì có chỗ import thừa — nhưng cả hai icon vẫn dùng ở toolbar nên không nên xảy ra.

- [ ] **Step 11: Commit**

```bash
git add mhm/src/pages/Reservations.tsx mhm/src/pages/Reservations.test.tsx
git commit -m "fix(reservations): live range label, today button, and today marker while paging"
```

---

### Task 3: Đường đọc hóa đơn chỉ-đọc trong useInvoiceDialog

Thêm `viewInvoice(bookingId)` — gọi lệnh chỉ-đọc `get_invoice` trước, chỉ fallback sang lệnh ghi `generate_invoice` khi chưa có hóa đơn. Xem lại khách đã trả phòng không nên sinh bản ghi ledger.

**Files:**
- Modify: `mhm/src/hooks/useInvoiceDialog.ts`
- Test: `mhm/src/hooks/useInvoiceDialog.test.ts`

**Interfaces:**
- Consumes: không có.
- Produces: `viewInvoice: (bookingId: string) => Promise<void>` trả về từ `useInvoiceDialog()`, cùng bộ với `openInvoice`, `invoiceOpen`, `invoiceData`, `invoiceLoading`, `closeInvoice`. Task 5 dùng.

- [ ] **Step 1: Viết test thất bại**

Trong `mhm/src/hooks/useInvoiceDialog.test.ts`, thêm mock cho `invoke` ngay dưới khối `vi.mock("sonner", ...)` (dòng 12-16):

```tsx
const invoke = vi.hoisted(() => vi.fn());

vi.mock("@tauri-apps/api/core", () => ({
  invoke,
}));
```

(`const invoke` phải đặt cùng nhóm với các `vi.hoisted` khác ở đầu file, tức ngay dưới dòng 6 `const toastError = ...`; khối `vi.mock` đặt cạnh các `vi.mock` khác.)

Rồi thêm vào cuối `describe("useInvoiceDialog", ...)` — trước dấu `});` cuối file:

```tsx
  const existingInvoice = {
    id: "invoice-9",
    invoice_number: "INV-20260726-009",
    booking_id: "booking-9",
    hotel_name: "CapyInn",
    hotel_address: "",
    hotel_phone: "",
    room_name: "1B",
    room_type: "standard",
    guest_name: "Hoseo Kim",
    guest_phone: null,
    check_in: "2026-07-23",
    check_out: "2026-07-25",
    nights: 2,
    pricing_breakdown: [{ label: "2 night(s) x 600000d", amount: 1200000 }],
    subtotal: 1200000,
    deposit_amount: 0,
    total: 1200000,
    balance_due: 0,
    policy_text: null,
    notes: null,
    status: "issued",
    created_at: "2026-07-25T09:12:00+07:00",
  } satisfies InvoiceData;

  it("opens an existing invoice without touching the write command", async () => {
    invoke.mockResolvedValueOnce(existingInvoice);
    const { result } = renderHook(() => useInvoiceDialog());

    await act(async () => {
      await result.current.viewInvoice("booking-9");
    });

    expect(invoke).toHaveBeenCalledWith("get_invoice", { bookingId: "booking-9" });
    expect(invokeWriteCommand).not.toHaveBeenCalled();
    expect(result.current.invoiceOpen).toBe(true);
    expect(result.current.invoiceData).toBe(existingInvoice);
    expect(result.current.invoiceLoading).toBe(false);
  });

  it("falls back to generating when no invoice exists yet", async () => {
    invoke.mockResolvedValueOnce(null);
    invokeWriteCommand.mockResolvedValueOnce(existingInvoice);
    const { result } = renderHook(() => useInvoiceDialog());

    await act(async () => {
      await result.current.viewInvoice("booking-9");
    });

    expect(invoke).toHaveBeenCalledWith("get_invoice", { bookingId: "booking-9" });
    expect(invokeWriteCommand).toHaveBeenCalledWith("generate_invoice", {
      bookingId: "booking-9",
    });
    expect(result.current.invoiceOpen).toBe(true);
    expect(result.current.invoiceData).toBe(existingInvoice);
  });

  it("shows an error when reading the invoice fails", async () => {
    invoke.mockRejectedValueOnce(new Error("db down"));
    const { result } = renderHook(() => useInvoiceDialog());

    await act(async () => {
      await result.current.viewInvoice("booking-9");
    });

    expect(toastError).toHaveBeenCalledWith("Lỗi tạo invoice: Error: db down");
    expect(result.current.invoiceOpen).toBe(false);
    expect(result.current.invoiceLoading).toBe(false);
  });
```

- [ ] **Step 2: Chạy test để xác nhận nó thất bại**

Run: `npm test -- src/hooks/useInvoiceDialog.test.ts`
Expected: FAIL — `result.current.viewInvoice is not a function`. Hai test cũ vẫn PASS.

- [ ] **Step 3: Cài đặt viewInvoice**

Thay toàn bộ `mhm/src/hooks/useInvoiceDialog.ts` bằng:

```tsx
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";

import type { InvoiceData } from "@/components/InvoicePDF";
import { invokeWriteCommand } from "@/lib/invokeCommand";

export function useInvoiceDialog() {
    const [invoiceOpen, setInvoiceOpen] = useState(false);
    const [invoiceData, setInvoiceData] = useState<InvoiceData | null>(null);
    const [invoiceLoading, setInvoiceLoading] = useState(false);

    const openInvoice = async (bookingId: string) => {
        setInvoiceLoading(true);
        try {
            const data = await invokeWriteCommand<InvoiceData>("generate_invoice", { bookingId });
            setInvoiceData(data);
            setInvoiceOpen(true);
        } catch (err) {
            toast.error("Lỗi tạo invoice: " + err);
        } finally {
            setInvoiceLoading(false);
        }
    };

    /**
     * Xem hóa đơn của một booking đã khép lại: đọc bản đã phát hành trước,
     * chỉ sinh mới khi chưa có. Tránh ghi ledger cho một thao tác chỉ để xem.
     */
    const viewInvoice = async (bookingId: string) => {
        setInvoiceLoading(true);
        try {
            const existing = await invoke<InvoiceData | null>("get_invoice", { bookingId });
            const data = existing ?? await invokeWriteCommand<InvoiceData>("generate_invoice", { bookingId });
            setInvoiceData(data);
            setInvoiceOpen(true);
        } catch (err) {
            toast.error("Lỗi tạo invoice: " + err);
        } finally {
            setInvoiceLoading(false);
        }
    };

    const closeInvoice = () => {
        setInvoiceOpen(false);
    };

    return {
        invoiceOpen,
        invoiceData,
        invoiceLoading,
        openInvoice,
        viewInvoice,
        closeInvoice,
    };
}
```

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `npm test -- src/hooks/useInvoiceDialog.test.ts`
Expected: PASS — cả 5 test.

- [ ] **Step 5: Commit**

```bash
git add mhm/src/hooks/useInvoiceDialog.ts mhm/src/hooks/useInvoiceDialog.test.ts
git commit -m "feat(invoice): add read-first viewInvoice for closed bookings"
```

---

### Task 4: Tách BookingDetailPopup với hai chế độ

Đưa popup ~60 dòng đang nội tuyến trong `Reservations.tsx` ra component riêng, thêm chế độ chỉ-đọc cho khách đã trả phòng. Task này chỉ tạo component và test của nó; việc đấu vào trang nằm ở Task 5.

**Files:**
- Create: `mhm/src/components/BookingDetailPopup.tsx`
- Test: `mhm/src/components/BookingDetailPopup.test.tsx`

**Interfaces:**
- Consumes: `BookingWithGuest` từ `@/types`, `fmtNumber` từ `@/lib/format`, `Button` từ `@/components/ui/button`.
- Produces:

```tsx
interface BookingDetailPopupProps {
    booking: BookingWithGuest;
    mode: "reservation" | "readonly";
    onClose: () => void;
    onConfirm?: (bookingId: string) => void;
    onEdit?: (booking: BookingWithGuest) => void;
    onCancel?: (bookingId: string) => void;
    onViewInvoice?: (bookingId: string) => void;
}
export default function BookingDetailPopup(props: BookingDetailPopupProps): JSX.Element
```

- [ ] **Step 1: Viết test thất bại**

Tạo `mhm/src/components/BookingDetailPopup.test.tsx`:

```tsx
import type { ButtonHTMLAttributes } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

vi.mock("@/components/ui/button", () => ({
  Button: ({ children, ...props }: ButtonHTMLAttributes<HTMLButtonElement>) => (
    <button {...props}>{children}</button>
  ),
}));

import BookingDetailPopup from "./BookingDetailPopup";
import type { BookingWithGuest } from "@/types";

function booking(overrides: Partial<BookingWithGuest> = {}): BookingWithGuest {
  return {
    id: "B-1",
    room_id: "1B",
    room_name: "Room 1B",
    guest_name: "Hoseo Kim",
    check_in_at: "2026-07-23",
    expected_checkout: "2026-07-25",
    actual_checkout: "2026-07-25T09:12:00+07:00",
    nights: 2,
    total_price: 1200000,
    paid_amount: 1200000,
    status: "checked_out",
    source: "phone",
    booking_type: null,
    deposit_amount: 0,
    scheduled_checkin: "2026-07-23",
    scheduled_checkout: "2026-07-25",
    guest_phone: "0900000000",
    ...overrides,
  } as BookingWithGuest;
}

describe("BookingDetailPopup", () => {
  it("shows reservation actions in reservation mode", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    const onEdit = vi.fn();
    const onCancel = vi.fn();

    render(
      <BookingDetailPopup
        booking={booking({ status: "booked", actual_checkout: null })}
        mode="reservation"
        onClose={vi.fn()}
        onConfirm={onConfirm}
        onEdit={onEdit}
        onCancel={onCancel}
      />,
    );

    await user.click(screen.getByRole("button", { name: /check-in/i }));
    expect(onConfirm).toHaveBeenCalledWith("B-1");

    await user.click(screen.getByRole("button", { name: /chỉnh sửa/i }));
    expect(onEdit).toHaveBeenCalledTimes(1);

    await user.click(screen.getByRole("button", { name: /hủy/i }));
    expect(onCancel).toHaveBeenCalledWith("B-1");

    expect(screen.queryByRole("button", { name: /xem hóa đơn/i })).toBeNull();
  });

  it("shows a read-only view with the actual checkout time for closed bookings", async () => {
    const user = userEvent.setup();
    const onViewInvoice = vi.fn();
    const onClose = vi.fn();

    render(
      <BookingDetailPopup
        booking={booking()}
        mode="readonly"
        onClose={onClose}
        onViewInvoice={onViewInvoice}
      />,
    );

    expect(screen.getByText(/Đã trả — Hoseo Kim/)).toBeTruthy();
    expect(screen.getByText("Trả phòng lúc")).toBeTruthy();
    expect(screen.getByText("Đã thanh toán")).toBeTruthy();

    expect(screen.queryByRole("button", { name: /check-in/i })).toBeNull();
    expect(screen.queryByRole("button", { name: /^hủy$/i })).toBeNull();

    await user.click(screen.getByRole("button", { name: /xem hóa đơn/i }));
    expect(onViewInvoice).toHaveBeenCalledWith("B-1");

    await user.click(screen.getByRole("button", { name: /đóng/i }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("falls back to the expected checkout when the actual one is missing", () => {
    render(
      <BookingDetailPopup
        booking={booking({ actual_checkout: null })}
        mode="readonly"
        onClose={vi.fn()}
        onViewInvoice={vi.fn()}
      />,
    );

    expect(screen.getByText("2026-07-25")).toBeTruthy();
  });
});
```

- [ ] **Step 2: Chạy test để xác nhận nó thất bại**

Run: `npm test -- src/components/BookingDetailPopup.test.tsx`
Expected: FAIL — `Failed to resolve import "./BookingDetailPopup"`.

- [ ] **Step 3: Tạo component**

Tạo `mhm/src/components/BookingDetailPopup.tsx`:

```tsx
import { CheckCircle2, XCircle, Pencil, FileText } from "lucide-react";

import { Button } from "@/components/ui/button";
import { fmtNumber } from "@/lib/format";
import type { BookingWithGuest } from "@/types";

interface BookingDetailPopupProps {
    booking: BookingWithGuest;
    mode: "reservation" | "readonly";
    onClose: () => void;
    onConfirm?: (bookingId: string) => void;
    onEdit?: (booking: BookingWithGuest) => void;
    onCancel?: (bookingId: string) => void;
    onViewInvoice?: (bookingId: string) => void;
}

function Row({ label, value }: { label: string; value: string }) {
    return (
        <div className="flex justify-between">
            <span>{label}</span>
            <span className="font-semibold">{value}</span>
        </div>
    );
}

export default function BookingDetailPopup({
    booking,
    mode,
    onClose,
    onConfirm,
    onEdit,
    onCancel,
    onViewInvoice,
}: BookingDetailPopupProps) {
    const isReadonly = mode === "readonly";
    const title = isReadonly
        ? `Đã trả — ${booking.guest_name}`
        : `Reservation — ${booking.guest_name}`;
    const checkInText = booking.scheduled_checkin || booking.check_in_at;
    const checkOutText = isReadonly
        ? booking.actual_checkout || booking.scheduled_checkout || booking.expected_checkout
        : booking.scheduled_checkout || booking.expected_checkout;

    return (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30" onClick={onClose}>
            <div className="bg-white rounded-2xl shadow-2xl p-6 w-[380px] space-y-4" onClick={(e) => e.stopPropagation()}>
                <h3 className="font-bold text-lg text-slate-800">{title}</h3>
                <div className="space-y-2 text-sm text-slate-600">
                    <Row label="Phòng" value={booking.room_id} />
                    <Row label="Check-in" value={checkInText} />
                    <Row label={isReadonly ? "Trả phòng lúc" : "Check-out"} value={checkOutText} />
                    <Row label="Số đêm" value={String(booking.nights)} />
                    <div className="flex justify-between">
                        <span>Tổng tiền</span>
                        <span className="font-bold text-slate-800">{fmtNumber(booking.total_price)}₫</span>
                    </div>
                    {isReadonly ? (
                        <div className="flex justify-between">
                            <span>Đã thanh toán</span>
                            <span className="font-semibold text-emerald-600">{fmtNumber(booking.paid_amount)}₫</span>
                        </div>
                    ) : (
                        (booking.deposit_amount || 0) > 0 && (
                            <div className="flex justify-between">
                                <span>Đã cọc</span>
                                <span className="font-semibold text-emerald-600">{fmtNumber(booking.deposit_amount || 0)}₫</span>
                            </div>
                        )
                    )}
                    {booking.guest_phone && <Row label="SĐT" value={booking.guest_phone} />}
                </div>

                {isReadonly ? (
                    <div className="flex gap-2 pt-2">
                        <Button
                            className="flex-1 bg-slate-700 hover:bg-slate-800 text-white rounded-xl h-10 gap-1.5 cursor-pointer"
                            onClick={() => onViewInvoice?.(booking.id)}
                        >
                            <FileText size={14} /> Xem hóa đơn
                        </Button>
                        <Button
                            variant="outline"
                            className="flex-1 border-slate-200 text-slate-600 hover:bg-slate-50 rounded-xl h-10 cursor-pointer"
                            onClick={onClose}
                        >
                            Đóng
                        </Button>
                    </div>
                ) : (
                    <div className="flex gap-2 pt-2">
                        <Button
                            className="flex-1 bg-emerald-600 hover:bg-emerald-700 text-white rounded-xl h-10 gap-1.5 cursor-pointer"
                            onClick={() => onConfirm?.(booking.id)}
                        >
                            <CheckCircle2 size={14} /> Check-in
                        </Button>
                        <Button
                            variant="outline"
                            className="flex-1 border-blue-200 text-blue-600 hover:bg-blue-50 rounded-xl h-10 gap-1.5 cursor-pointer"
                            onClick={() => onEdit?.(booking)}
                        >
                            <Pencil size={14} /> Chỉnh sửa
                        </Button>
                        <Button
                            variant="outline"
                            className="flex-1 border-red-200 text-red-600 hover:bg-red-50 rounded-xl h-10 gap-1.5 cursor-pointer"
                            onClick={() => onCancel?.(booking.id)}
                        >
                            <XCircle size={14} /> Hủy
                        </Button>
                    </div>
                )}
            </div>
        </div>
    );
}
```

- [ ] **Step 4: Chạy test để xác nhận pass**

Run: `npm test -- src/components/BookingDetailPopup.test.tsx`
Expected: PASS — cả 3 test.

- [ ] **Step 5: Kiểm tra kiểu**

Run: `npx tsc --noEmit`
Expected: không lỗi.

- [ ] **Step 6: Commit**

```bash
git add mhm/src/components/BookingDetailPopup.tsx mhm/src/components/BookingDetailPopup.test.tsx
git commit -m "feat(reservations): extract BookingDetailPopup with a read-only mode"
```

---

### Task 5: Đấu popup vào trang Reservations và mở được khách đã trả phòng

Thay popup nội tuyến bằng component vừa tạo, cho bar xám bấm được, nối nút "Xem hóa đơn" vào `viewInvoice`.

**Files:**
- Modify: `mhm/src/pages/Reservations.tsx` (import; handler click bar; khối popup nội tuyến dòng ~331-393 sau các task trước; phần render cuối)
- Test: `mhm/src/pages/Reservations.test.tsx`

**Interfaces:**
- Consumes: `BookingDetailPopup` (Task 4), `viewInvoice` từ `useInvoiceDialog` (Task 3), `data-testid="booking-bar-<id>"` (Task 1).
- Produces: không có.

- [ ] **Step 1: Viết test thất bại**

Trong `mhm/src/pages/Reservations.test.tsx`, mock `InvoiceDialog` cạnh các mock component khác (sau khối mock `RoomDrawer` ở dòng 57-59):

```tsx
vi.mock("@/components/InvoiceDialog", () => ({
  default: ({ open }: { open: boolean }) => (open ? <div>invoice-dialog</div> : null),
}));
```

Rồi thêm `describe` mới vào cuối file:

```tsx
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
  });
});
```

- [ ] **Step 2: Chạy test để xác nhận nó thất bại**

Run: `npm test -- src/pages/Reservations.test.tsx -t "checked-out"`
Expected: FAIL — popup không mở, `Unable to find an element with the text: /Đã trả — Hoseo Kim/`.

- [ ] **Step 3: Thêm import và hook vào Reservations.tsx**

Trong `mhm/src/pages/Reservations.tsx`, thêm vào khối import:

```tsx
import BookingDetailPopup from "@/components/BookingDetailPopup";
import InvoiceDialog from "@/components/InvoiceDialog";
import { useInvoiceDialog } from "@/hooks/useInvoiceDialog";
```

Bỏ `fmtNumber` khỏi dòng import `@/lib/format` nếu sau khi xóa popup nội tuyến không còn chỗ nào dùng (kiểm tra bằng `grep -n "fmtNumber" src/pages/Reservations.tsx`). Cũng bỏ `CheckCircle2, XCircle, Pencil` khỏi import `lucide-react` — chúng chuyển hết sang `BookingDetailPopup`; giữ lại `Search, ChevronLeft, ChevronRight, Plus`.

Trong thân component, ngay dưới `const [drawerRoomId, setDrawerRoomId] = useState<string | null>(null);` thêm:

```tsx
    const { invoiceOpen, invoiceData, viewInvoice, closeInvoice } = useInvoiceDialog();
```

- [ ] **Step 4: Cho bar xám bấm được**

Trong `onClick` của thanh bar (Task 1 Step 5), thay ba dòng điều phối bằng:

```tsx
                                                    onClick={() => {
                                                        if (bar.status === "active") setDrawerRoomId(bar.room_id);
                                                        else setSelectedBooking(bar);
                                                    }}
```

`getBookingBars` đã loại `cancelled`, nên nhánh `else` chỉ nhận `booked` và `checked_out`.

- [ ] **Step 5: Thay popup nội tuyến bằng component**

Xóa toàn bộ khối `{selectedBooking && (<div className="fixed inset-0 ...">...</div>)}` (khối popup nội tuyến, bắt đầu bằng comment `{/* Reservation Action Popup */}`) và thay bằng:

```tsx
            {selectedBooking && (
                <BookingDetailPopup
                    booking={selectedBooking}
                    mode={selectedBooking.status === "checked_out" ? "readonly" : "reservation"}
                    onClose={() => setSelectedBooking(null)}
                    onConfirm={handleConfirmReservation}
                    onEdit={(booking) => { setEditBooking(booking); setSelectedBooking(null); }}
                    onCancel={handleCancelReservation}
                    onViewInvoice={viewInvoice}
                />
            )}
```

- [ ] **Step 6: Render InvoiceDialog**

Ngay trước `<ReservationSheet ... />` ở cuối phần render, thêm:

```tsx
            <InvoiceDialog
                open={invoiceOpen}
                onOpenChange={(nextOpen) => {
                    if (!nextOpen) closeInvoice();
                }}
                data={invoiceData}
            />
```

- [ ] **Step 7: Chạy test**

Run: `npm test -- src/pages/Reservations.test.tsx`
Expected: PASS — toàn bộ 12 test. Hai test cũ (`confirms reservations...`, `cancels reservations...`) vẫn phải xanh: chúng bấm vào text tên khách rồi bấm nút, luồng đó không đổi.

- [ ] **Step 8: Kiểm tra kiểu**

Run: `npx tsc --noEmit`
Expected: không lỗi. Nếu báo import thừa (`fmtNumber`, `CheckCircle2`, `XCircle`, `Pencil`) thì dọn nốt ở Step 3.

- [ ] **Step 9: Commit**

```bash
git add mhm/src/pages/Reservations.tsx mhm/src/pages/Reservations.test.tsx
git commit -m "feat(reservations): open a read-only detail popup for checked-out guests"
```

---

### Task 6: Store thôi ép đổi tab sau check-in/check-out

Bỏ `activeTab: "dashboard"` khỏi `checkIn` và `checkOut`. Điều hướng thuộc về trang gọi.

**Files:**
- Modify: `mhm/src/stores/useHotelStore.ts` (dòng 139-142 trong `checkIn`; dòng 173-176 trong `checkOut`)
- Test: `mhm/src/stores/useHotelStore.test.ts` (chỉ **thêm** `describe` mới ở cuối; file này có thay đổi chưa commit, không được đụng vào phần cũ)

**Interfaces:**
- Consumes: không có.
- Produces: không có.

- [ ] **Step 1: Viết test thất bại**

Thêm vào cuối `mhm/src/stores/useHotelStore.test.ts`, **sau** dấu `});` đóng `describe("useHotelStore monitoring context", ...)`:

```tsx
describe("useHotelStore navigation side effects", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.spyOn(console, "error").mockImplementation(() => {});
    createCorrelationId.mockReturnValue("COR-1A2B3C4D");
    invokeWriteCommand.mockResolvedValue(undefined);
    invoke.mockImplementation(async (command: string) => {
      if (command === "get_rooms") return [];
      if (command === "get_housekeeping_tasks") return [];
      if (command === "get_dashboard_stats") {
        return { total_rooms: 10, occupied: 2, vacant: 8, cleaning: 0, revenue_today: 0 };
      }
      throw new Error(`Unhandled invoke ${command}`);
    });
    useHotelStore.setState({
      rooms: [],
      stats: null,
      dashboardRefreshVersion: 0,
      activeTab: "reservations",
      loading: false,
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("stays on the current tab after check-out and still bumps the dashboard version", async () => {
    await useHotelStore.getState().checkOut("booking-1", "full", 500000);

    expect(useHotelStore.getState().activeTab).toBe("reservations");
    expect(useHotelStore.getState().dashboardRefreshVersion).toBe(1);
  });

  it("stays on the current tab after check-in", async () => {
    await useHotelStore.getState().checkIn(
      "101",
      [{ full_name: "Nguyen Van A", doc_number: "012345678901" }],
      1,
      500000,
      "walk-in",
      null,
    );

    expect(useHotelStore.getState().activeTab).toBe("reservations");
    expect(useHotelStore.getState().dashboardRefreshVersion).toBe(1);
  });
});
```

Nếu chữ ký `checkIn` trong store khác thứ tự tham số trên, mở `mhm/src/stores/useHotelStore.ts` xem khai báo `checkIn` trong interface `HotelStore` và chỉnh lời gọi cho khớp — test hiện có ở dòng ~79 trong cùng file là mẫu tham chiếu.

- [ ] **Step 2: Chạy test để xác nhận nó thất bại**

Run: `npm test -- src/stores/useHotelStore.test.ts -t "navigation side effects"`
Expected: FAIL — `expected 'dashboard' to be 'reservations'` ở cả hai test.

- [ ] **Step 3: Bỏ ép đổi tab**

Trong `mhm/src/stores/useHotelStore.ts`, trong `checkIn` thay:

```tsx
        set((state) => ({
          activeTab: "dashboard",
          dashboardRefreshVersion: state.dashboardRefreshVersion + 1,
        }));
```

bằng:

```tsx
        set((state) => ({
          dashboardRefreshVersion: state.dashboardRefreshVersion + 1,
        }));
```

Làm y hệt trong `checkOut`. Hai chỗ này là **hai lần xuất hiện duy nhất** còn lại của `activeTab: "dashboard"` ngoài giá trị khởi tạo store ở dòng 81 — giữ nguyên dòng 81.

Kiểm tra lại: `grep -n 'activeTab: "dashboard"' src/stores/useHotelStore.ts` phải chỉ còn đúng một dòng (dòng khởi tạo).

- [ ] **Step 4: Chạy test store**

Run: `npm test -- src/stores/useHotelStore.test.ts`
Expected: PASS — toàn bộ test trong file, gồm cả các test có sẵn về monitoring context.

- [ ] **Step 5: Chạy toàn bộ test để bắt hồi quy**

Run: `npm test`
Expected: PASS toàn bộ. Nếu có test nào ở `src/App.*.test.tsx` khẳng định tab nhảy về dashboard sau check-out thì sửa test đó cho khớp hành vi mới — đó là hành vi cũ đang bị sửa, không phải hồi quy.

- [ ] **Step 6: Kiểm tra kiểu**

Run: `npx tsc --noEmit`
Expected: không lỗi.

- [ ] **Step 7: Commit**

```bash
git add mhm/src/stores/useHotelStore.ts mhm/src/stores/useHotelStore.test.ts
git commit -m "fix(store): keep the current tab after check-in and check-out"
```

---

### Task 7: Kiểm chứng thủ công trên app thật

Bốn lỗi đều là lỗi cảm nhận bằng mắt. Test tự động phủ logic; bước này xác nhận trên UI thật.

**Files:** không sửa file nào.

**Interfaces:**
- Consumes: toàn bộ Task 1-6.
- Produces: không có.

- [ ] **Step 1: Chạy app**

Run: `npm run tauri dev`
Expected: cửa sổ CapyInn mở, đăng nhập được vào tab Reservations.

- [ ] **Step 2: Kiểm hình học bar**

Mở tab Reservations, tìm một khách đang ở nhiều đêm.
Expected: bar bắt đầu ở **giữa** ô ngày nhận phòng và kết thúc ở **giữa** ô ngày trả phòng. Hai khách nối tiếp cùng phòng chạm nhau ở giữa ô, không chồng lên nhau.

- [ ] **Step 3: Kiểm điều hướng**

Bấm mũi tên phải vài lần.
Expected: label đổi theo dải ngày (ví dụ `THÁNG 7–8 / 2026` rồi `THÁNG 8 NĂM 2026`); nút "Hôm nay" xuất hiện; **không** còn booking cũ dính ở cột đầu lưới. Bấm "Hôm nay" thì quay về dải chứa hôm nay và nút biến mất. Bấm mũi tên trái một lần: vạch xanh "hôm nay" vẫn còn vì hôm nay vẫn trong lưới.

- [ ] **Step 4: Kiểm khách đã trả phòng**

Bấm vào một bar màu xám.
Expected: popup "Đã trả — <tên>" hiện ra với dòng "Trả phòng lúc" và "Đã thanh toán", chỉ có hai nút "Xem hóa đơn" và "Đóng". Bấm "Xem hóa đơn" mở được hóa đơn.

- [ ] **Step 5: Kiểm không nhảy tab**

Từ tab Reservations, bấm vào một bar xanh lá (khách đang ở) để mở drawer, bấm Check-out và xác nhận.
Expected: drawer đóng, **vẫn ở tab Reservations**, bar khách đó chuyển sang màu xám.

- [ ] **Step 6: Commit (nếu có chỉnh sửa phát sinh)**

Nếu bước kiểm chứng lộ ra chỗ cần chỉnh, sửa rồi commit; nếu không thì bỏ qua step này.

```bash
git add -A mhm/src
git commit -m "fix(reservations): manual verification adjustments"
```

---

## Tóm tắt thay đổi

| File | Loại | Task |
|---|---|---|
| `mhm/src/pages/Reservations.tsx` | Sửa | 1, 2, 5 |
| `mhm/src/pages/Reservations.test.tsx` | Sửa | 1, 2, 5 |
| `mhm/src/components/BookingDetailPopup.tsx` | Tạo mới | 4 |
| `mhm/src/components/BookingDetailPopup.test.tsx` | Tạo mới | 4 |
| `mhm/src/hooks/useInvoiceDialog.ts` | Sửa | 3 |
| `mhm/src/hooks/useInvoiceDialog.test.ts` | Sửa | 3 |
| `mhm/src/stores/useHotelStore.ts` | Sửa | 6 |
| `mhm/src/stores/useHotelStore.test.ts` | Sửa (chỉ thêm cuối file) | 6 |

Task 1, 2, 3, 4 độc lập nhau, có thể làm song song. Task 5 cần xong 1, 3, 4. Task 6 độc lập. Task 7 cần xong hết.
