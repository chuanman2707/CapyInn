# Mùa cao điểm — khai theo khoảng ngày, và tính đúng từng đêm

- **Ngày**: 2026-07-29
- **Nhánh**: `feature/peak-season`, cắt từ `main` tại `a233ac9`
- **Tiếp nối**: `2026-07-28-dat-phong-truoc-gia-theo-so-khach-design.md`

## Bối cảnh

Lời than gốc của chủ khách sạn là *"ngày đặt là mùa cao điểm với 4 người, nên anh
lấy giá 600k"*. Phiên trước xử nửa **"4 người"**: số khách giờ đẩy giá lên qua
`rooms.extra_person_fee`. Nửa **"mùa cao điểm"** vẫn treo, vì hai lý do độc lập
nhau:

1. **Không có chỗ nào để khai.** Bảng `special_dates` tồn tại từ đầu, backend đã
   có `get_special_dates` và `save_special_date`, nhưng giao diện chưa hề có màn
   hình nào gọi tới. Cả `src/` chỉ có đúng một tham chiếu, và nó nằm trong file
   mock (`src/__mocks__/tauri-core.ts:117`). Cũng **chưa có lệnh xoá**, nên kể cả
   khai được thì khai sai là kẹt luôn.

2. **Khai được cũng ra giá sai.** Engine tra `special_dates` bằng **đúng ngày
   nhận phòng**, rồi nhân mức % ấy cho **toàn bộ** số đêm
   (`pricing_queries.rs:311` → `domain/booking/pricing.rs:137` →
   `pricing.rs:118`). Nó không đi từng đêm như phụ thu cuối tuần vẫn làm
   (`pricing.rs:419`).

Hậu quả của (2), lấy Tết khai 14/02–22/02 +40%:

| Khách ở | Đêm nằm trong Tết | Engine tính | Đúng ra |
|---|---|---|---|
| 12/02 → 16/02 | 2 / 4 đêm | **+0%** — ngày đến 12/02 không phải lễ | +40% cho 2 đêm |
| 22/02 → 25/02 | 1 / 3 đêm | **+40% cả 3 đêm** | +40% cho 1 đêm |

Chiều thứ nhất là mất tiền của nhà, chiều thứ hai là thu lố của khách. Làm màn
hình khai báo mà bỏ qua (2) thì đưa cho chủ nhà một nút bấm ra giá sai.

## Quyết định thiết kế

- **Khai theo khoảng ngày, một mức % cho cả khoảng.** Dưới DB vẫn một dòng một
  ngày, không đổi bảng, không migration.
- **Màn hình gom ngày liền nhau + cùng nhãn + cùng % thành một dòng.** Chủ nhà
  nghĩ theo kỳ nghỉ chứ không theo ngày lẻ. Gom được là suy ra chắc chắn từ ba
  thuộc tính đó, không cần lưu thêm gì.
- **Trùng ngày thì báo rõ rồi hỏi**, chứ không ghi đè im lặng như backend hiện
  giờ đang làm (`pricing_repository.rs:85`, `ON CONFLICT(date) DO UPDATE`).
- **Sửa luật đếm đêm luôn trong đợt này**, vì màn hình khai báo không có giá trị
  nếu luật sai.

### Vì sao tính bằng **mức bình quân** chứ không sửa `pricing.rs`

Cách hiển nhiên là cho `crate::pricing::calculate_price` nhận một danh sách % theo
từng đêm. Nhưng có một cách cho ra **kết quả đúng y hệt** mà không phải chạm vào
`pricing.rs`:

```
% hiệu dụng = (tổng % của từng đêm trong kỳ ở) ÷ số đêm
```

Đây **không phải xấp xỉ**. Vì `special = base × pct`, và `base = giá đêm × số đêm`:

```
base × (Σ pct_d / N) = (base/N) × Σ pct_d = giá đêm × Σ pct_d
```

đúng bằng tổng phụ thu tính riêng từng đêm. Kiểm hai ví dụ:

- 12/02→16/02, Tết 14–22 +40%. Bốn đêm 12,13,14,15 → Σ = 0+0+40+40 = 80, chia 4 =
  **20%**. Phụ thu = 20% × 4 đêm = 40% × 2 đêm. ✓
