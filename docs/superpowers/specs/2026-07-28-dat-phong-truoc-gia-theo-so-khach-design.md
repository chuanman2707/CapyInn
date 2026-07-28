# Đặt phòng trước — Ngày đi bấm được lịch, và giá theo số khách

Ngày: 2026-07-28
Phạm vi: `mhm/src/components/ReservationSheet.tsx`, `mhm/src-tauri/src/domain/booking/pricing.rs`,
`mhm/src-tauri/src/queries/booking/pricing_queries.rs`, `mhm/src-tauri/src/services/booking/{reservation_lifecycle,stay_lifecycle,pricing_service}.rs`,
`mhm/src-tauri/src/db/migrations.rs`, `mhm/src-tauri/src/models.rs`, `mhm/src-tauri/src/commands/pricing.rs`.

## Bối cảnh

Người dùng báo hai triệu chứng ở sheet "Đặt phòng trước". Khảo sát cho thấy triệu chứng thứ hai
che một lỗi tiền tệ rộng hơn nhiều so với vẻ ngoài của nó.

| # | Triệu chứng | Nguyên nhân | Vị trí |
|---|---|---|---|
| 1 | Ô "Ngày đi" bấm không ra lịch | Ô để `readOnly`, giá trị suy ra từ Ngày đến + Số đêm | `ReservationSheet.tsx:219-224` |
| 2 | Không có cách nào đổi tiền phòng | Form không có ô giá; `CreateReservationRequest` không nhận giá | `models.rs:490-501` |
| 2a | Số trên form có thể lệch số lưu vào sổ | Form tự nhân `base_price × số đêm`; backend lại hỏi engine (có cộng % cuối tuần / ngày lễ) | `ReservationSheet.tsx:359` |
| 2b | Phụ thu thêm người khai báo rồi vẫn không có tác dụng | Settings lưu `rooms.extra_person_fee` xuống DB, nhưng engine tính giá không bao giờ đọc tới | `RoomFormDialog.tsx:116` ↔ `domain/booking/pricing.rs` |

Bối cảnh thực tế của người dùng: phòng 2A niêm yết 500.000₫ là giá cho 2 người. Khách đặt 4 người,
phụ thu 50.000₫/người → 600.000₫/đêm. Đây là **luật giá thường trực của khách sạn**, không phải
tình huống cá biệt.

## Quyết định thiết kế

Sáu quyết định đã chốt với người dùng:

1. **Bỏ hẳn ô "Số đêm".** Chỉ còn Ngày đến + Ngày đi, cả hai bấm ra lịch. Số đêm suy ra từ hai ngày.
2. **Giá đến từ số khách, không phải từ ô gõ tay.** Người dùng đã cân nhắc phương án gõ giá tay
   và loại bỏ nó.
3. **Engine vẫn là nơi duy nhất định giá.** Không có cột giá đè, không có nhánh "bỏ qua engine".
4. **Phụ thu là khoản tiền phẳng theo đêm**, không bị nhân thêm % cuối tuần hay ngày lễ.
5. **Form gọi engine để xem trước giá**, bỏ phép nhân phía giao diện.
6. **Số khách không có trần.** Ô trong Settings tên là "số khách tính giá base" — là mốc tính tiền,
   không phải sức chứa.

### Vì sao không chọn phương án gõ giá tay

Phương án đó cần thêm một **cột tiền** mới, kéo theo việc đăng ký vào ba danh sách canh gác tiền tệ
(`MONEY_COLUMNS` và `MONEY_TABLES` trong `money_migration.rs`, cộng danh sách tên cột viết cứng trong
`scripts/verify/no-float-money.mjs`). Quên một trong ba thì không có gì báo lỗi.

Nặng hơn: nó đưa vào hệ thống một **nguồn định giá thứ hai** cạnh engine, buộc cả năm chỗ tính tiền
phải rẽ nhánh "có giá đè không?". Hướng số khách chỉ cần một cột **số đếm** — nằm ngoài mọi danh sách
canh gác tiền — và cả năm chỗ chỉ truyền thêm một tham số, không chỗ nào rẽ nhánh.

## 1. Form đặt phòng

`ReservationSheet.tsx`. Ô "Số khách" chiếm đúng chỗ ô "Số đêm" cũ.

| Ô | Thay đổi |
|---|---|
| Phòng | giữ nguyên |
| Ngày đến \| Ngày đi | bỏ `readOnly` ở Ngày đi; cả hai là `<input type="date">` bấm ra lịch |
| ~~Số đêm~~ | bỏ hẳn |
| **Số khách** | ô mới, `type="number"`, `min={1}`, không có `max` |
| Thông tin khách, Nguồn, Tiền cọc, Ghi chú | giữ nguyên |

