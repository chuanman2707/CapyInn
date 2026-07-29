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
   (`pricing.rs:427`).

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
  giờ đang làm (`repositories/booking/pricing_repository.rs:85`, `ON CONFLICT(date) DO UPDATE`).
- **Sửa luật đếm đêm luôn trong đợt này**, vì màn hình khai báo không có giá trị
  nếu luật sai.

### Vì sao tính bằng **mức bình quân** chứ không sửa `pricing.rs`

Cách hiển nhiên là cho `crate::pricing::calculate_price` nhận một danh sách % theo
từng đêm. Nhưng có một cách rẻ hơn nhiều mà không phải chạm vào `pricing.rs`:

```
% hiệu dụng = (tổng % của từng ngày trong kỳ ở) ÷ N        (N = số ngày kỳ ở)
```

Điều luôn đúng, với **mọi** kiểu tính giá:

```
base × (Σ pct_d / N) = Σ ( (base/N) × pct_d )
```

Nghĩa là: chia `base` thành N phần bằng nhau, mỗi ngày chịu mức % của riêng nó
trên phần của mình. Đó là định nghĩa của "chia đều theo ngày", và nó là hệ quả
đại số chứ không phải xấp xỉ.

Điều **chỉ đúng với `nightly`, `overnight`, `daily`**: `base/N` đúng bằng giá một
đêm đã cấu hình. Ba nhánh này tính `base = giá đêm × số ngày`, đếm ngày **tương
đương** với `calculate_weekend_uplift` (`pricing.rs:115-116`, `:251-260`,
`:345-346`, đối chiếu `:440`). Nói *tương đương* chứ không phải *cùng một biểu
thức*: `calculate_overnight` viết `if days == 0 { 1 } else { days }` thay vì
`.max(1)`. Hai cách cho cùng kết quả ở đây, vì `calculate_price` đã thoát sớm khi
`co <= ci` (`pricing.rs:86-96`) nên `days` không bao giờ âm. Nên với ba nhánh
này, "chia đều theo ngày" **chính là** "từng đêm chịu mức của nó":

- 12/02→16/02, Tết 14–22 +40%. Bốn đêm 12,13,14,15 → Σ = 0+0+40+40 = 80, chia 4 =
  **20%**. Phụ thu = 20% × 4 đêm = 40% × 2 đêm. ✓
- 20/02→25/02, Tết +40% các đêm 20,21,22 rồi Hè +25% các đêm 23,24 → Σ = 170,
  chia 5 = **34%**. 5 đêm × 34% = 3 đêm × 40% + 2 đêm × 25%. ✓ Kỳ ở vắt qua hai
  mùa khác giá cũng ra đúng, mà chữ ký hàm không đổi.

### `hourly` là ngoại lệ, và nó đổi hành vi

`calculate_hourly` (`pricing.rs:155-174`) tính `base = giá giờ × số giờ`, hoặc
một mức trần `overnight_rate`, hoặc `daily_rate × ceil(số giờ / 24)`. **Không**
cái nào tỉ lệ với `total_days` của `calculate_weekend_uplift`. Nên với kiểu giờ,
`base/N` không phải giá một đêm, và "chia đều theo ngày" **không** trùng với
"từng đêm chịu mức của nó".

Kỳ ở theo giờ **trong cùng một ngày** thì `N = 1`, mức hiệu dụng bằng đúng mức
của ngày đó, kết quả **y hệt** hôm nay. Kỳ ở theo giờ **vắt qua từ hai ngày trở
lên** thì đổi số. Ví dụ giá giờ 20k / qua đêm 300k / ngày 400k, ở
`2026-02-13T20:00 → 2026-02-15T02:00` (30 giờ), Tết 14–22 +40%:

| | Kết quả |
|---|---|
| `base` | 600.000₫ (`raw_hourly`, không chạm trần) |
| Hôm nay | phụ thu **0₫** — ngày đến 13/02 chưa khai |
| Sau khi sửa | `(0+40)/2 = 20%` → phụ thu **120.000₫** |