- 20/02→25/02, Tết +40% các đêm 20,21,22 rồi Hè +25% các đêm 23,24 → Σ = 170,
  chia 5 = **34%**. 5 đêm × 34% = 3 đêm × 40% + 2 đêm × 25%. ✓ Kỳ ở vắt qua hai
  mùa khác giá cũng ra đúng, mà chữ ký hàm không đổi.

Lợi thêm: chỉ **một** lần làm tròn ở bước nhân %, thay vì làm tròn từng đêm rồi
cộng lại.

Lợi về va chạm: nhánh `refactor/pricing-preview-honesty` đang chạy song song ở
phiên khác **không chạm `pricing.rs`** (đã đối chiếu `git diff main...`). Giữ
`pricing.rs` nguyên vẹn thu vùng va chạm còn đúng hai file, và một trong hai
(`domain/booking/pricing.rs`) là của phiên này.

### Vì sao không thêm bảng "khoảng" thật

Sạch hơn về mô hình, nhưng phải thêm migration, thêm đường sinh ngày, thêm chỗ
đồng bộ giữa bảng khoảng và bảng ngày. Gom hiển thị đạt đúng mục tiêu người dùng
với chi phí bằng không dưới tầng dữ liệu. Nếu sau này cần khoảng chồng nhau có
thứ tự ưu tiên thì mới đáng đổi.

## 1. Luật tính giá — đếm từng đêm

### `domain/booking/pricing.rs`

`StayPricingInputs.special_uplift_pct: f64` **bị thay** bằng dữ liệu thô của cả
kỳ ở:

```rust
/// Các ngày đã khai là mùa cao điểm, đọc cho đúng khoảng của kỳ ở này.
/// Ngày nào không khai thì không có mặt trong danh sách.
pub(crate) special_days: Vec<SpecialDay>,

#[derive(Debug, Clone)]
pub(crate) struct SpecialDay {
    pub(crate) date: String, // `YYYY-MM-DD`
    pub(crate) uplift_pct: f64,
}
```

Kiểu này định nghĩa **trong `domain`**, không mượn `SpecialDate` của
`queries::booking::pricing_queries`. Chiều phụ thuộc là queries → domain;
`architecture_guard` giữ chiều ấy.

Hàm thuần mới, đi từng ngày y hệt `calculate_weekend_uplift` (`pricing.rs:419`)
để hai phụ thu không bao giờ bất đồng về "một kỳ ở gồm những ngày nào":

```rust
/// Mức uplift bình quân trên số đêm. Xem phần "Vì sao tính bằng mức bình quân"
/// trong spec: nhân mức này với `base` ra đúng bằng cộng phụ thu từng đêm.
fn effective_special_uplift(inputs: &StayPricingInputs) -> BookingResult<f64>
```

- Lấy ngày của `check_in`, `check_out` bằng chính bộ phân tích mà
  `nights_between` đang dùng (`value.get(..10)`, an toàn với chuỗi nhiều byte).
- `total_days = (co - ci).num_days().max(1)` — **y hệt** `calculate_weekend_uplift`,
  nên kỳ ở trong ngày (theo giờ) đếm đúng ngày nhận phòng.
- Đi `total_days` bước từ `ci`, mỗi bước cộng `uplift_pct` của ngày đó, không có
  thì cộng 0.
- Trả `tổng / total_days`.
- `special_days` rỗng → trả `0.0`, không rẽ nhánh riêng.

`calculate_from_loaded_inputs` (`pricing.rs` dòng 132–138 hiện tại) đổi đối số
cuối từ `inputs.special_uplift_pct` thành `effective_special_uplift(inputs)?`.
Không còn thay đổi nào khác trong hàm.

### `queries/booking/pricing_queries.rs`

`SPECIAL_UPLIFT_SQL` (tra một ngày) **bị thay** bằng:

```rust
const SPECIAL_DAYS_IN_RANGE_SQL: &str =
    "SELECT date, CAST(uplift_pct AS REAL) AS uplift_pct
     FROM special_dates WHERE date >= ? AND date <= ? ORDER BY date";
```