**Nạp giá trị mặc định.** Chọn phòng → ô Số khách nạp `room.max_guests`. Đổi sang phòng khác →
nạp lại theo phòng mới, kể cả khi người dùng đã sửa tay, vì mốc tính giá base gắn với từng phòng.

**Ràng buộc ngày.** Trước đây Ngày đi luôn hợp lệ vì máy tự tính; giờ gõ tay được nên phải chặn:

- `min` của ô Ngày đi = Ngày đến + 1 ngày.
- Ngày đi ≤ Ngày đến → hiện lỗi và khoá nút "Đặt phòng".
- Số đêm = `differenceInCalendarDays(checkOut, checkIn)`, gửi xuống backend như cũ.

**Chế độ sửa** (`editBooking`) dùng đúng bộ ô này; ô Số khách nạp từ `booking.guests`.

## 2. Cột `bookings.guests`

Thêm qua `execute_compat_alter` trong `db/migrations.rs`, cùng lối với các cột đã có
(`pricing_snapshot` ở dòng 224, `pricing_type` ở dòng 231):

```sql
ALTER TABLE bookings ADD COLUMN guests INTEGER
```

Cột **cho phép rỗng**. Booking cũ để rỗng, được hiểu là *không phụ thu* — giá của chúng không đổi
một đồng nào sau khi nâng cấp.

Đây là **số đếm, không phải tiền**. Không đăng ký vào `MONEY_COLUMNS`, `MONEY_TABLES`, hay danh sách
trong `no-float-money.mjs`. Tên cột cũng cố ý không chứa từ khoá tiền tệ nào để không lọt vào lưới quét.

`CreateReservationRequest` (`models.rs:490`) nhận thêm `guests: Option<i32>`. Rỗng ⇒ không phụ thu,
giá đúng bằng kết quả hôm nay. Form luôn gửi giá trị; kiểu `Option` là để các nơi gọi khác
(gateway, agent) không phải sửa theo.

## 3. Engine: một dòng phụ thu

`StayPricingInputs` (`domain/booking/pricing.rs:14`) là khớp nối có sẵn cho việc này — nó chính là
"mọi thứ engine cần, đã đọc xong khỏi DB". Thêm ba trường:

```rust
pub(crate) guests: Option<i32>,
pub(crate) base_guests: i32,          // rooms.max_guests
pub(crate) extra_person_fee: MoneyVnd, // rooms.extra_person_fee
```

`pricing_queries.rs` đọc thêm `max_guests` và `extra_person_fee` khi nạp inputs. Đường theo `room_id`
(`load_stay_pricing_inputs_tx`) đọc thẳng từ dòng phòng. Đường theo loại phòng
(`FALLBACK_BASE_PRICE_SQL`, dòng 25) mở rộng cùng câu lệnh đang có.

Công thức, cộng vào sau khi engine đã tính xong phần % cuối tuần và ngày lễ:

```
khách_thêm = max(0, guests - base_guests)
phụ_thu    = khách_thêm × extra_person_fee × số_đêm
```

**Đặt phép cộng ở đâu.** Trong `calculate_from_loaded_inputs` (`domain/booking/pricing.rs:77`),
*sau* khi `crate::pricing::calculate_price` đã trả kết quả: đẩy thêm một dòng nhãn
`"Phụ thu N khách"` vào `PricingResult.breakdown` rồi cộng vào `total`. Module `crate::pricing`
giữ nguyên, không đổi một dòng — nó vẫn chỉ biết về giá phòng và các loại %, còn luật thêm người
nằm ở tầng domain cùng chỗ với các đầu vào của nó.

Hệ quả: phụ thu **không** nằm trong `base_amount`, nên không bị `weekend_uplift_pct` hay
`special_dates.uplift_pct` nhân lên.

Lý do: thêm người là khoản phẳng. Cho % chồng lên nó vừa khó giải thích với khách, vừa khó soát
khi cầm hoá đơn dò ngược. `guests` rỗng hoặc `extra_person_fee = 0` → phụ thu bằng 0, kết quả
giống hệt hôm nay.

Phép nhân dùng `checked_mul_money` như phần còn lại của module, để số khách nhập bậy không tràn số.

## 4. Năm chỗ tính tiền

Cả năm đều gọi `calculate_stay_price_tx`. Chúng nhận thêm một tham số `guests` và truyền thẳng vào
engine — **không chỗ nào rẽ nhánh**, vì không có gì để rẽ.

| Chỗ | Vị trí | Lấy `guests` từ đâu |
|---|---|---|
| Tạo đặt phòng | `reservation_lifecycle.rs:240` | `req.guests` |
| Khách nhận phòng | `reservation_lifecycle.rs:609` | `load_booked_reservation` (`:1043`) đọc thêm cột |
| Sửa đặt phòng | `reservation_lifecycle.rs:758` | như trên |
| Gia hạn ở thêm | `stay_lifecycle.rs:1074` | dòng booking đang đọc sẵn |
| Trả phòng sớm | `stay_lifecycle.rs:680` | dòng booking đang đọc sẵn |