Đây là **chủ ý**, không phải tác dụng phụ: một kỳ ở đè lên ngày lễ thì phải chịu
phụ thu, và chia đều theo ngày là cách phân bổ duy nhất có nghĩa khi `base` không
tính theo đêm. Nó được chốt bằng một test riêng, không để trôi vào phần "không
đổi".

Và diện ảnh hưởng nhỏ hơn nghe tưởng. Muốn `N ≥ 2` thì kỳ ở phải **dài quá 24
giờ**. Mà nhánh chặn trần về `overnight_rate` chỉ chạy khi `duration_hours <= 13`
(`pricing.rs:161`), nên nó không bao giờ chạm tới `N ≥ 2`. Nói cách khác: đổi số
**chỉ xảy ra với kỳ ở tính theo giờ mà dài hơn một ngày** — thứ vốn đã hiếm, và
vốn đã là ca mà tính theo giờ cho ra giá kỳ quặc.

### Làm tròn

Chỉ **một** lần làm tròn, ở bước nhân %, thay vì làm tròn từng đêm rồi cộng lại.
Với giá đêm chẵn thì hai cách ra cùng số. Với giá lẻ thì lệch vài đồng — 333.333₫
một đêm, ba đêm +40%: gộp ra 400.000₫, cộng từng đêm ra 399.999₫. Bản gộp là bản
đúng theo hợp đồng; test phải ghi **số tuyệt đối**, không được viết kiểu "tính
từng đêm rồi cộng lại rồi so bằng".

### Lợi thêm: vùng tranh chấp co lại

Nhánh `refactor/pricing-preview-honesty` đang chạy song song ở
phiên khác **không chạm `pricing.rs`** (đã đối chiếu `git diff --stat main...`,
21 file, không có file này). Giữ `pricing.rs` nguyên vẹn thu vùng tranh chấp ở
tầng engine còn đúng **một** file production: `queries/booking/pricing_queries.rs`.
Chi tiết ở phần Rủi ro.

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

Hàm thuần mới, đi từng ngày y hệt `calculate_weekend_uplift` (`pricing.rs:427`)
để hai phụ thu không bao giờ bất đồng về "một kỳ ở gồm những ngày nào":

```rust
/// Mức uplift bình quân trên số ngày của kỳ ở.
///
/// Nhân mức này với `base` là chia đều `base` cho từng ngày rồi cho mỗi ngày
/// chịu mức của riêng nó. Với `nightly`/`overnight`/`daily`, phần chia đều ấy
/// đúng bằng giá một đêm, nên kết quả là "từng đêm chịu mức của nó". Với
/// `hourly` thì `base` không tính theo đêm, và đây là phép phân bổ theo ngày —
/// cố ý, xem spec 2026-07-29-mua-cao-diem-design.md.
fn effective_special_uplift(inputs: &StayPricingInputs) -> BookingResult<f64>
```

- Lấy ngày của `check_in`, `check_out` bằng chính bộ phân tích mà
  `nights_between` đang dùng (`value.get(..10)`, an toàn với chuỗi nhiều byte).
- `total_days = (co_date - ci_date).num_days().max(1)` — trừ trên **`NaiveDate`**
  đã cắt, không phải trên datetime. Trừ datetime cho ra 3 với
  `12/02T14:00 → 16/02T12:00` trong khi đáp án là 4. Con số này phải **y hệt**
  `calculate_weekend_uplift` (`pricing.rs:440`), nếu không hai phụ thu sẽ bất
  đồng về "kỳ ở này gồm những ngày nào".
- Đi `total_days` bước từ `ci_date`, mỗi bước cộng `uplift_pct` của ngày đó,
  không có thì cộng 0.
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