Cận trên lấy **bao gồm** ngày trả phòng. Đọc dư một ngày là cố ý: quyết định
"ngày nào là một đêm" thuộc về `domain`, tầng đọc không được tự cắt.

`load_special_uplift` / `load_special_uplift_tx` thành
`load_special_days(pool, check_in, check_out)` / `load_special_days_tx(...)`,
bind `date_key(check_in)` và `date_key(check_out)`.

Ba chỗ dựng `StayPricingInputs` đổi theo, **giữ nguyên** cách xử lỗi đang có:

| Chỗ dựng | Dòng hiện tại | Lỗi đọc `special_dates` |
|---|---|---|
| `load_stay_pricing_inputs_tx` | 166 | ném lỗi — đường này thu tiền thật |
| `load_stay_pricing_inputs_for_room_type` | 205 | `unwrap_or_default()` — chỉ xem trước |
| `load_stay_pricing_inputs_for_room` | 243 | `unwrap_or_default()` — chỉ xem trước |

> **Ghi chú va chạm.** Nhánh `refactor/pricing-preview-honesty` đang đổi chính
> quy ước này theo chiều ngược lại: bên đó bản xem trước cũng ném lỗi (test
> `a_failed_special_date_read_fails_the_preview_instead_of_quoting_no_uplift`).
> Spec này **cố ý giữ hành vi của `main`** để phần diff còn lại là đổi kiểu dữ
> liệu thuần. Lúc gộp hai nhánh, lấy bản nghiêm của nhánh kia.

### Không đổi

- Đêm vừa là thứ 7 vừa là ngày lễ **ăn cả hai** phụ thu. Đó là hành vi hiện tại,
  đổi nó là quyết định về giá chứ không phải sửa lỗi.
- Nhãn dòng phân tích vẫn là `"Phụ thu ngày lễ"` (`pricing.rs:133`), không kèm
  số ngày. Muốn hiện "2 đêm lễ" thì phải sửa `pricing.rs` — đúng thứ đang tránh.
- Phụ thu thêm khách vẫn nằm **ngoài** `base_amount` (`domain/booking/pricing.rs:99`),
  nên khai Tết không thổi phồng tiền phụ thu khách.
- Kỳ ở theo giờ trong ngày ra kết quả y như trước khi sửa.

## 2. Lệnh backend

### Thêm mới

```rust
save_special_date_range(from: String, to: String, label: String, uplift_pct: f64)
delete_special_dates(dates: Vec<String>)
```

Cả hai `require_admin`, cả hai chạy trong **một** transaction — nửa khoảng nằm
lại trong DB là giá sai âm thầm, tệ hơn là báo lỗi.

`save_special_date` cũ **giữ nguyên chữ ký** (đã đăng ký ở `lib.rs:395`), nhưng
gọi lại đường khoảng với `from = to = date`, để chỉ còn một đường ghi.

Repository nhận cả khoảng, giữ nguyên `ON CONFLICT(date) DO UPDATE` hiện có:

```rust
upsert_special_date_range_tx(tx, ids, from, to, label, uplift_pct, now)
delete_special_dates_tx(tx, dates)
```

`id` sinh mới cho từng ngày; ngày đã tồn tại thì `ON CONFLICT` giữ `id` và
`created_at` cũ, đúng như ghi chú ở `pricing_repository.rs:72`.

### Chặn đầu vào (ở tầng lệnh, trước khi mở transaction)

| Điều kiện | Vì sao |
|---|---|
| `from` và `to` đúng dạng `YYYY-MM-DD` | tránh ghi rác vào cột `date` |
| `to >= from` | gõ ngược thì khoảng rỗng, ghi xong không hiện ra |
| khoảng ≤ 366 ngày | gõ nhầm năm sẽ sinh hàng nghìn dòng |
| `0 <= uplift_pct <= 500` | `uplift_pct` là `REAL` không chặn gì; âm là giảm giá ngầm |
| `label` bỏ khoảng trắng còn khác rỗng | gom cụm dựa vào nhãn; nhãn rỗng gom nhầm hai kỳ khác nhau |
| `dates` không rỗng, mỗi phần tử đúng dạng | xoá rỗng là lệnh vô nghĩa |

