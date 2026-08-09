# Rà cuối vòng 2 trước merge — báo cáo

Nhánh: `feat/void-booking-and-manual-rate`
Worktree: `/Users/binhan/HotelManager/.worktrees/void-manual-rate`
Commit gốc: `9d652da` (cây sạch)

Không đụng tới 8 đường đọc SQL + guard tiền đoàn mà reviewer opus đã xác nhận sạch (9/9 mutant đỏ đúng) — chỉ đổi tên/mở rộng docstring của test canh chúng (M1), không đổi logic.

## Xử lý từng mục

### I1 — Escape đi vòng qua guard `busy`
`mhm/src/components/VoidBookingDialog.tsx`, handler Escape trong `useEffect`.

- Thêm `if (busy) return;` ngay sau `event.stopPropagation()`, thêm `busy` vào dependency array của `useEffect`.
- TDD: viết test `"Escape KHÔNG đóng hộp thoại trong lúc đang xóa (busy)..."` trước — xác nhận RED (`onClose` bị gọi dù đang busy), sau đó áp fix, xác nhận GREEN. Toàn bộ 19 test trong file đều xanh.

### I2 — Comment nói sai sự thật
Cùng file, nhánh `loadError`. Comment cũ khẳng định "component không tự bắt Escape ở đây". Đã viết lại đúng sự thật: `useEffect` Escape đăng ký vô điều kiện, nhánh `loadError` vẫn đóng được bằng Escape; nút "Đóng" chỉ là lối thoát thứ hai, không phải duy nhất.

### M1 — Tên test hứa "every" mà không có cơ chế
Chọn **phương án (a)**: đổi tên, không xây guard quét SQL (b).

Lý do: (b) đòi hỏi hạ tầng phân tích tĩnh quét `FROM bookings`/`JOIN bookings` trong `queries/` — chi phí kỹ thuật và rủi ro maintenance cao hơn nhiều (regex trên SQL đa dòng, alias, CTE, sub-select, chuỗi literal chứa "FROM bookings"... dễ false-positive chặn CI hoặc false-negative để lọt đúng loại bug đang sửa). Đây là lượt rà cuối trước merge, không phải lúc đầu tư hạ tầng mới. Tiền lệ trong chính file này (`voided_booking_disappears_from_the_recognized_revenue_totals`, từng đổi tên từ `...every_revenue_path` ở vòng review trước) cho thấy đổi tên + docstring liệt kê tường minh là cách đã được chấp nhận cho đúng lớp lỗi này.

Thực hiện:
- Đổi tên `voided_booking_disappears_from_every_booking_read_path` →
  `voided_booking_disappears_from_the_eight_audited_booking_read_paths`
  (`mhm/src-tauri/src/services/booking/tests/reporting.rs`).
- Viết lại docstring: nói rõ đây là PHẠM VI THẬT (tám đường đã kiểm chứng bằng test), không phải toàn bộ SQL từng đọc `bookings` trong repo; ai thêm đường đọc mới phải thêm mục vào danh sách. Xoá sạch chữ "every" khỏi tên hàm và docstring liên quan (còn một chỗ nhắc tên CŨ đã đổi ở test khác, `...every_revenue_path`, giữ nguyên vì đó là ghi chép lịch sử của MỘT test khác, không phải lời hứa của test này).
- Bổ sung mục còn thiếu: `guest_queries::search_guest_summaries_by_phone` dùng chung hằng SQL `GUEST_SUMMARY_SELECT` với `load_guest_summaries` (mục 3) nên đã được vá theo, chỉ thiếu tên trong danh sách — đã thêm ghi chú.
- `cargo test voided_booking_disappears_from_the_eight_audited_booking_read_paths` → pass. Không sửa logic nên không cần mutation lại (đã được xác nhận sạch ở vòng review trước).

### M2 — `GuestProfileSheet` không cạn kiệt dù comment nói có
`mhm/src/components/GuestProfileSheet.tsx`.

- Đổi `interface BookingWithRoom.status` từ `string` → `BookingStatus`.
- Đổi tham số `guestBookingStatusLabel(status: string)` → `(status: BookingStatus)`.
- **Lệch với chỉ dẫn gốc ("sửa một chữ")**: đo thực tế bằng cách thêm tạm một status giả vào union `BookingStatus`, chạy `npx tsc --noEmit` — chỉ đổi kiểu tham số KHÔNG làm tsc đỏ ở đây (switch có `default: return status` vẫn hợp lệ vì `BookingStatus` là subtype của `string`, không có gì ép cạn kiệt). Phải thêm `assertUnreachableStatus(status: never)` giống hệt cơ chế đã có ở `Reservations.tsx` thì tsc mới đỏ đúng ở CẢ HAI file khi thêm status giả. Đã áp cả hai thay đổi, đo lại xác nhận: `GuestProfileSheet.tsx(52,44)` + `Reservations.tsx(137,44)` + `(156,44)` đều đỏ. Sau đó hoàn nguyên status giả, `tsc --noEmit` sạch.
- TDD: export `guestBookingStatusLabel` (named export, giữ nguyên default export), viết test gọi hàm với status ép kiểu `as never` mô phỏng "status lạ" — xác nhận RED trên code cũ (trả về chuỗi thô, không ném lỗi), áp fix, xác nhận GREEN (ném lỗi thay vì lộ chuỗi DB thô).