**Sửa `date_key` luôn** (`pricing_queries.rs:72-78`). Nó đang là `&date_str[..10]`
— cắt theo **byte**, nên **panic** nếu byte thứ 10 rơi vào giữa một ký tự nhiều
byte. Tầng domain đã được vá đúng lỗi này ở phiên trước (`get(..10)`, kèm test
`nights_between_rejects_check_in_with_non_char_boundary_byte_ten_without_panicking`,
`domain/booking/pricing.rs:288`). Thay đổi này cho `date_key` ăn **hai** chuỗi
thay vì một, nên phải vá trước khi tăng phơi nhiễm: `get(..10).unwrap_or(date_str)`.

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

### Hai lệnh, không phải ba

```rust
#[tauri::command]
pub async fn save_special_date_range(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    remove: Vec<String>,
    from: String,
    to: String,
    label: String,
    uplift_pct: f64,
) -> Result<(), String>

#[tauri::command]
pub async fn delete_special_dates(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    dates: Vec<String>,
) -> Result<(), String>
```

`state` và `app` là bắt buộc, không phải trang trí: `require_admin` cần `state`,
và `emit_db_update` cần `AppHandle` (`commands/mod.rs:44`). `save_special_date`
cũ không có `app` — đừng chép chữ ký của nó.

`remove` là **chìa khoá của tính nguyên tử**. Khai mới thì nó rỗng. Sửa một cụm
cho ngắn lại thì nó chứa những ngày rơi ra khỏi khoảng mới, và cả việc xoá lẫn
việc ghi nằm trong **cùng một** transaction.

Nếu tách thành "gọi xoá rồi gọi ghi" thì đó là hai transaction: xoá xong mà ghi
hỏng là mất hẳn mấy ngày đã khai, màn hình tải lại hiện một mùa bị cụt, không ai
biết. Đúng cái "nửa khoảng nằm lại trong DB" mà spec này nói là không chấp nhận
được — và nó rơi vào luồng dễ gặp nhất.

Cả hai lệnh `require_admin`. Cả hai gọi `emit_db_update(&app, "pricing")` sau khi
commit, như `save_pricing_rule` đang làm (`commands/pricing.rs:92`) — giá vừa
đổi thì màn hình khác đang hiện giá phải biết.

### Bỏ `save_special_date`

Lệnh cũ (`commands/pricing.rs:169`, đăng ký ở `lib.rs:396`) **không có ai gọi** —
không trong `src/`, không trong `gateway/`, đã dò trên toàn bộ 73 nhánh còn sống:
ở đâu cũng chỉ có đúng hai chỗ, định nghĩa và dòng đăng ký. `gateway/proxy.rs`
không có đường chuyển tiếp lệnh theo tên, nên không ai với tới nó lúc chạy được.
Giữ nó lại nghĩa là có hai đường ghi vào cùng một bảng tiền, và phải nuôi một
test chỉ để bảo vệ một lệnh không ai dùng. Bỏ. `save_special_date_range` với
`from = to` làm được đúng việc ấy và làm được nhiều hơn.

Bỏ nó thì `pricing_repository::upsert_special_date(pool, …)`
(`pricing_repository.rs:74-98`) mất người gọi cuối cùng. **Chuyển nó thành bản
`_tx`**, đừng để lại bản nhận `pool`: nó `pub` trong lib crate nên `dead_code`
không cảnh báo, và nó sẽ mục ở đó không ai biết.

### Tầng nào làm gì