Lỗi trả về tiếng Việt, theo lối các lệnh khác trong `commands/pricing.rs`.

### Không thêm

Không cần lệnh đọc mới. Màn hình đã tải toàn bộ qua `get_special_dates`, nên
việc dò trùng ngày làm ngay phía giao diện. Đây là ứng dụng một máy một người
dùng; khoảng hở giữa lúc dò và lúc ghi không có ý nghĩa thực tế.

## 3. Màn hình khai báo

Cài đặt → mục mới **Peak Season** (`CalendarDays`), chỉ hiện với admin, xếp cạnh
**Pricing** trong khối `isCurrentAdmin` ở `src/pages/settings/index.tsx:68`.
Nhãn thanh bên bằng tiếng Anh cho khớp các mục cũ; nội dung bên trong tiếng Việt,
đúng như `PricingSection.tsx` đang làm.

File mới `src/pages/settings/SpecialDatesSection.tsx`. Không nhét vào
`PricingSection.tsx` — file đó đang lo bảng giá theo loại phòng, thêm một CRUD
nữa là nó ôm hai việc.

### Danh sách

```
Tết Nguyên đán    14/02 – 22/02  (9 ngày)   +40%   [Sửa] [Xoá]
Lễ 30/4           30/04 – 03/05  (4 ngày)   +30%   [Sửa] [Xoá]
```

Gom cụm là hàm thuần, tách riêng ra `src/lib/specialDateRanges.ts` để thử được
mà không cần dựng component:

```ts
export type SpecialDateRow = { id: string; date: string; label: string; uplift_pct: number };
export type SpecialDateRange = {
  from: string; to: string; days: number;
  label: string; uplift_pct: number;
  dates: string[]; // mọi ngày trong cụm, dùng khi xoá
};
export function groupSpecialDates(rows: SpecialDateRow[]): SpecialDateRange[];
```

Luật gom: sắp theo `date`, nối ngày kế tiếp vào cụm đang mở khi **cả ba** đều
khớp — liền ngay hôm sau, cùng `label`, cùng `uplift_pct`. Hở một ngày, hoặc
khác nhãn, hoặc khác mức, thì mở cụm mới.

So sánh ngày bằng chuỗi `YYYY-MM-DD` thuần. "Ngày kế tiếp" cần một bước lịch, làm
bằng `Date.UTC(...)` rồi cắt mười ký tự đầu — **tuyệt đối không**
`new Date("2026-02-14T00:00:00")`, kiểu ấy phân tích theo giờ địa phương rồi in
ra UTC, lệch một ngày ở UTC+7. Đó đúng là lỗi phiên trước đã dính.

Hàm bước ngày này để **cục bộ trong `specialDateRanges.ts`**, không tách ra
`src/lib/`. Lý do là va chạm: `ReservationSheet.tsx:35` có `addDays` riêng nhận
và trả `string`, còn nhánh `refactor/pricing-preview-honesty` đang thêm
`src/lib/datetime.ts` với một `addDays` **khác chữ ký** (nhận và trả `Date`).
Dựng thêm một module dùng chung lúc này là chuốc lấy đụng độ trùng tên với kiểu
không tương thích. Gộp ba chỗ này về một chỗ là việc **sau khi hai nhánh đã
gộp**, đã ghi ở phần Ngoài phạm vi.

Trống thì hiện một dòng chỉ đường, không phải bảng rỗng.

### Form

Nhãn / Từ ngày / Đến ngày / % phụ thu. Hai ô ngày đều bấm ra lịch, giống ô ngày
trong `ReservationSheet.tsx`. Nút lưu tắt khi nhãn rỗng hoặc `to < from`.

### Trùng ngày

Trước khi gọi lệnh ghi, dò khoảng đang khai với danh sách đã tải. Có trùng thì
hiện hộp xác nhận liệt kê thẳng, kèm mức cũ và mức mới:

> 3 ngày đã khai sẽ bị ghi đè: 20/02, 21/02, 22/02 — Tết Nguyên đán +40% → Hè
> đầu năm +25%. Tiếp tục?

