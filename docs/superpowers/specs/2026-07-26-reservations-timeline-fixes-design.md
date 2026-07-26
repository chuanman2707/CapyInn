# Reservations Timeline — Bốn lỗi UI/UX

Ngày: 2026-07-26
Phạm vi: `mhm/src/pages/Reservations.tsx`, `mhm/src/stores/useHotelStore.ts`, một component mới, một hook mở rộng.

## Bối cảnh

Lưới timeline ở tab Reservations hiển thị 16 ngày, mỗi cột rộng 80px, mỗi phòng một hàng.
Người dùng báo bốn vấn đề. Ba trong số đó là lỗi logic thật, một là quyết định điều hướng
đặt sai tầng (store thay vì trang gọi).

| # | Triệu chứng | Nguyên nhân | Vị trí |
|---|---|---|---|
| 1 | Khách ở 25–29 nhưng bar chỉ tô hết ngày 28 | Bar vẽ theo ô nguyên `[startCol, endCol)` | `Reservations.tsx:150-152` |
| 2a | Booking đã kết thúc vẫn hiện ở cột đầu lưới | `startCol` bị clamp về 0 **trước** bước lọc ngoài vùng | `Reservations.tsx:150-154` |
| 2b | Label "THÁNG 7 NĂM 2026" không đổi khi chuyển tuần | `useState(new Date()...)` khởi tạo một lần | `Reservations.tsx:89` |
| 2d | Lùi một tuần thì mất highlight "hôm nay" dù hôm nay còn trong lưới | `isToday: i === 3 && offset === 0` | `Reservations.tsx:54` |
| 3 | Bấm bar xám (đã trả phòng) không ra gì | `onClick` chỉ có nhánh `booked` và `active` | `Reservations.tsx:292-295` |
| 4 | Check-out xong bị nhảy về tab Overview | Store hard-code `activeTab: "dashboard"` | `useHotelStore.ts:140, 174` |

## Quyết định thiết kế

Bốn quyết định đã chốt với người dùng:

1. **Hình học bar:** nửa ngày ở cả hai đầu (chuẩn Cloudbeds/Mews), không phải chỉ nửa ngày cuối.
2. **Điều hướng:** mũi tên giữ nhịp nhảy 7 ngày, label tự cập nhật, thêm nút "Hôm nay".
3. **Khách đã trả phòng:** popup chỉ-đọc kèm nút "Xem hóa đơn".
4. **Phạm vi sửa tab:** bỏ ép đổi tab ở cả `checkIn` lẫn `checkOut`.

## 1. Hình học thanh bar

Chuyển từ đơn vị **ô nguyên** sang **ngày thập phân**. Đơn vị: ngày; nhân 80px khi render.

```
rawStart = differenceInCalendarDays(checkIn,  DAYS[0].dateObj) + 0.5
rawEnd   = differenceInCalendarDays(checkOut, DAYS[0].dateObj) + 0.5
```

Kết quả với khách nhận 25, trả 29 trên lưới bắt đầu ngày 23:
`rawStart = 2.5`, `rawEnd = 6.5` → bar bắt đầu giữa ô 25, kết thúc giữa ô 29.
Hai khách nối tiếp cùng phòng chạm nhau ở giữa ô mà không chồng lên nhau.

Quy tắc bổ sung:

- **Bề rộng tối thiểu:** nếu `rawEnd - rawStart < 0.5` (khách nhận và trả cùng ngày,
  hoặc dữ liệu checkout ≤ checkin) thì ép `rawEnd = rawStart + 0.5`. Bar không bao giờ biến mất.
- **Lọc ngoài vùng:** bỏ qua booking khi `rawStart >= 16 || rawEnd <= 0`.
  Phép so sánh này chạy trên giá trị **chưa clamp** — đây chính là chỗ sửa lỗi 2a.
- **Clamp để vẽ:** `visStart = max(0, rawStart)`, `visEnd = min(16, rawEnd)`;
  `left = visStart * 80`, `width = (visEnd - visStart) * 80`.
- **Đánh dấu bị cắt:** `clippedLeft = rawStart < 0`, `clippedRight = rawEnd > 16`.
  Bên bị cắt render góc vuông (`rounded-l-none` / `rounded-r-none`) thay vì bo tròn,
  báo hiệu booking còn kéo dài ngoài màn hình.

Kiểu `BookingBar` đổi từ `{ startCol, length }` sang `{ left, width, clippedLeft, clippedRight }`
(đơn vị px, tính sẵn) để JSX không phải nhân chia.

## 2. Điều hướng thời gian

### 2a. Lọc ngoài vùng

Đã mô tả ở mục 1 — cùng một khối code. Logic hiện tại clamp `startCol` về 0 rồi mới
tính `endCol = Math.max(startCol + 1, ...)`, khiến `endCol` luôn ≥ 1 nên điều kiện
`endCol <= 0` không bao giờ đúng. Mọi booking quá khứ đều vẽ thành bar 1 ngày ở cột 0.

### 2b. Label dải ngày

Bỏ state `currentMonth`, thay bằng hàm thuần tính từ `DAYS`:

```
formatRangeLabel(days):
  first = days[0].dateObj, last = days[days.length - 1].dateObj
  cùng tháng & năm  → "THÁNG 7 NĂM 2026"
  cùng năm          → "THÁNG 7–8 / 2026"
  khác năm          → "12/2026 – 1/2027"
```

### 2c. Nút "Hôm nay"

Hiện khi `dateOffset !== 0`, bấm thì `setDateOffset(0)`.

### 2d. Highlight hôm nay