Hai chỗ cuối nằm ngoài màn đặt phòng nhưng nằm trong phạm vi có chủ ý: bỏ chúng lại thì khách đặt
600.000₫/đêm sẽ bị tính 500.000₫ cho đêm gia hạn, và bị tính thiếu khi trả phòng sớm hơn dự kiến.

`ModifyReservationRequest` (`models.rs:504`) nhận thêm `new_guests: Option<i32>`; rỗng nghĩa là
giữ nguyên số khách đang lưu, không phải xoá về không.

## 5. Xem trước giá — bỏ phép nhân phía giao diện

Form đang tự tính `base_price × nights` (`ReservationSheet.tsx:359`). Đó là nguồn gốc của việc số
trên màn hình lệch số trong sổ, và nếu để nguyên thì thêm phụ thu sẽ làm khoảng lệch rộng ra.

Bỏ phép nhân đó. Form gọi lệnh `calculate_price_preview` đã có (`commands/pricing.rs:109`), theo đúng
lối `useAvailability` đang dùng (gọi có trễ, có cờ đang tải). Hiển thị breakdown do engine trả về:

```
Giá phòng × 2 đêm                 1.000.000₫
Phụ thu 2 khách × 50.000 × 2 đêm    200.000₫
                                  1.200.000₫
```

**Lệch giữa hai đường tra cứu.** Phụ thu là thuộc tính của **từng phòng**, còn `calculate_price_preview`
tra theo **loại phòng**. Hai phòng cùng loại đặt phí thêm người khác nhau thì số xem trước sẽ sai.
Xử lý: thêm đường nhận thẳng `room_id` cho `calculate_price_preview`, dùng khi form đã chọn phòng.
Giữ nguyên đường theo loại phòng cho các nơi gọi khác (`gateway/tools.rs:1237`).

## 6. Kiểm chứng

Chạy `npm run verify:full`. Test viết thêm:

**Tiền — mỗi chỗ trong bảng ở mục 4 một test.** Phòng base 500.000₫/2 người, phụ thu 50.000₫:

- Đặt 4 khách, 2 đêm → tổng 1.200.000₫.
- Đặt xong dời ngày → vẫn 600.000₫/đêm.
- Đặt xong khách nhận phòng → vẫn 600.000₫/đêm.
- Gia hạn thêm 1 đêm → đêm thêm tính 600.000₫.
- Trả phòng sớm 1 đêm → 600.000₫ × số đêm ở thực tế.

**Biên:**

- 2 khách → phụ thu bằng 0, tổng đúng bằng kết quả hôm nay.
- `extra_person_fee = 0` → giá không đổi dù nhập bao nhiêu khách.
- Booking cũ, cột `guests` rỗng → giá không đổi.
- Phụ thu không bị `weekend_uplift_pct` nhân lên: cùng một booking, đặt vào cuối tuần và ngày thường,
  phần chênh lệch chỉ nằm ở `base_amount`.

**Form:**

- Ô Ngày đi bấm ra được lịch (không còn `readOnly`).
- Ngày đi ≤ Ngày đến → nút "Đặt phòng" khoá.
- Đổi phòng → ô Số khách nạp lại theo `max_guests` của phòng mới.
- Dòng tổng tiền lấy từ `calculate_price_preview`, không phải phép nhân phía giao diện.

## Ngoài phạm vi

Ghi lại để làm đợt sau, không làm trong đợt này:

1. **Màn hình khai báo mùa cao điểm.** Bảng `special_dates` đã có trong DB và engine đã đọc
   `uplift_pct`, nhưng không có màn hình nào để nhập. Ví dụ ban đầu của người dùng ("mùa cao điểm
   với 4 người") có hai yếu tố; spec này phủ yếu tố số khách, còn yếu tố mùa cao điểm cần màn hình đó.
2. **Số khách cho khách vãng lai.** `check_in_tx` (`stay_lifecycle.rs:258`) tạo booking mới không qua
   đặt phòng trước, nên chưa có ô số khách. Cùng một cột `guests`, chỉ thiếu ô nhập.

## Rủi ro

| Rủi ro | Mức | Xử lý |
|---|---|---|
| Số xem trước lệch số thật khi hai phòng cùng loại khác phí thêm người | Trung bình | Đường xem trước nhận `room_id`, mục 5 |
| Sót một trong năm chỗ tính tiền → tụt giá âm thầm | Trung bình | Mỗi chỗ một test tiền, mục 6 |
| Người dùng nhập số khách âm hoặc rất lớn | Thấp | `min={1}` ở form, kẹp `max(0, …)` và `checked_mul_money` ở engine |
| Booking cũ đổi giá sau nâng cấp | Thấp | Cột cho phép rỗng, rỗng ⇒ không phụ thu; có test |