Quá 10 ngày trùng thì liệt kê 10 ngày đầu rồi "…và N ngày nữa". Bấm huỷ thì
**không gọi lệnh ghi nào**. Khi sửa chính cụm đang mở thì ngày của chính cụm ấy
không tính là trùng.

### Sửa và xoá

- **Sửa**: đổ cụm vào form. Lưu = `delete_special_dates(ngày cũ không còn nằm
  trong khoảng mới)` rồi `save_special_date_range(khoảng mới)`. Rút ngắn khoảng
  thì mấy ngày rơi ra bị xoá thật, không sót lại thành ngày lễ mồ côi.
- **Xoá**: hỏi lại một lần, rồi `delete_special_dates(cụm.dates)` — một lệnh cho
  cả cụm.
- Xong việc thì tải lại bằng `get_special_dates`, không tự sửa state cục bộ, để
  màn hình luôn phản ánh cái đang thật sự nằm trong DB.

Ghi qua `invokeWriteCommand` như `PricingSection.tsx:47`. Báo kết quả bằng
`toast`, theo đúng lối file đó.

## 4. Kiểm chứng

Theo TDD: test đỏ trước, và với hai ca dưới đây phải **thấy** nó đỏ đúng vì lý do
số học, không phải vì thiếu hàm.

### Rust — `domain/booking/pricing.rs`

| Test | Con số |
|---|---|
| kỳ ở bắt đầu **trước** mùa | 12/02→16/02, Tết 14–22 +40%, giá 500k/đêm → phụ thu 400.000₫ (2 đêm × 40%), **không phải 0** |
| kỳ ở kéo **quá** mùa | 22/02→25/02, cùng khai → phụ thu 200.000₫ (1 đêm), **không phải 600.000₫** |
| vắt qua hai mức | 20/02→25/02, +40% ba đêm rồi +25% hai đêm → đúng bằng tổng tính riêng từng đêm |
| nằm trọn trong mùa | kết quả **y hệt** trước khi sửa — chống hồi quy |
| không khai ngày nào | `special_days` rỗng → phụ thu 0 |
| kỳ ở theo giờ | cùng ngày, ngày đó có khai → ra như cũ |
| lễ trùng thứ 7 | vẫn ăn **cả hai** phụ thu — khoá hành vi hiện có |

### Rust — `queries/booking/pricing_queries.rs`

- `load_special_days` chỉ trả ngày trong khoảng, bỏ ngày ngoài hai đầu.
- Ngày trả phòng nằm trong kết quả (cận trên bao gồm).
- Bản theo mã phòng và bản theo loại phòng đọc cùng một khoảng.
- `special_dates` hỏng: đường `_tx` ném lỗi, hai đường xem trước trả rỗng.

### Rust — lệnh và repository

- `save_special_date_range` 14/02→22/02 ghi đúng **9** dòng.
- Ghi đè khoảng có ngày trùng: `label` và `uplift_pct` đổi, `created_at` **giữ
  nguyên**.
- `save_special_date` một ngày vẫn ghi đúng một dòng (chữ ký cũ còn dùng được).
- `delete_special_dates` xoá đúng danh sách, không đụng ngày khác.
- Mỗi luật chặn đầu vào một test: ngày sai dạng, `to < from`, 367 ngày, `-10`,
  `600`, nhãn toàn khoảng trắng, `dates` rỗng.
- Không phải admin thì cả hai lệnh ghi bị từ chối.
- Lỗi giữa chừng khi ghi khoảng → DB không còn dòng nào của khoảng đó.

### Frontend — `src/lib/specialDateRanges.test.ts`

- 9 dòng liền nhau cùng nhãn cùng mức → **1** cụm, `days = 9`, `dates` đủ 9.
- Hở một ngày → **2** cụm.
- Liền ngày nhưng khác nhãn → **2** cụm.
- Liền ngày, cùng nhãn, khác mức → **2** cụm.
- Đầu vào rỗng → mảng rỗng.
- Đầu vào không theo thứ tự ngày → vẫn gom đúng.
- Vắt qua ranh giới tháng và ranh giới năm (28/02→01/03, 31/12→01/01).