`getDateRange` tính `isToday` bằng `formatLocalDate(d) === formatLocalDate(startOfLocalDay(new Date()))`
thay vì `i === 3 && offset === 0`. Vạch dọc xanh (`Reservations.tsx:283-285`) ăn theo cùng cờ này
nên tự đúng.

### 2e. Bố cục

Cụm điều hướng `‹ [label] › [Hôm nay]` dời từ ô góc trái rộng 140px lên thanh toolbar,
đặt bên trái ô tìm kiếm. Ô góc chỉ còn chữ "Rooms". Lý do: 140px không đủ chứa label
hai tháng cộng nút "Hôm nay", và toolbar đang có khoảng trống.

## 3. Popup khách đã trả phòng

### Tách component

Popup hiện tại là ~60 dòng JSX nội tuyến trong `Reservations.tsx` (413 dòng).
Tách ra `mhm/src/components/BookingDetailPopup.tsx` với hai chế độ:

```
interface BookingDetailPopupProps {
  booking: BookingWithGuest
  mode: "reservation" | "readonly"
  onClose: () => void
  onConfirm?: (id: string) => void   // chỉ mode reservation
  onEdit?:    (b: BookingWithGuest) => void
  onCancel?:  (id: string) => void
  onViewInvoice?: (id: string) => void  // chỉ mode readonly
}
```

- `mode="reservation"`: giữ nguyên nội dung và ba nút Check-in / Chỉnh sửa / Hủy như hiện tại.
- `mode="readonly"`: tiêu đề "Đã trả — {tên}", các dòng Phòng, Check-in,
  **Trả phòng lúc** (`actual_checkout`, fallback `expected_checkout` nếu null),
  Số đêm, Tổng tiền, Đã thanh toán, SĐT. Hai nút: "Xem hóa đơn" và "Đóng".

`Reservations.tsx` chọn mode theo `selectedBooking.status`: `booked` → `reservation`,
`checked_out` → `readonly`.

### Điều phối click trên bar

```
booked      → setSelectedBooking(bar)   // như cũ
active      → setDrawerRoomId(bar.room_id)  // như cũ
checked_out → setSelectedBooking(bar)   // mới
```

### Xem hóa đơn

Backend đã có hai lệnh (`src-tauri/src/lib.rs:391-392`):

- `get_invoice(bookingId) -> Option<InvoiceData>` — chỉ đọc, không ghi ledger.
- `generate_invoice(bookingId, idempotencyKey, ...)` — lệnh ghi, idempotent.

Thêm `viewInvoice(bookingId)` vào `mhm/src/hooks/useInvoiceDialog.ts`: gọi `get_invoice`
trước; chỉ khi trả về `null` mới fallback sang `generate_invoice`. Xem lại khách cũ
là thao tác đọc, không nên sinh bản ghi ledger mới.

`Reservations.tsx` dùng hook này và render `<InvoiceDialog>` sẵn có.

## 4. Không đổi tab sau check-in/check-out

Bỏ `activeTab: "dashboard"` khỏi hai chỗ trong `mhm/src/stores/useHotelStore.ts`:
`checkIn` (dòng 140) và `checkOut` (dòng 174). Giữ nguyên `dashboardRefreshVersion + 1`
và các lệnh `fetchRooms()` / `fetchStats()` phía trên — dữ liệu vẫn được tải lại.

Điều hướng thuộc về trang gọi, không thuộc store. Cả bốn trang dùng `RoomDrawer`
(Dashboard, Rooms, Housekeeping, Reservations) đều ở nguyên tại chỗ sau khi sửa;
drawer tự đóng qua `handleClose()` như hiện tại.

`RoomDetailPanel.tsx` cũng có `setTab("dashboard")` (dòng 75) nhưng component này
không được import ở bất kỳ đâu — code chết, không đụng tới trong lần sửa này.

## Kiểm thử

Bổ sung vào `mhm/src/pages/Reservations.test.tsx`:

- Booking kết thúc trước ngày đầu lưới không render bar nào (lỗi 2a).
- Booking bắt đầu sau ngày cuối lưới không render bar nào.
- Khách 25→29 trên lưới bắt đầu 23 cho `left = 200px`, `width = 320px`.
- Khách nhận và trả cùng ngày vẫn có bar `width = 40px`.
- Booking vắt ra ngoài mép trái render `left = 0` và có class góc vuông.
- Bấm bar `checked_out` mở popup readonly có dòng "Trả phòng lúc"; popup không có nút Hủy.
- Nút "Xem hóa đơn" gọi `get_invoice`, không gọi `generate_invoice` khi hóa đơn đã tồn tại.
- Label đổi thành dạng hai tháng khi lưới vắt qua ranh giới tháng.
- Nút "Hôm nay" chỉ hiện khi đã chuyển tuần và đưa `dateOffset` về 0.

Bổ sung vào `mhm/src/stores/useHotelStore.test.ts`:

- `checkOut()` không thay đổi `activeTab` (giữ nguyên tab đang mở).
- `checkIn()` không thay đổi `activeTab`.
- Cả hai vẫn tăng `dashboardRefreshVersion`.

File `mhm/src/stores/useHotelStore.test.ts` hiện đang có thay đổi chưa commit trong
working tree — cần kiểm tra và giữ lại thay đổi đó khi sửa.

Component mới `BookingDetailPopup.tsx` có test riêng cho hai mode render đúng
tập nút tương ứng.

## Ngoài phạm vi

- Kéo-thả bar để đổi ngày hoặc đổi phòng.
- Menu chọn tháng/năm trực tiếp (đã cân nhắc, người dùng chọn phương án mũi tên + nút "Hôm nay").
- Dọn dẹp `RoomDetailPanel.tsx` không dùng đến.
- Số cột hiển thị (giữ nguyên 16 ngày).