Theo `docs/architecture/core-pms-boundaries.md:100` ("new write behavior should
have a clear service or lifecycle home") và theo đúng lối `save_pricing_rule`
đang đi (`commands/pricing.rs:70-93` → `pricing_service.rs:101-140`):

| Tầng | Việc |
|---|---|
| `commands/pricing.rs` | `require_admin`, **sinh `id` và `now`**, gọi service, `emit_db_update` |
| `services/booking/pricing_service.rs` | **chặn đầu vào**, mở transaction, trải `from..to` thành danh sách ngày, gọi repository, commit |
| `repositories/booking/pricing_repository.rs` | `upsert_special_date_tx(tx, …)` một ngày một lần, và `delete_special_dates_tx(tx, dates)` |

**Chặn đầu vào ở service, không phải ở lệnh.** `core-pms-boundaries.md:110` cho
phép cả hai, nên đây là một lựa chọn thật chứ không phải chuyện hiển nhiên. Chọn
service vì `save_pricing_rule` đã đi lối ấy, và vì như thế thì lỗi trả về là
`CommandError::user` chứ không phải `String` trần. Bảy test chặn đầu vào cũng nằm
ở service.

**Sinh `id` và `now` ở tầng lệnh**, hệt `save_pricing_rule`
(`commands/pricing.rs:87-88` truyền `uuid::Uuid::new_v4().to_string()` và
`chrono::Local::now().to_rfc3339()` vào `pricing_service.rs:101-106`). Hai lý do,
lý do thứ hai mới là lý do thật:

1. Service thuần hơn — không tự đọc đồng hồ, không tự lấy số ngẫu nhiên.
2. **Đó là đường duy nhất để viết được cái test quan trọng nhất của phần này.**
   Mọi cột đều `NOT NULL` và do service tự điền, còn `ON CONFLICT(date)` đã nuốt
   mất ràng buộc duy nhất trên `date`. Chỗ hỏng-được-theo-ý còn lại trong
   transaction là **đụng khoá chính `id`** — mà muốn đụng thì `id` phải đến từ
   bên ngoài. Service tự sinh `id` là tự bịt mất chỗ đó.

**Thứ tự trong transaction: xoá `remove` trước, ghi khoảng sau.** Ghi trước xoá
sau thì một ngày vừa nằm trong `remove` vừa nằm trong `[from..to]` sẽ bị xoá mất
ngay bên trong chính cái transaction dựng lên để đừng mất nó. Giao diện như đặc
tả không bao giờ sinh ra chồng lấn ấy, nhưng đây là hợp đồng của lệnh chứ không
phải của một màn hình, nên **chặn luôn**: `remove` giao `[from..to]` phải rỗng.

Trải khoảng thành từng ngày là quy tắc nghiệp vụ, nên nó ở service; repository
giữ nguyên độ ngu ngốc của nó, mỗi lần một dòng, dùng lại `ON CONFLICT(date) DO
UPDATE` sẵn có. Ngày đã tồn tại thì `ON CONFLICT` giữ `id` và `created_at` cũ,
đúng như ghi chú ở `pricing_repository.rs:72`.

> **Lưu ý cho người làm.** `architecture_guard.rs:244` chỉ dò chuỗi `sqlx::query`
> trong `commands/`, nên một lệnh tự mở transaction vẫn **qua được** guard. Đừng
> lấy "test xanh" làm bằng chứng là đã đặt đúng tầng. Trong toàn bộ cây, không
> có lệnh nào cầm `Transaction<'_, Sqlite>`; mọi `pool.begin()` đều nằm ở service
> (hoặc `folio_repository.rs:20`). Đi theo lối đó.

### Chặn đầu vào (ở service, trước khi mở transaction)

`save_special_date_range`:

| Điều kiện | Vì sao |
|---|---|
| `from` và `to` đúng dạng `YYYY-MM-DD` | tránh ghi rác vào cột `date` |
| `to >= from` | gõ ngược thì khoảng rỗng, ghi xong không hiện ra |
| khoảng ≤ 366 ngày | gõ nhầm năm sẽ sinh hàng nghìn dòng |
| `0 <= uplift_pct <= 500` | `uplift_pct` là `REAL` không chặn gì; âm là giảm giá ngầm |
| `label` bỏ khoảng trắng còn khác rỗng | gom cụm dựa vào nhãn; nhãn rỗng gom nhầm hai kỳ khác nhau |
| `remove` **được phép rỗng** — đó là ca khai mới | đừng bắt buộc như `dates` |
| mỗi phần tử của `remove` đúng dạng `YYYY-MM-DD` | nó đi thẳng vào `DELETE … WHERE date IN (…)` |
| `remove` ≤ 366 phần tử | cùng lý do với trần của khoảng |
| `remove` giao `[from..to]` phải rỗng | xem phần thứ tự ở trên: chồng lấn là mất ngày |

`delete_special_dates`:

| Điều kiện | Vì sao |
|---|---|
| `dates` **không rỗng**, mỗi phần tử đúng dạng | xoá rỗng là lệnh vô nghĩa |

Hai bảng tách riêng vì luật của `remove` và của `dates` **ngược nhau** ở chỗ
rỗng: `remove` rỗng là hợp lệ, `dates` rỗng là lỗi.

Lỗi trả về tiếng Việt qua `CommandError::user`, theo lối `pricing_service.rs`.

### Không thêm

Không cần lệnh đọc mới. Màn hình đã tải toàn bộ qua `get_special_dates`, nên
việc dò trùng ngày làm ngay phía giao diện. Đây là ứng dụng một máy một người
dùng; khoảng hở giữa lúc dò và lúc ghi không có ý nghĩa thực tế.

Gateway MCP cũng không cần đổi gì. Không có công cụ MCP nào đọc hay ghi
`special_dates`. Nhưng công cụ `calculate_price` (`gateway/tools.rs:1233-1249`)
đi qua `load_stay_pricing_inputs_for_room_type` — một trong ba chỗ spec này sửa —
nên nó **được hưởng bản sửa miễn phí**, không phải đụng dòng nào.

## 3. Màn hình khai báo

Cài đặt → mục mới **Peak Season** (`CalendarDays`), chỉ hiện với admin, xếp cạnh
**Pricing** trong khối `isCurrentAdmin` ở `src/pages/settings/index.tsx:68`.
Nhãn thanh bên bằng tiếng Anh cho khớp các mục cũ; nội dung bên trong tiếng Việt,
đúng như `PricingSection.tsx` đang làm.

Đấu dây đủ ba chỗ, đừng sót: thêm `"peak-season"` vào `SettingsSectionKey`
(`index.tsx:33`), thêm một dòng render cạnh `{activeSection === "pricing" && …}`
(`index.tsx:132`), và đăng ký hai lệnh mới trong `invoke_handler` ở `lib.rs`.

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

- **Sửa**: đổ cụm vào form. Lưu = **một** lệnh
  `save_special_date_range(remove: ngày cũ không còn trong khoảng mới, …)`. Rút
  ngắn khoảng thì mấy ngày rơi ra bị xoá thật, không sót lại thành ngày lễ mồ
  côi — và xoá với ghi cùng chung một transaction, hỏng thì hỏng cả, không mất
  ngày nào.
- **Xoá**: hỏi lại một lần, rồi `delete_special_dates(cụm.dates)` — một lệnh cho
  cả cụm.
- Xong việc thì tải lại bằng `get_special_dates`, không tự sửa state cục bộ, để
  màn hình luôn phản ánh cái đang thật sự nằm trong DB.

Ghi qua `invokeWriteCommand` như `PricingSection.tsx:47`. Báo kết quả bằng
`toast`, theo đúng lối file đó.

### Hai file hạ tầng phải sửa kèm, không sửa là đỏ

- **`mhm/tests/frontend-invoke-wrapper-guardrails.test.ts`** là bánh cóc hai
  chiều: `RAW_INVOKE_ALLOWED_COMMANDS` (`:20-62`) liệt kê mọi `invoke` trần trong
  `src/` kèm lý do, và test ở `:260-290` đỏ với bất kỳ `invoke` trần nào không có
  trong danh sách. `get_special_dates` **chưa có** trong đó (hôm nay không ai
  gọi). Bắt chước y `PricingSection.tsx` — nơi `invoke("get_pricing_rules")` trần
  *đã* được cho phép ở `:45` — sẽ ra một test đỏ mà nguyên nhân rất khó lần. Phải
  thêm `get_special_dates` kèm lý do, và cân nhắc thêm hai lệnh ghi mới vào
  `PMS_WRITE_COMMANDS_REQUIRING_WRAPPER`.
- **`src/__mocks__/tauri-core.ts`** **ném lỗi** với mọi lệnh chưa đăng ký. Hai
  lệnh mới phải có trong `defaults`, hoặc mỗi test phải `setMockResponse`.

## 4. Kiểm chứng

Theo TDD: test đỏ trước, và với hai ca dưới đây phải **thấy** nó đỏ đúng vì lý do
số học, không phải vì thiếu hàm.

### Rust — `domain/booking/pricing.rs`

Ba luật bắt buộc cho mọi test ở phần này. Vi phạm luật nào cũng ra một test đỏ vì
lý do chẳng liên quan gì tới thứ đang kiểm.

**Một — ghi số tuyệt đối.** Cấm viết kiểu "tính từng đêm rồi cộng lại rồi so
bằng": xem phần Làm tròn, cách ấy lệch vài đồng với giá lẻ.

**Hai — khẳng định trên `surcharge_amount`, không phải `total`.** Mọi con số dưới
đây là **riêng phần phụ thu ngày lễ**, đúng bằng `surcharge_amount` trong
`calculate_nightly` (`pricing.rs:141`) và `calculate_hourly` (`:227`).

**Ba — đặt `weekend_uplift_pct = 0` trong fixture.** Cái bẫy: `sample_inputs()`
để `stored_rule: None`, nên `build_effective_pricing_rule`
(`domain/booking/pricing.rs:75-83`) rơi xuống `..Default::default()`, tức
`weekend_uplift_pct: 20.0` (`pricing.rs:46`). Mà **mọi** ngày trong các ví dụ
dưới đây đều đụng cuối tuần:

| Ngày | Thứ | | Ngày | Thứ |
|---|---|---|---|---|
| 12/02/2026 | Năm | | 20/02/2026 | Sáu |
| 13/02/2026 | Sáu | | **21/02/2026** | **Bảy** |
| **14/02/2026** | **Bảy** | | **22/02/2026** | **CN** |
| **15/02/2026** | **CN** | | 23/02/2026 | Hai |
| 16/02/2026 | Hai | | 24/02/2026 | Ba |

Để mặc 20% thì `total` gánh thêm phụ thu cuối tuần và mọi con số dưới đây sai
hết. Test cũ `calculate_from_loaded_inputs_applies_special_uplift` né bằng cách
chọn 20–22/04 (Hai–Ba) — đó là chủ ý, không phải ngẫu nhiên.

| Test | Con số |
|---|---|
| kỳ ở bắt đầu **trước** mùa | 12/02→16/02, Tết 14–22 +40%, giá 500k/đêm → phụ thu **400.000₫**, **không phải 0** |
| kỳ ở kéo **quá** mùa | 22/02→25/02, cùng khai → phụ thu **200.000₫**, **không phải 600.000₫** |
| vắt qua hai mức | 20/02→25/02, giá 500k/đêm, +40% ba đêm rồi +25% hai đêm → mức hiệu dụng 34%, phụ thu **850.000₫** trên `base` 2.500.000₫ |
| nằm trọn trong mùa | kết quả **y hệt** trước khi sửa — chống hồi quy |
| không khai ngày nào | `special_days` rỗng → phụ thu 0 |
| theo giờ, **cùng ngày** | ngày đó có khai → ra **như cũ** |
| theo giờ, **vắt hai ngày** | ví dụ 30 giờ ở phần `hourly` là ngoại lệ: `base` 600.000₫, phụ thu **120.000₫** — khoá lại đúng cái hành vi đã đổi, kèm chú thích là chủ ý |
| lễ trùng thứ 7 | vẫn ăn **cả hai** phụ thu — khoá hành vi hiện có |
| ngày sai định dạng trong DB | không panic, coi như không khai |

### Rust — `queries/booking/pricing_queries.rs`

- `load_special_days` chỉ trả ngày trong khoảng, bỏ ngày ngoài hai đầu.
- Ngày trả phòng nằm trong kết quả (cận trên bao gồm).
- Bản theo mã phòng và bản theo loại phòng đọc cùng một khoảng.
- `special_dates` hỏng: đường `_tx` ném lỗi, hai đường xem trước trả rỗng.

### Rust — service, lệnh và repository

- `save_special_date_range` 14/02→22/02 ghi đúng **9** dòng.
- `from = to` ghi đúng **một** dòng.
- Ghi đè khoảng có ngày trùng: `label` và `uplift_pct` đổi, `created_at` **giữ
  nguyên**.
- **`remove` và ghi trong cùng một transaction** — test quan trọng nhất của phần
  này, là lý do tồn tại của tham số `remove`. Cách làm cho nó hỏng có kiểm soát:
  truyền vào một `id` đã tồn tại trên **một ngày khác**, để bước upsert đụng khoá
  chính `id` giữa chừng. (`ON CONFLICT` chỉ đỡ cho `date`, không đỡ cho `id`.)
  Rồi khẳng định những ngày trong `remove` **vẫn còn nguyên** trong DB. Test này
  chỉ viết được vì `id` do tầng lệnh truyền vào — xem phần Tầng nào làm gì.
- `remove` giao `[from..to]` khác rỗng → bị từ chối, không ghi gì.
- Thứ tự đúng: `remove` được xoá **trước** khi ghi khoảng.
- `delete_special_dates` xoá đúng danh sách, không đụng ngày khác.
- Mỗi luật chặn đầu vào một test: ngày sai dạng, `to < from`, 367 ngày, `-10`,
  `600`, nhãn toàn khoảng trắng, `dates` rỗng.
- Không phải admin thì cả hai lệnh ghi bị từ chối.

### Frontend — `src/lib/specialDateRanges.test.ts`

- 9 dòng liền nhau cùng nhãn cùng mức → **1** cụm, `days = 9`, `dates` đủ 9.
- Hở một ngày → **2** cụm.
- Liền ngày nhưng khác nhãn → **2** cụm.
- Liền ngày, cùng nhãn, khác mức → **2** cụm.
- Đầu vào rỗng → mảng rỗng.
- Đầu vào không theo thứ tự ngày → vẫn gom đúng.
- Vắt qua ranh giới tháng và ranh giới năm (28/02→01/03, 31/12→01/01).
- Ngày sai định dạng trong DB → danh sách **không sập**. Cột `date` không có ràng
  buộc `CHECK` (`db/migrations.rs:209-216`) và lệnh ghi cũ chưa hề kiểm tra, nên
  dữ liệu rác là có thật.

Ở phía Rust, tính chất "ngày rác thì coi như không khai" **tự có** từ cách so
sánh: `effective_special_uplift` dựng khoá `YYYY-MM-DD` rồi dò trong danh sách,
nên chuỗi rác đơn giản là không bao giờ khớp. Đừng thêm đường phân tích-rồi-báo-
lỗi; làm thế là đổi đúng cái hành vi mà test này sinh ra để khoá.

### Frontend — `SpecialDatesSection.test.tsx`

- Danh sách hiện **một** dòng cho chín ngày Tết.
- Khai khoảng không trùng → gọi `save_special_date_range` đúng một lần với đúng
  đối số.
- Khai khoảng có trùng → hiện đúng danh sách ngày trùng; bấm **huỷ** thì
  `invokeWriteCommand` **không** được gọi.
- Bấm tiếp tục → mới gọi lệnh ghi.
- Xoá cụm → `delete_special_dates` nhận đúng 9 ngày trong một lần gọi.
- Sửa cụm cho ngắn lại → những ngày rơi ra có mặt trong tham số **`remove` của
  lệnh ghi**, và **không** có lệnh `delete_special_dates` nào được gọi. Test này
  chính là chỗ khoá lại việc bỏ luồng hai lệnh.
- Lệnh lỗi → hiện toast lỗi, danh sách không bị đổi ngầm.

### Cổng chung

`npm run verify:full`, `cargo check --all-targets`, và **`cargo fmt --check`** —
cái cuối CI có gác mà `verify:full` không chạy, phiên trước đã dính đỏ vì nó.

## Thứ tự thi công

Ba khối tách rời được, và **luật tính giá phải đi trước, thành commit riêng**:

1. **Luật đếm đêm** — `domain/booking/pricing.rs` + `queries/booking/pricing_queries.rs`
   (kèm bản vá `date_key`). Khối này ôm toàn bộ rủi ro về tiền và toàn bộ rủi ro
   va chạm. Nó đứng một mình cũng có giá trị: ai đã lỡ khai `special_dates` bằng
   tay thì được tính đúng ngay.
2. **Lệnh ghi** — command + service + repository, kèm test.
3. **Màn hình** — module gom cụm, section, và hai file hạ tầng nói ở trên.

Đừng trộn (1) vào giữa phần CRUD. Nếu phải quay lại hoặc phải gỡ tay lúc gộp
nhánh, tách sẵn thế này là đỡ nhất.

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
  Đây đúng là thứ `src/lib/datetime.ts` của nhánh
  `refactor/pricing-preview-honesty` (`localRfc3339` / `localDayKey`) sinh ra để
  vá. Để nhánh ấy lo, đừng vá song song.

## Rủi ro

- **Va chạm với `refactor/pricing-preview-honesty`.** Nhánh kia cắt từ `36878af`
  chứ không phải từ `main` (`a233ac9`), nên bên đó `load_stay_pricing_inputs_for_room`
  **chưa tồn tại** và các loader chưa có tham số `guests` — cả hai thứ ấy lên
  `main` sau. Nghĩa là nhánh kia phải rebase lên `main` trước đã; việc gỡ tay là
  của bước rebase đó, không phải của nhánh này.
  - Ở tầng engine, file production tranh chấp thật sự là
    **`queries/booking/pricing_queries.rs`**, một file.
  - `services/booking/pricing_service.rs` bị **cả hai** nhánh chạm, nhưng lệch
    vùng: nhánh này thêm code production (hai hàm ghi mới, theo phần 2), nhánh
    kia chỉ thêm trong `mod tests`. Nhiều khả năng tự gộp được, nhưng đừng đọc
    câu trên thành "chỉ một file bị đụng".
  - Trong file ấy, nhánh kia còn đổi `FALLBACK_BASE_PRICE_SQL` (thêm `ORDER BY
    id`, `pricing_queries.rs:25`) — khác vùng, nhiều khả năng tự gộp được, nhưng
    ghi ra đây để không ai bất ngờ.
  - Chốt sẵn cách gỡ: giữ **kiểu dữ liệu của nhánh này**, giữ **cách xử lỗi
    nghiêm của nhánh kia** (xem trước cũng ném lỗi).
- **Đổi luật là đổi giá cho khách đang ở, không chỉ đơn bị sửa.** Đơn đã lưu giữ
  `total_price` cũ. Nhưng `CheckoutSettlementMode::ActualNights`
  (`stay_lifecycle.rs:668-690`) **tính lại cả kỳ ở** lúc trả phòng. Khai Tết khi
  khách đang ở trong phòng sẽ đổi số tiền họ trả lúc đi, dù không ai sửa ngày,
  không ai gia hạn. Đây là **chủ ý** — luật cũ sai thì số cũ cũng sai — nhưng
  phải ghi ra để sau này không ai coi là lỗi. (`extend_stay` `:1078` thì không
  ảnh hưởng: nó chỉ tính một đêm tăng thêm, `N = 1`, luật cũ và mới ra cùng số.)
- **Gom cụm là suy đoán, không phải dữ liệu.** Hai kỳ khác nhau tình cờ liền
  ngày, cùng nhãn, cùng mức sẽ hiện thành một dòng. Vô hại — xoá cụm ấy đúng là
  xoá chừng đó ngày — nhưng phải nói rõ để sau này không ai coi đó là lỗi.
- **Không có ràng buộc `CHECK` dưới DB.** Mọi chặn đầu vào nằm ở tầng service. Ghi
  thẳng vào SQLite bằng tay vẫn lọt. Chấp nhận: mọi bảng khác trong dự án cũng
  thế.