### Frontend — `SpecialDatesSection.test.tsx`

- Danh sách hiện **một** dòng cho chín ngày Tết.
- Khai khoảng không trùng → gọi `save_special_date_range` đúng một lần với đúng
  đối số.
- Khai khoảng có trùng → hiện đúng danh sách ngày trùng; bấm **huỷ** thì
  `invokeWriteCommand` **không** được gọi.
- Bấm tiếp tục → mới gọi lệnh ghi.
- Xoá cụm → `delete_special_dates` nhận đúng 9 ngày trong một lần gọi.
- Sửa cụm cho ngắn lại → những ngày rơi ra có mặt trong lệnh xoá.
- Lệnh lỗi → hiện toast lỗi, danh sách không bị đổi ngầm.

### Cổng chung

`npm run verify:full`, `cargo check --all-targets`, và **`cargo fmt --check`** —
cái cuối CI có gác mà `verify:full` không chạy, phiên trước đã dính đỏ vì nó.

## Ngoài phạm vi

- **Hiện số đêm lễ trong dòng phân tích.** Muốn "Phụ thu 2 đêm lễ (Tết)" thì
  phải sửa `pricing.rs` — đúng file đang cố giữ nguyên để khỏi đụng nhánh kia.
  Làm sau khi hai nhánh đã gộp.
- **Khoảng chồng nhau có thứ tự ưu tiên.** `UNIQUE(date)` chỉ cho một mức mỗi
  ngày. Hộp xác nhận ghi đè là câu trả lời cho vòng này.
- **Mức phụ thu khác nhau theo loại phòng.** `special_dates` không có cột loại
  phòng. Đổi được nhưng là quyết định về giá, không phải sửa lỗi.
- **Nhãn hoá đơn còn tiếng Anh** (`invoice_generation.rs:120`). Tiền đúng, chỉ
  chữ sai. Đã ghi ở spec trước, vẫn treo.
- **Gộp ba bộ trợ giúp ngày về một chỗ.** `ReservationSheet.tsx:35`, module gom
  cụm của spec này, và `src/lib/datetime.ts` của nhánh kia — mỗi nơi một hàm
  bước ngày, chữ ký khác nhau. Gộp trước khi hai nhánh gộp là tự tạo đụng độ.
- **Số khách cho nhận phòng vãng lai và nhận phòng đoàn.** Là việc của phiên
  kia, cố ý không đụng.
- **Ngày "hôm nay" tính theo UTC**, sai trong khoảng 00:00–07:00 giờ Việt Nam.
  Không thuộc màn hình này.

## Rủi ro

- **Va chạm với `refactor/pricing-preview-honesty`.** Hai nhánh cùng sửa
  `pricing_queries.rs` và `pricing_service.rs`. Đã thu hẹp bằng cách không đụng
  `pricing.rs`, nhưng lúc gộp vẫn phải gỡ tay ba chỗ dựng `StayPricingInputs`.
  Chốt sẵn: giữ **kiểu dữ liệu của nhánh này**, giữ **cách xử lỗi của nhánh kia**.
- **Đổi luật là đổi giá cho đơn đã đặt.** Đơn đã lưu giữ `total_price` cũ, không
  bị tính lại. Nhưng đặt trước rồi **sửa ngày** hay **gia hạn** thì đi lại engine
  và ra số mới — đúng hơn số cũ, mà vẫn là một thay đổi chủ nhà cần biết.
- **Gom cụm là suy đoán, không phải dữ liệu.** Hai kỳ khác nhau tình cờ liền
  ngày, cùng nhãn, cùng mức sẽ hiện thành một dòng. Vô hại — xoá cụm ấy đúng là
  xoá chừng đó ngày — nhưng phải nói rõ để sau này không ai coi đó là lỗi.
- **Không có ràng buộc `CHECK` dưới DB.** Mọi chặn đầu vào nằm ở tầng lệnh. Ghi
  thẳng vào SQLite bằng tay vẫn lọt. Chấp nhận: mọi bảng khác trong dự án cũng
  thế.