### M3 — Chữ "engine" lọt vào chuỗi tiếng Việt
`mhm/src/components/shared/RateOverrideField.tsx:94-96`: đổi "khác giá engine" → "khác giá hệ thống tính".

- TDD: test `"chuỗi cảnh báo không lẫn từ kỹ thuật 'engine'"` — RED trước, GREEN sau.
- Grep toàn bộ `.tsx` (không phải `.test.tsx`) tìm `engine|override|folio|payload|snapshot`: mọi kết quả còn lại đều là tên biến/prop/type hoặc comment code (`rateOverride`, `engineTotal` prop, `CheckoutSettlementPayload` type, `log.folio_revenue` field access, "WebKit... engine" trong comment về trình duyệt) — không có chuỗi JSX hiển thị cho người dùng nào khác lẫn từ kỹ thuật.

### M5 — Hai câu tài liệu sai
Đọc code xác minh trước khi sửa:
- `db/core_extensions.rs:189`: `rate_overridden_at` là cột `TEXT` thêm ở migration v26, không có giá trị mặc định.
- `reservation_lifecycle.rs` (nhánh có override): gán `Some(now.clone())` — `now` là timestamp RFC3339, không phải số tiền. Giá thật nằm ở `pricing_snapshot.manual_rate.rate_per_night` (field `"rate_per_night": rate` trong JSON snapshot).
- `hooks/useRoomPrices.ts:154`, `sumRoomPricesWithOverrides`: `total += override * nights` — đúng phép nhân giống `RateOverrideField`, dùng cho màn group check-in.

Sửa:
- `mhm/src/CLAUDE.md`: đổi "The unit is per night, matching `rate_overridden_at`" → nói rõ đơn vị khớp `pricing_snapshot.manual_rate.rate_per_night`, và `rate_overridden_at` chỉ là cờ timestamp RFC3339, không mang giá/đơn vị. Đổi "documented exception" số ít → liệt kê CẢ `RateOverrideField` LẪN `sumRoomPricesWithOverrides` là hai ngoại lệ.
- `mhm/src-tauri/CLAUDE.md`: sửa câu tương tự — giá tay nằm ở `pricing_snapshot.manual_rate.rate_per_night`, `rate_overridden_at` chỉ là cờ.

Đây là sửa tài liệu thuần, không đổi hành vi — không cần test.

### M6 — Badge "Tổng" dùng danh sách đen
`mhm/src/pages/Reservations.tsx`.

Quyết định: **"Tổng" không nên đếm `cancelled`.** "Tổng" đứng cạnh ba badge Đang ở/Đặt trước/Đã trả — tất cả mô tả các lượt còn đang hiện diện trên lịch (có bar). `cancelled`, giống `voided`, không có bar nào trên lịch (không nằm trong `VISIBLE_BOOKING_STATUSES`) nên không nên góp vào "Tổng" của cùng khối UI đó. Dùng lại đúng một danh sách trắng (`VISIBLE_BOOKING_STATUSES`, đã có sẵn làm nguồn chân lý cho việc vẽ bar) cho cả hai chỗ, thay vì hai danh sách (một đen, một trắng) dễ lệch nhau như đã xảy ra trong chính commit này.

- Đổi `totalCount = visibleBookings.filter(b => b.status !== "voided").length` → `visibleBookings.filter(b => VISIBLE_BOOKING_STATUSES.includes(b.status)).length`.
- TDD: test `"không đếm lượt đã hủy (cancelled) vào tổng số booking hiển thị..."` — RED trước (đếm ra 2 thay vì 1), GREEN sau.

### Không sửa
M4 (cảnh báo vàng bật mọi lần giảm giá) — theo đúng chỉ dẫn, đây là khẩu vị chứ không phải bug.

## Kết quả cổng

Từ `mhm/`:
- `npm test` → **99 file, 962 test, tất cả pass.**
- `npx tsc --noEmit` → sạch, không lỗi.
- `npm run build` → build thành công (`tsc && vite build`, chỉ có cảnh báo chunk-size vốn có từ trước, không phải lỗi).
- `npm run verify:money` → "No Rust PMS money f64 contracts found."

Từ `mhm/src-tauri/`:
- `cargo test` → **1415 passed; 0 failed; 0 ignored.**
- `cargo clippy --all-targets -- -D warnings` → sạch, không cảnh báo.
- `cargo fmt -- --check` → sạch, không lệch định dạng.

## Kiểm răng (mutation)

Mỗi fix có hành vi thay đổi (I1, M2, M3, M6) đều đi qua đúng chu trình RED → GREEN theo TDD:
viết test trước, chạy xác nhận đỏ đúng lý do (hành vi cũ), áp fix, chạy xác nhận xanh — bản thân
chu trình này chính là phép kiểm răng cho từng test mới. M1 chỉ đổi tên/docstring, không đổi
logic được test canh (logic 8 đường đọc đã được reviewer trước mutation 9/9 đỏ đúng, không đụng
lại). M5 là tài liệu thuần, không có test để kiểm răng.

## Commit

Xem SHA trong output `git log` sau khi commit — commit message tiếng Việt, mô tả đủ 6 mục đã sửa.
