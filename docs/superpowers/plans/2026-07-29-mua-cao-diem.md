# Mùa cao điểm — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Chủ khách sạn khai được mùa cao điểm theo khoảng ngày, và engine tính phụ thu đúng theo từng đêm nằm trong mùa thay vì nhìn mỗi ngày nhận phòng.

**Architecture:** Engine giữ nguyên `crate::pricing::calculate_price` với tham số `f64` cũ; tầng `domain` quy đổi danh sách ngày lễ của kỳ ở thành **một mức bình quân** rồi truyền vào. Tầng `queries` đọc cả khoảng thay vì một ngày. Đường ghi mới đi `command → service → repository` trong một transaction. Giao diện là một mục Cài đặt mới, gom ngày liền nhau thành khoảng khi hiển thị.

**Tech Stack:** Rust + sqlx/SQLite + Tauri 2 (backend); React 18 + TypeScript + Vitest + shadcn/ui + sonner (frontend).

**Spec:** `docs/superpowers/specs/2026-07-29-mua-cao-diem-design.md` — đọc trước khi làm task đầu tiên.

**Worktree:** `/Users/binhan/HotelManager/.worktrees/peak-season`, nhánh `feature/peak-season`. Mọi đường dẫn dưới đây tính từ `mhm/` trong worktree đó.

## Global Constraints

- **Tiền là số nguyên VND.** `MoneyVnd = i64`. Không có `f64` nào chạm vào tiền. `uplift_pct` là phần trăm, **không phải tiền** — nó vẫn là `f64` như hiện tại.
- **Tầng lối đi cho ghi:** `UI → command → service → repository → SQLite`. Không lệnh nào được cầm `Transaction<'_, Sqlite>`. `architecture_guard.rs:244` **không** bắt được vi phạm này, nên đừng lấy test xanh làm bằng chứng.
- **Tầng lối đi cho đọc:** `UI → command → query → SQLite`.
- **`domain/`, `queries/`, `repositories/`, `services/` không bao giờ import `crate::commands`.**
- **Ngày dạng `YYYY-MM-DD` so sánh bằng chuỗi.** Không dùng `new Date("2026-02-14T00:00:00")` ở TypeScript — nó phân tích theo giờ địa phương rồi in ra UTC, lệch một ngày ở UTC+7.
- **Cắt chuỗi ngày bằng `get(..10)`**, không bao giờ `&s[..10]` — cắt theo byte sẽ panic với ký tự nhiều byte.
- **Chặn đầu vào nằm ở service**, trả `CommandError::user(codes::VALIDATION_INVALID_INPUT, …)` với thông điệp tiếng Việt.
- **Trần:** khoảng ≤ 366 ngày; `0 <= uplift_pct <= 500`.
- **Test Rust khẳng định trên `surcharge_amount`**, không phải `total`, và fixture phải đặt `weekend_uplift_pct = 0` — xem Task 2 Bước 1. **Một ngoại lệ duy nhất:** test `a_holiday_that_falls_on_a_weekend_still_charges_both` cố ý bật 20% và cố ý khẳng định `total`, vì thứ nó khoá chính là việc hai phụ thu cộng dồn.
- **Chạy `cargo fmt` trước mỗi lần commit backend.** CI gác `cargo fmt --check` nhưng `verify:full` không chạy nó.

---

## File Structure

| File | Trách nhiệm | Task |
|---|---|---|
| `src-tauri/src/queries/booking/pricing_queries.rs` | `date_key` an toàn; đọc ngày lễ cả khoảng | 1, 2 |
| `src-tauri/src/domain/booking/pricing.rs` | kiểu `SpecialDay`; quy tắc bình quân | 2 |
| `src-tauri/src/repositories/booking/pricing_repository.rs` | ghi/xoá một dòng `special_dates` trong transaction | 3 |
| `src-tauri/src/services/booking/pricing_service.rs` | chặn đầu vào, mở transaction, trải khoảng | 4, 5 |
| `src-tauri/src/commands/pricing.rs` | `require_admin`, sinh `id`/`now`, `emit_db_update` | 3, 4, 5 |
| `src-tauri/src/lib.rs` | đăng ký lệnh | 3, 4, 5 |
| `src/lib/specialDateRanges.ts` | **mới** — gom cụm và dò trùng, thuần, không React | 6 |
| `src/pages/settings/SpecialDatesSection.tsx` | **mới** — màn hình khai báo | 7 |
| `src/pages/settings/index.tsx` | thêm mục vào thanh bên | 7 |
| `src/__mocks__/tauri-core.ts` | mock hai lệnh mới | 7 |
| `tests/frontend-invoke-wrapper-guardrails.test.ts` | cho phép `get_special_dates` gọi trần | 7 |

---

### Task 1: `date_key` không được panic

Task nhỏ nhất và độc lập nhất. Làm trước vì Task 2 cho `date_key` ăn **hai** chuỗi thay vì một, tăng gấp đôi phơi nhiễm với lỗi này.

**Files:**
- Modify: `src-tauri/src/queries/booking/pricing_queries.rs:72-78`
- Test: cùng file, trong `mod tests`

**Interfaces:**
- Consumes: không có
- Produces: `fn date_key(date_str: &str) -> &str` — chữ ký không đổi, chỉ đổi ruột

- [ ] **Bước 1: Thêm test đỏ**

Thêm vào `mod tests` ở cuối `pricing_queries.rs`. Trong khối `use super::{…}` của module test, thêm `date_key` vào danh sách import.

```rust
    #[test]
    fn date_key_does_not_panic_when_byte_ten_splits_a_character() {
        // Chín ký tự ASCII rồi một ký tự hai byte: byte thứ 10 rơi vào giữa
        // ký tự ấy. `&value[..10]` sẽ panic ở đây.
        let value = "123456789é";

        assert_eq!(date_key(value), value);
    }
```

- [ ] **Bước 2: Chạy test cho thấy nó đỏ**

```bash
cd mhm/src-tauri && cargo test -p capyinn date_key_does_not_panic -- --nocapture
```

Kỳ vọng: **FAIL**, panic với thông điệp kiểu `byte index 10 is not a char boundary`.

- [ ] **Bước 3: Sửa `date_key`**

Thay toàn bộ thân hàm ở `pricing_queries.rs:72-78`:

```rust
/// `special_dates.date` is a bare `YYYY-MM-DD`, so an RFC3339 check-in has to
/// be truncated before it will match.
///
/// Cắt bằng `get` chứ không phải `&value[..10]`: chuỗi đến từ dữ liệu người
/// dùng, và cắt theo byte sẽ panic nếu byte thứ mười rơi vào giữa một ký tự
/// nhiều byte. Tầng domain đã vá đúng lỗi này ở `nights_between`.
fn date_key(date_str: &str) -> &str {
    date_str.get(..10).unwrap_or(date_str)
}
```

- [ ] **Bước 4: Chạy lại cho xanh**

```bash
cd mhm/src-tauri && cargo test -p capyinn date_key_does_not_panic
```

Kỳ vọng: **PASS**, `test result: ok. 1 passed`.

- [ ] **Bước 5: Chạy cả module để chắc không vỡ gì**

```bash
cd mhm/src-tauri && cargo test -p capyinn pricing_queries
```

Kỳ vọng: PASS toàn bộ, không có warning mới.

- [ ] **Bước 6: Commit**

```bash
cd mhm/src-tauri && cargo fmt
cd /Users/binhan/HotelManager/.worktrees/peak-season
git add mhm/src-tauri/src/queries/booking/pricing_queries.rs
git commit -m "fix(pricing): stop date_key panicking on a multi-byte boundary

It byte-sliced a string that comes from user data. The domain layer was
hardened against exactly this; the query layer was not, and the peak-season
change is about to feed it a second string."
```

---

### Task 2: Phụ thu ngày lễ tính theo từng đêm

Đây là **toàn bộ** thay đổi luật giá, trong một đơn vị biên dịch được. Đổi kiểu trong `domain` thì `queries` phải đổi theo cùng lúc, không tách được.

**Files:**
- Modify: `src-tauri/src/domain/booking/pricing.rs` (struct `StayPricingInputs`, `nights_between`, `calculate_from_loaded_inputs`, `mod tests`)
- Modify: `src-tauri/src/queries/booking/pricing_queries.rs` (hằng SQL, hai loader, ba chỗ dựng `StayPricingInputs`, `mod tests`)

**Interfaces:**
- Consumes: `date_key` từ Task 1
- Produces:
  - `pub(crate) struct SpecialDay { pub(crate) date: String, pub(crate) uplift_pct: f64 }` trong `domain::booking::pricing`
  - `StayPricingInputs.special_days: Vec<SpecialDay>` thay cho `special_uplift_pct: f64`
  - `fn date_only(value: &str) -> BookingResult<NaiveDate>` (riêng tư trong domain)
  - `fn effective_special_uplift(inputs: &StayPricingInputs) -> BookingResult<f64>` (riêng tư trong domain)

- [ ] **Bước 1: Sửa fixture test trước, và hiểu vì sao**

Trong `domain/booking/pricing.rs`, `mod tests`, hàm `sample_inputs()`: đổi `special_uplift_pct: 0.0` thành `special_days: Vec::new()`, và **thêm** một `stored_rule` có `weekend_uplift_pct: 0.0`.

Vì sao phải thêm `stored_rule`: `sample_inputs()` đang để `stored_rule: None`, nên `build_effective_pricing_rule` rơi xuống `..Default::default()`, tức `weekend_uplift_pct: 20.0` (`pricing.rs:46`). Mà **mọi** ngày trong các test dưới đây đều đụng cuối tuần — 14/02 và 15/02/2026 là thứ Bảy và Chủ nhật, 21/02 và 22/02 cũng vậy. Để mặc 20% thì `total` gánh thêm phụ thu cuối tuần và mọi con số sai hết.

```rust
    /// Giá 500k một đêm, **không** phụ thu cuối tuần. Bỏ `weekend_uplift_pct`
    /// đi là mọi con số trong các test ngày lễ dưới đây sai, vì `sample_inputs`
    /// không có `stored_rule` sẽ rơi xuống mặc định 20% của `PricingRule`.
    fn holiday_rule() -> StoredPricingRule {
        StoredPricingRule {
            room_type: "standard".to_string(),
            hourly_rate: 20_000,
            overnight_rate: 300_000,
            daily_rate: 500_000,
            overnight_start: "22:00".to_string(),
            overnight_end: "11:00".to_string(),
            daily_checkin: "14:00".to_string(),
            daily_checkout: "12:00".to_string(),
            early_checkin_surcharge_pct: 0.0,
            late_checkout_surcharge_pct: 0.0,
            weekend_uplift_pct: 0.0,
        }
    }

    fn special(date: &str, uplift_pct: f64) -> SpecialDay {
        SpecialDay { date: date.to_string(), uplift_pct }
    }

    /// Tết 14/02–22/02 +40%, đúng khai báo dùng chung cho các test dưới.
    fn tet_2026() -> Vec<SpecialDay> {
        (14..=22)
            .map(|day| special(&format!("2026-02-{day:02}"), 40.0))
            .collect()
    }
```

Sửa `sample_inputs()`:

```rust
    fn sample_inputs() -> StayPricingInputs {
        StayPricingInputs {
            room_type: "standard".to_string(),
            stored_rule: None,
            fallback_base_price: None,
            special_days: Vec::new(),
            check_in: "2026-04-20".to_string(),
            check_out: "2026-04-22".to_string(),
            pricing_type: "nightly".to_string(),
            guests: None,
            base_guests: 2,
            extra_person_fee: 0,
        }
    }
```

Trong `use super::{…}` của `mod tests`, thêm `SpecialDay`.

Hai test cũ đang gán `inputs.special_uplift_pct = 10.0` (`domain/booking/pricing.rs:256` và `:362`) — đổi thành:

```rust
        inputs.special_days = vec![special("2026-04-20", 10.0), special("2026-04-21", 10.0)];
```

Kỳ ở của `sample_inputs` là 20/04→22/04, tức hai đêm 20 và 21. Khai cả hai đêm ở 10% cho mức bình quân đúng bằng 10%, nên hai test cũ giữ nguyên con số kỳ vọng.

- [ ] **Bước 2: Viết các test đỏ cho luật mới**

Thêm vào `mod tests` của `domain/booking/pricing.rs`:

```rust
    #[test]
    fn special_uplift_covers_only_the_nights_inside_the_season() {
        // 12/02→16/02 là bốn đêm 12,13,14,15. Tết phủ 14 và 15.
        // Hôm nay engine nhìn mỗi ngày đến 12/02 rồi báo 0₫.
        let mut inputs = sample_inputs();
        inputs.stored_rule = Some(holiday_rule());
        inputs.special_days = tet_2026();
        inputs.check_in = "2026-02-12".to_string();
        inputs.check_out = "2026-02-16".to_string();

        let pricing = calculate_from_loaded_inputs(&inputs).unwrap();

        // 2 đêm × 500.000 × 40% = 400.000
        assert_eq!(pricing.surcharge_amount, 400_000);
    }

    #[test]
    fn special_uplift_stops_at_the_end_of_the_season() {
        // 22/02→25/02 là ba đêm 22,23,24. Chỉ đêm 22 còn trong Tết.
        // Hôm nay engine thấy ngày đến 22/02 là lễ rồi tính 40% cho cả ba đêm.
        let mut inputs = sample_inputs();
        inputs.stored_rule = Some(holiday_rule());
        inputs.special_days = tet_2026();
        inputs.check_in = "2026-02-22".to_string();
        inputs.check_out = "2026-02-25".to_string();

        let pricing = calculate_from_loaded_inputs(&inputs).unwrap();

        // 1 đêm × 500.000 × 40% = 200.000, không phải 600.000
        assert_eq!(pricing.surcharge_amount, 200_000);
    }

    #[test]
    fn special_uplift_spans_two_different_seasons() {
        // 20/02→25/02 là năm đêm. Tết +40% ba đêm 20,21,22; Hè +25% hai đêm 23,24.
        let mut inputs = sample_inputs();
        inputs.stored_rule = Some(holiday_rule());
        inputs.special_days = tet_2026()
            .into_iter()
            .chain([special("2026-02-23", 25.0), special("2026-02-24", 25.0)])
            .collect();
        inputs.check_in = "2026-02-20".to_string();
        inputs.check_out = "2026-02-25".to_string();

        let pricing = calculate_from_loaded_inputs(&inputs).unwrap();

        // Mức bình quân (40×3 + 25×2)/5 = 34%. base = 5 × 500.000 = 2.500.000.
        // 2.500.000 × 34% = 850.000 = 3×200.000 + 2×125.000.
        assert_eq!(pricing.base_amount, 2_500_000);
        assert_eq!(pricing.surcharge_amount, 850_000);
    }

    #[test]
    fn special_uplift_is_unchanged_for_a_stay_entirely_inside_the_season() {
        // Chống hồi quy: kỳ ở nằm trọn trong mùa phải ra y như luật cũ.
        let mut inputs = sample_inputs();
        inputs.stored_rule = Some(holiday_rule());
        inputs.special_days = tet_2026();
        inputs.check_in = "2026-02-15".to_string();
        inputs.check_out = "2026-02-18".to_string();

        let pricing = calculate_from_loaded_inputs(&inputs).unwrap();

        // Cả ba đêm 15,16,17 đều là Tết → vẫn đúng 40% trên toàn bộ base.
        assert_eq!(pricing.base_amount, 1_500_000);
        assert_eq!(pricing.surcharge_amount, 600_000);
    }

    #[test]
    fn no_special_days_means_no_surcharge() {
        let mut inputs = sample_inputs();
        inputs.stored_rule = Some(holiday_rule());
        inputs.check_in = "2026-02-12".to_string();
        inputs.check_out = "2026-02-16".to_string();

        let pricing = calculate_from_loaded_inputs(&inputs).unwrap();

        assert_eq!(pricing.surcharge_amount, 0);
    }

    #[test]
    fn hourly_stay_within_one_day_is_unchanged() {
        // N = 1, mức bình quân bằng đúng mức của ngày ấy. Kết quả như luật cũ.
        let mut inputs = sample_inputs();
        inputs.stored_rule = Some(holiday_rule());
        inputs.special_days = tet_2026();
        inputs.pricing_type = "hourly".to_string();
        inputs.check_in = "2026-02-14T08:00:00".to_string();
        inputs.check_out = "2026-02-14T13:00:00".to_string();

        let pricing = calculate_from_loaded_inputs(&inputs).unwrap();

        // 5 giờ × 20.000 = 100.000; 100.000 × 40% = 40.000.
        assert_eq!(pricing.base_amount, 100_000);
        assert_eq!(pricing.surcharge_amount, 40_000);
    }

    #[test]
    fn hourly_stay_across_two_days_is_prorated_and_this_is_a_deliberate_change() {
        // Ngoại lệ đã ghi trong spec: `base` của kiểu giờ không tính theo đêm,
        // nên chia đều theo ngày là cách phân bổ duy nhất có nghĩa. Trước khi
        // sửa, kỳ ở này ra 0₫ vì ngày đến 13/02 chưa khai.
        let mut inputs = sample_inputs();
        inputs.stored_rule = Some(holiday_rule());
        inputs.special_days = tet_2026();
        inputs.pricing_type = "hourly".to_string();
        inputs.check_in = "2026-02-13T20:00:00".to_string();
        inputs.check_out = "2026-02-15T02:00:00".to_string();

        let pricing = calculate_from_loaded_inputs(&inputs).unwrap();

        // 30 giờ × 20.000 = 600.000, không chạm trần nào.
        // Hai ngày lịch 13 và 14; (0 + 40)/2 = 20% → 120.000.
        assert_eq!(pricing.base_amount, 600_000);
        assert_eq!(pricing.surcharge_amount, 120_000);
    }

    #[test]
    fn a_holiday_that_falls_on_a_weekend_still_charges_both() {
        // Khoá hành vi hiện có: hai phụ thu cộng dồn, không cái nào nuốt cái
        // nào. Đây là quyết định về giá, đổi nó không thuộc phạm vi đợt này.
        // Chú ý fixture: test này là chỗ DUY NHẤT cố ý bật phụ thu cuối tuần.
        let mut rule = holiday_rule();
        rule.weekend_uplift_pct = 20.0;
        let mut inputs = sample_inputs();
        inputs.stored_rule = Some(rule);
        inputs.special_days = tet_2026();
        // 14/02/2026 là thứ Bảy, 15/02 là Chủ nhật — hai đêm, cả hai đều cuối
        // tuần và cả hai đều trong Tết.
        inputs.check_in = "2026-02-14".to_string();
        inputs.check_out = "2026-02-16".to_string();

        let pricing = calculate_from_loaded_inputs(&inputs).unwrap();

        assert_eq!(pricing.base_amount, 1_000_000);
        assert_eq!(pricing.weekend_amount, 200_000, "2 đêm × 500.000 × 20%");
        assert_eq!(pricing.surcharge_amount, 400_000, "2 đêm × 500.000 × 40%");
        assert_eq!(pricing.total, 1_600_000, "cả hai phụ thu đều được cộng");
    }

    #[test]
    fn a_malformed_stored_date_simply_never_matches() {
        // Cột `date` không có ràng buộc CHECK, nên rác là có thật. Tính chất
        // "coi như không khai" tự có từ việc so sánh chuỗi — đừng thêm đường
        // phân tích-rồi-báo-lỗi, làm thế là đổi đúng hành vi test này khoá.
        let mut inputs = sample_inputs();
        inputs.stored_rule = Some(holiday_rule());
        inputs.special_days = vec![special("khong-phai-ngay", 40.0)];
        inputs.check_in = "2026-02-12".to_string();
        inputs.check_out = "2026-02-16".to_string();

        let pricing = calculate_from_loaded_inputs(&inputs).unwrap();

        assert_eq!(pricing.surcharge_amount, 0);
    }
```

- [ ] **Bước 3: Chạy cho thấy đỏ vì lý do đúng**

```bash
cd mhm/src-tauri && cargo test -p capyinn domain::booking::pricing 2>&1 | head -40
```

Kỳ vọng: **FAIL biên dịch** — `no field 'special_days' on type 'StayPricingInputs'`, `cannot find function 'special'`, `cannot find type 'SpecialDay'`. Đây là đỏ đúng: kiểu chưa tồn tại.

- [ ] **Bước 4: Thêm kiểu `SpecialDay` và đổi trường**

Trong `domain/booking/pricing.rs`, trong `struct StayPricingInputs`, thay dòng `pub(crate) special_uplift_pct: f64,` bằng:

```rust
    /// Những ngày trong khoảng kỳ ở đã được khai là mùa cao điểm. Ngày không
    /// khai thì không có mặt. Rỗng nghĩa là không phụ thu.
    pub(crate) special_days: Vec<SpecialDay>,
```

Ngay sau `struct StayPricingInputs`, thêm:

```rust
/// Một ngày đã khai là mùa cao điểm.
///
/// Định nghĩa ở đây chứ không mượn `queries::booking::pricing_queries::SpecialDate`:
/// chiều phụ thuộc là queries → domain, và `architecture_guard` giữ chiều ấy.
#[derive(Debug, Clone)]
pub(crate) struct SpecialDay {
    /// `YYYY-MM-DD` đúng như nằm trong cột `special_dates.date`.
    pub(crate) date: String,
    pub(crate) uplift_pct: f64,
}
```

- [ ] **Bước 5: Tách `date_only` rồi viết `effective_special_uplift`**

Thay `nights_between` (hiện ở `domain/booking/pricing.rs:86-95`) bằng hai hàm:

```rust
/// Ngày lịch của một mốc thời gian.
///
/// Cắt bằng `get` chứ không phải `&value[..10]`: một chuỗi nhiều byte sẽ làm
/// bản cắt theo byte panic.
fn date_only(value: &str) -> BookingResult<NaiveDate> {
    let head = value.get(..10).unwrap_or(value);
    NaiveDate::parse_from_str(head, "%Y-%m-%d")
        .map_err(|error| BookingError::datetime_parse(error.to_string()))
}

pub(crate) fn nights_between(check_in: &str, check_out: &str) -> BookingResult<i64> {
    Ok((date_only(check_out)? - date_only(check_in)?)
        .num_days()
        .max(0))
}
```

Thêm ngay sau đó:

```rust
/// Mức uplift bình quân trên số ngày của kỳ ở.
///
/// Nhân mức này với `base` là chia đều `base` cho từng ngày rồi cho mỗi ngày
/// chịu mức của riêng nó: `base × (Σ pct_d / N) = Σ ((base/N) × pct_d)`.
///
/// Với `nightly`/`overnight`/`daily`, `base/N` đúng bằng giá một đêm đã cấu
/// hình, nên kết quả **là** "từng đêm chịu mức của nó". Với `hourly` thì `base`
/// không tính theo đêm và đây là phép phân bổ theo ngày — cố ý, xem spec
/// `2026-07-29-mua-cao-diem-design.md`.
///
/// Đi từng ngày y hệt `crate::pricing::calculate_weekend_uplift`, nên phụ thu
/// cuối tuần và phụ thu ngày lễ không bao giờ bất đồng về việc một kỳ ở gồm
/// những ngày nào.
fn effective_special_uplift(inputs: &StayPricingInputs) -> BookingResult<f64> {
    if inputs.special_days.is_empty() {
        return Ok(0.0);
    }

    let check_in = date_only(&inputs.check_in)?;
    let check_out = date_only(&inputs.check_out)?;
    let total_days = (check_out - check_in).num_days().max(1);

    let mut total_pct = 0.0;
    let mut date = check_in;
    for _ in 0..total_days {
        let key = date.format("%Y-%m-%d").to_string();
        if let Some(day) = inputs.special_days.iter().find(|day| day.date == key) {
            total_pct += day.uplift_pct;
        }
        date = date.succ_opt().unwrap_or(date);
    }

    Ok(total_pct / total_days as f64)
}
```

Trong `calculate_from_loaded_inputs`, đổi đối số cuối của `crate::pricing::calculate_price` từ `inputs.special_uplift_pct` thành `effective_special_uplift(inputs)?`:

```rust
    let mut result = crate::pricing::calculate_price(
        &rule,
        &inputs.check_in,
        &inputs.check_out,
        &inputs.pricing_type,
        effective_special_uplift(inputs)?,
    )
    .map_err(BookingError::datetime_parse)?;
```

- [ ] **Bước 6: Đổi tầng đọc**

Trong `queries/booking/pricing_queries.rs`:

Thay hằng `SPECIAL_UPLIFT_SQL` (`:41-42`) bằng:

```rust
const SPECIAL_DAYS_IN_RANGE_SQL: &str =
    "SELECT date, CAST(uplift_pct AS REAL) AS uplift_pct
     FROM special_dates WHERE date >= ? AND date <= ? ORDER BY date";
```

Trong khối `use crate::domain::booking::pricing::{…}` ở đầu file, thêm `SpecialDay`.

Thay `load_special_uplift` (`:311`) và `load_special_uplift_tx` (`:391`) bằng:

```rust
/// Ngày lễ trong khoảng kỳ ở.
///
/// Cận trên **bao gồm** ngày trả phòng. Đọc dư một ngày là cố ý: quyết định
/// "ngày nào là một đêm" thuộc về `domain`, tầng đọc không được tự cắt.
async fn load_special_days(
    pool: &Pool<Sqlite>,
    check_in: &str,
    check_out: &str,
) -> BookingResult<Vec<SpecialDay>> {
    let rows = sqlx::query_as(SPECIAL_DAYS_IN_RANGE_SQL)
        .bind(date_key(check_in))
        .bind(date_key(check_out))
        .fetch_all(pool)
        .await
        .map_err(database_error)?;

    Ok(special_days_from_rows(rows))
}

async fn load_special_days_tx(
    tx: &mut Transaction<'_, Sqlite>,
    check_in: &str,
    check_out: &str,
) -> BookingResult<Vec<SpecialDay>> {
    let rows = sqlx::query_as(SPECIAL_DAYS_IN_RANGE_SQL)
        .bind(date_key(check_in))
        .bind(date_key(check_out))
        .fetch_all(&mut **tx)
        .await
        .map_err(database_error)?;

    Ok(special_days_from_rows(rows))
}

/// Bản pool và bản transaction chỉ khác nhau ở chỗ chạy câu lệnh. Dùng chung
/// một bộ ánh xạ để hai đường không thể hiểu khác nhau về cùng một hàng.
fn special_days_from_rows(rows: Vec<(String, f64)>) -> Vec<SpecialDay> {
    rows.into_iter()
        .map(|(date, uplift_pct)| SpecialDay { date, uplift_pct })
        .collect()
}
```

Sửa ba chỗ dựng `StayPricingInputs`, **giữ nguyên** cách xử lỗi của từng chỗ:

- `load_stay_pricing_inputs_tx` (`:166`) — đường thu tiền thật, ném lỗi:
```rust
    let special_days = load_special_days_tx(tx, check_in, check_out).await?;
```
- `load_stay_pricing_inputs_for_room_type` (`:205`) — chỉ xem trước, khoan dung:
```rust
    let special_days = load_special_days(pool, check_in, check_out)
        .await
        .unwrap_or_default();
```
- `load_stay_pricing_inputs_for_room` (`:243`) — như trên, chép y hệt.

Ở cả ba, trong phần dựng struct, đổi `special_uplift_pct,` thành `special_days,`.

- [ ] **Bước 7: Chạy test domain cho xanh**

```bash
cd mhm/src-tauri && cargo test -p capyinn domain::booking::pricing
```

Kỳ vọng: **PASS**, kể cả tám test mới. Nếu `special_uplift_spans_two_different_seasons` đỏ ở 199.999 thay vì 200.000, dừng lại và báo — con số ấy phụ thuộc vào chế độ làm tròn ở `money.rs:58`.

- [ ] **Bước 8: Thêm test cho tầng đọc**

Trong `mod tests` của `pricing_queries.rs`. Hai test hiện có đang khẳng định `inputs.special_uplift_pct` (`:551`, `:583`) — đổi chúng sang `special_days`. Bảng dựng trong test hiện là `CREATE TABLE special_dates (date TEXT PRIMARY KEY, uplift_pct REAL)` (`:365`), đủ dùng.

```rust
    #[tokio::test]
    async fn load_special_days_returns_only_the_days_inside_the_stay() {
        let pool = test_pool().await;
        for (date, pct) in [
            ("2026-02-13", 40.0),
            ("2026-02-14", 40.0),
            ("2026-02-15", 40.0),
            ("2026-02-16", 40.0),
            ("2026-02-17", 40.0),
        ] {
            sqlx::query("INSERT INTO special_dates (date, uplift_pct) VALUES (?, ?)")
                .bind(date)
                .bind(pct)
                .execute(&pool)
                .await
                .unwrap();
        }

        let days = load_special_days(&pool, "2026-02-14", "2026-02-16")
            .await
            .unwrap();

        // Cận trên bao gồm ngày trả phòng, nên 16 có mặt; 13 và 17 thì không.
        let dates: Vec<&str> = days.iter().map(|day| day.date.as_str()).collect();
        assert_eq!(dates, vec!["2026-02-14", "2026-02-15", "2026-02-16"]);
    }

    #[tokio::test]
    async fn load_special_days_truncates_an_rfc3339_bound() {
        let pool = test_pool().await;
        sqlx::query("INSERT INTO special_dates (date, uplift_pct) VALUES (?, ?)")
            .bind("2026-02-14")
            .bind(40.0)
            .execute(&pool)
            .await
            .unwrap();

        let days = load_special_days(&pool, "2026-02-14T08:00:00+07:00", "2026-02-14T20:00:00+07:00")
            .await
            .unwrap();

        assert_eq!(days.len(), 1);
        assert_eq!(days[0].uplift_pct, 40.0);
    }
```

Hai test này dùng lại hàm dựng pool **đã có sẵn** trong `mod tests` của `pricing_queries.rs` — nó tạo `special_dates` ở `:365`. Mở file, chép đúng tên hàm, đừng viết mới.

Ngoài ra hai test cũ đang khẳng định `inputs.special_uplift_pct` ở `:551` và `:583`. Đổi chúng sang kiểu mới, ví dụ:

```rust
        assert_eq!(inputs.special_days.len(), 1);
        assert_eq!(inputs.special_days[0].uplift_pct, 10.0);
```
và
```rust
        assert!(inputs.special_days.is_empty());
```

Trong `use super::{…}` của module test, thêm `load_special_days`.

- [ ] **Bước 9: Chạy toàn bộ test Rust**

```bash
cd mhm/src-tauri && cargo test -p capyinn 2>&1 | tail -20
```

Kỳ vọng: **PASS**, không có warning mới. Nếu có chỗ nào khác còn nhắc `special_uplift_pct`, trình biên dịch sẽ chỉ đúng dòng.

- [ ] **Bước 10: Commit**

```bash
cd mhm/src-tauri && cargo fmt && cargo check --all-targets
cd /Users/binhan/HotelManager/.worktrees/peak-season
git add mhm/src-tauri/src/domain/booking/pricing.rs mhm/src-tauri/src/queries/booking/pricing_queries.rs
git commit -m "fix(pricing): charge the holiday uplift per night, not per check-in day

It looked special_dates up by the check-in date alone and then applied that one
percentage to every night. A stay starting the day before Tet was quoted at 0%;
a stay starting on the last day of Tet paid the full uplift for nights outside
it.

The rule now walks the stay day by day, exactly as the weekend uplift does, and
hands calculate_price the average — which is algebraically the same as charging
each night its own rate, so crate::pricing keeps its signature."
```

---

### Task 3: Tầng ghi một dòng, và bỏ lệnh cũ

**Files:**
- Modify: `src-tauri/src/repositories/booking/pricing_repository.rs:74-98`
- Modify: `src-tauri/src/commands/pricing.rs:168-190` (xoá `save_special_date`)
- Modify: `src-tauri/src/lib.rs:396` (bỏ đăng ký)

**Interfaces:**
- Consumes: không có
- Produces:
  - `pub struct SpecialDateUpsert<'a> { pub id: &'a str, pub date: &'a str, pub label: &'a str, pub uplift_pct: f64, pub now: &'a str }`
  - `pub async fn upsert_special_date_tx(tx: &mut Transaction<'_, Sqlite>, upsert: &SpecialDateUpsert<'_>) -> Result<(), sqlx::Error>`
  - `pub async fn delete_special_dates_tx(tx: &mut Transaction<'_, Sqlite>, dates: &[String]) -> Result<(), sqlx::Error>`

- [ ] **Bước 1: Viết test đỏ cho repository**

Trong `mod tests` của `pricing_repository.rs` (nếu file chưa có `mod tests`, tạo mới ở cuối file theo lối `settings_store.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::{delete_special_dates_tx, upsert_special_date_tx, SpecialDateUpsert};
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::{Executor, Pool, Sqlite};

    async fn test_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        pool.execute(
            "CREATE TABLE special_dates (
                id TEXT PRIMARY KEY,
                date TEXT NOT NULL,
                label TEXT NOT NULL DEFAULT '',
                uplift_pct REAL NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                UNIQUE(date)
            )",
        )
        .await
        .unwrap();
        pool
    }

    #[tokio::test]
    async fn upsert_keeps_the_original_created_at_when_the_date_already_exists() {
        let pool = test_pool().await;
        let mut tx = pool.begin().await.unwrap();
        upsert_special_date_tx(
            &mut tx,
            &SpecialDateUpsert {
                id: "id-1",
                date: "2026-02-14",
                label: "Tết",
                uplift_pct: 40.0,
                now: "2026-01-01T00:00:00+07:00",
            },
        )
        .await
        .unwrap();
        upsert_special_date_tx(
            &mut tx,
            &SpecialDateUpsert {
                id: "id-2",
                date: "2026-02-14",
                label: "Tết Nguyên đán",
                uplift_pct: 45.0,
                now: "2026-06-01T00:00:00+07:00",
            },
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let row: (String, String, f64, String) = sqlx::query_as(
            "SELECT id, label, uplift_pct, created_at FROM special_dates WHERE date = ?",
        )
        .bind("2026-02-14")
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(row.0, "id-1", "id cũ phải được giữ");
        assert_eq!(row.1, "Tết Nguyên đán");
        assert_eq!(row.2, 45.0);
        assert_eq!(row.3, "2026-01-01T00:00:00+07:00", "created_at phải được giữ");
    }

    #[tokio::test]
    async fn delete_removes_exactly_the_listed_dates() {
        let pool = test_pool().await;
        let mut tx = pool.begin().await.unwrap();
        for (index, date) in ["2026-02-14", "2026-02-15", "2026-02-16"].iter().enumerate() {
            upsert_special_date_tx(
                &mut tx,
                &SpecialDateUpsert {
                    id: &format!("id-{index}"),
                    date,
                    label: "Tết",
                    uplift_pct: 40.0,
                    now: "2026-01-01T00:00:00+07:00",
                },
            )
            .await
            .unwrap();
        }
        delete_special_dates_tx(&mut tx, &["2026-02-15".to_string()])
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let remaining: Vec<(String,)> =
            sqlx::query_as("SELECT date FROM special_dates ORDER BY date")
                .fetch_all(&pool)
                .await
                .unwrap();

        assert_eq!(
            remaining.iter().map(|row| row.0.as_str()).collect::<Vec<_>>(),
            vec!["2026-02-14", "2026-02-16"]
        );
    }
}
```

- [ ] **Bước 2: Chạy cho thấy đỏ**

```bash
cd mhm/src-tauri && cargo test -p capyinn pricing_repository 2>&1 | head -20
```

Kỳ vọng: **FAIL biên dịch** — `cannot find function 'upsert_special_date_tx'`.

- [ ] **Bước 3: Viết hai hàm repository**

Trong `pricing_repository.rs`, đổi dòng import đầu file thành `use sqlx::{Pool, Sqlite, Transaction};`.

Thay **toàn bộ** `upsert_special_date` (`:72-98`, kể cả khối doc phía trên) bằng:

```rust
/// Giá trị đã được service kiểm và đã đủ, writer không quyết định gì.
pub struct SpecialDateUpsert<'a> {
    pub id: &'a str,
    pub date: &'a str,
    pub label: &'a str,
    pub uplift_pct: f64,
    pub now: &'a str,
}

/// `created_at` chỉ được ghi lúc thêm mới; lần cập nhật giữ nguyên mốc cũ, nên
/// nó không nằm trong danh sách `DO UPDATE SET`. `id` cũng vậy: ngày đã tồn tại
/// thì giữ mã cũ, để mọi tham chiếu bên ngoài còn dùng được.
///
/// Nhận `tx` chứ không nhận `pool`: khai một khoảng là nhiều dòng, và mất nửa
/// khoảng còn tệ hơn báo lỗi.
pub async fn upsert_special_date_tx(
    tx: &mut Transaction<'_, Sqlite>,
    upsert: &SpecialDateUpsert<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO special_dates (id, date, label, uplift_pct, created_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(date) DO UPDATE SET
            label = excluded.label,
            uplift_pct = excluded.uplift_pct",
    )
    .bind(upsert.id)
    .bind(upsert.date)
    .bind(upsert.label)
    .bind(upsert.uplift_pct)
    .bind(upsert.now)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Xoá từng ngày một thay vì dựng `IN (…)`: sqlx không bind được mảng cho
/// SQLite, và ghép chuỗi SQL động là thứ không đáng đổi lấy một vòng lặp tối đa
/// 366 bước.
pub async fn delete_special_dates_tx(
    tx: &mut Transaction<'_, Sqlite>,
    dates: &[String],
) -> Result<(), sqlx::Error> {
    for date in dates {
        sqlx::query("DELETE FROM special_dates WHERE date = ?")
            .bind(date)
            .execute(&mut **tx)
            .await?;
    }

    Ok(())
}
```

- [ ] **Bước 4: Bỏ lệnh `save_special_date`**

Xoá toàn bộ khối `#[tauri::command]` + `pub async fn save_special_date(…)` ở `commands/pricing.rs:168-190` (mở file xác nhận biên trên và dưới trước khi xoá).

Trong `lib.rs`, xoá dòng `commands::pricing::save_special_date,` (`:396`).

Nếu sau khi xoá mà `uuid` hoặc `pricing_repository` không còn được `commands/pricing.rs` dùng nữa, xoá luôn dòng `use` tương ứng — `cargo check` sẽ báo warning nếu còn thừa.

- [ ] **Bước 5: Chạy cho xanh**

```bash
cd mhm/src-tauri && cargo test -p capyinn pricing_repository && cargo check --all-targets 2>&1 | tail -20
```

Kỳ vọng: hai test PASS, `cargo check` **không có warning nào**.

- [ ] **Bước 6: Commit**

```bash
cd mhm/src-tauri && cargo fmt
cd /Users/binhan/HotelManager/.worktrees/peak-season
git add mhm/src-tauri/src/repositories/booking/pricing_repository.rs mhm/src-tauri/src/commands/pricing.rs mhm/src-tauri/src/lib.rs
git commit -m "refactor(pricing): make the special-date write transactional, drop the dead command

save_special_date had no caller on any live branch — only its definition and
its invoke_handler line — and declaring a season is many rows, so the writer now
takes a transaction. Keeping the old one-row pool version would have left two
write paths into a money table for a command nobody invokes."
```

---

### Task 4: Lệnh khai một khoảng

**Files:**
- Modify: `src-tauri/src/services/booking/pricing_service.rs`
- Modify: `src-tauri/src/commands/pricing.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `SpecialDateUpsert`, `upsert_special_date_tx`, `delete_special_dates_tx` từ Task 3
- Produces:
  - `pub struct SaveSpecialDateRange { pub remove: Vec<String>, pub from: String, pub to: String, pub label: String, pub uplift_pct: f64 }`
  - `pub async fn save_special_date_range(pool: &Pool<Sqlite>, request: SaveSpecialDateRange, id_base: String, now: String) -> CommandResult<()>`
  - lệnh Tauri `save_special_date_range` với tham số `remove, from, to, label, uplift_pct`

- [ ] **Bước 1: Viết test đỏ ở service**

Trong `mod tests` của `pricing_service.rs`. Module ấy đã có hàm dựng pool riêng cho các test giá — **đừng sửa nó**, các test cũ đang dựa vào đúng những bảng nó tạo. Thêm một hàm riêng, chỉ dựng bảng cần cho phần này:

```rust
    async fn special_dates_pool() -> Pool<Sqlite> {
        use sqlx::sqlite::SqlitePoolOptions;
        use sqlx::Executor;

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        pool.execute(
            "CREATE TABLE special_dates (
                id TEXT PRIMARY KEY,
                date TEXT NOT NULL,
                label TEXT NOT NULL DEFAULT '',
                uplift_pct REAL NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                UNIQUE(date)
            )",
        )
        .await
        .unwrap();
        pool
    }
```

Lược đồ phải khớp `db/migrations.rs:209-216`, đặc biệt là `id TEXT PRIMARY KEY` — test `a_failed_write_leaves_every_removed_day_in_place` dựa vào đúng ràng buộc ấy để ép transaction hỏng.

Trong `use super::{…}` của module test, thêm `delete_special_dates, save_special_date_range, SaveSpecialDateRange`.

```rust
    #[tokio::test]
    async fn save_special_date_range_writes_one_row_per_day() {
        let pool = special_dates_pool().await;

        save_special_date_range(
            &pool,
            SaveSpecialDateRange {
                remove: Vec::new(),
                from: "2026-02-14".to_string(),
                to: "2026-02-22".to_string(),
                label: "Tết Nguyên đán".to_string(),
                uplift_pct: 40.0,
            },
            "base".to_string(),
            "2026-01-01T00:00:00+07:00".to_string(),
        )
        .await
        .unwrap();

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM special_dates")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 9);
    }

    #[tokio::test]
    async fn save_special_date_range_accepts_a_single_day() {
        let pool = special_dates_pool().await;

        save_special_date_range(
            &pool,
            SaveSpecialDateRange {
                remove: Vec::new(),
                from: "2026-04-30".to_string(),
                to: "2026-04-30".to_string(),
                label: "Lễ 30/4".to_string(),
                uplift_pct: 30.0,
            },
            "base".to_string(),
            "2026-01-01T00:00:00+07:00".to_string(),
        )
        .await
        .unwrap();

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM special_dates")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 1);
    }

    #[tokio::test]
    async fn a_failed_write_leaves_every_removed_day_in_place() {
        // Test quan trọng nhất của task này: `remove` tồn tại để việc xoá và
        // việc ghi cùng chung một transaction.
        //
        // Ép hỏng giữa chừng bằng cách cho `id_base` sinh ra một mã đã tồn tại
        // trên một ngày KHÁC, để bước upsert đụng khoá chính. `ON CONFLICT` chỉ
        // đỡ cho `date`, không đỡ cho `id` — đây là chỗ hỏng-được-theo-ý duy
        // nhất, và nó chỉ dùng được vì `id` do bên ngoài truyền vào.
        let pool = special_dates_pool().await;
        sqlx::query(
            "INSERT INTO special_dates (id, date, label, uplift_pct, created_at)
             VALUES ('base-1', '2026-12-25', 'Noel', 10.0, '2026-01-01T00:00:00+07:00'),
                    ('keep-1', '2026-02-25', 'Tết', 40.0, '2026-01-01T00:00:00+07:00')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let error = save_special_date_range(
            &pool,
            SaveSpecialDateRange {
                remove: vec!["2026-02-25".to_string()],
                from: "2026-02-14".to_string(),
                to: "2026-02-16".to_string(),
                label: "Tết".to_string(),
                uplift_pct: 40.0,
            },
            "base".to_string(),
            "2026-06-01T00:00:00+07:00".to_string(),
        )
        .await
        .expect_err("mã trùng phải làm cả transaction hỏng");
        let _ = error;

        let kept: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM special_dates WHERE date = '2026-02-25'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(kept.0, 1, "ngày trong `remove` phải còn nguyên khi ghi hỏng");

        let written: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM special_dates WHERE date = '2026-02-14'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(written.0, 0, "không được để lại nửa khoảng");
    }

    #[tokio::test]
    async fn remove_is_deleted_before_the_range_is_written() {
        let pool = special_dates_pool().await;
        sqlx::query(
            "INSERT INTO special_dates (id, date, label, uplift_pct, created_at)
             VALUES ('old-1', '2026-02-20', 'Tết', 40.0, '2026-01-01T00:00:00+07:00')",
        )
        .execute(&pool)
        .await
        .unwrap();

        save_special_date_range(
            &pool,
            SaveSpecialDateRange {
                remove: vec!["2026-02-20".to_string()],
                from: "2026-02-14".to_string(),
                to: "2026-02-16".to_string(),
                label: "Tết".to_string(),
                uplift_pct: 40.0,
            },
            "base".to_string(),
            "2026-06-01T00:00:00+07:00".to_string(),
        )
        .await
        .unwrap();

        let dates: Vec<(String,)> = sqlx::query_as("SELECT date FROM special_dates ORDER BY date")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(
            dates.iter().map(|row| row.0.as_str()).collect::<Vec<_>>(),
            vec!["2026-02-14", "2026-02-15", "2026-02-16"]
        );
    }

    #[tokio::test]
    async fn save_special_date_range_rejects_bad_input() {
        let pool = special_dates_pool().await;

        let base = || SaveSpecialDateRange {
            remove: Vec::new(),
            from: "2026-02-14".to_string(),
            to: "2026-02-16".to_string(),
            label: "Tết".to_string(),
            uplift_pct: 40.0,
        };
        let run = |request| {
            let pool = pool.clone();
            async move {
                save_special_date_range(
                    &pool,
                    request,
                    "base".to_string(),
                    "2026-01-01T00:00:00+07:00".to_string(),
                )
                .await
            }
        };

        // ngày sai định dạng
        let mut request = base();
        request.from = "14/02/2026".to_string();
        run(request).await.expect_err("ngày sai định dạng");

        // ngày kết thúc trước ngày bắt đầu
        let mut request = base();
        request.to = "2026-02-10".to_string();
        run(request).await.expect_err("to < from");

        // quá 366 ngày
        let mut request = base();
        request.to = "2027-02-16".to_string();
        run(request).await.expect_err("367 ngày");

        // phần trăm âm
        let mut request = base();
        request.uplift_pct = -10.0;
        run(request).await.expect_err("phần trăm âm");

        // phần trăm quá trần
        let mut request = base();
        request.uplift_pct = 600.0;
        run(request).await.expect_err("phần trăm quá 500");

        // nhãn toàn khoảng trắng
        let mut request = base();
        request.label = "   ".to_string();
        run(request).await.expect_err("nhãn rỗng");

        // phần tử của remove sai định dạng
        let mut request = base();
        request.remove = vec!["hôm-qua".to_string()];
        run(request).await.expect_err("remove sai định dạng");

        // remove chồng lên khoảng đang khai
        let mut request = base();
        request.remove = vec!["2026-02-15".to_string()];
        run(request).await.expect_err("remove chồng lấn");

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM special_dates")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 0, "không lần nào được ghi gì");
    }
```

- [ ] **Bước 2: Chạy cho thấy đỏ**

```bash
cd mhm/src-tauri && cargo test -p capyinn pricing_service 2>&1 | head -20
```

Kỳ vọng: **FAIL biên dịch** — `cannot find function 'save_special_date_range'`.

- [ ] **Bước 3: Viết service**

Thêm vào `pricing_service.rs`. Bổ sung import: `use chrono::NaiveDate;`, `use crate::app_error::{codes, CommandError};`, và thêm `delete_special_dates_tx, upsert_special_date_tx, SpecialDateUpsert` vào khối `use crate::repositories::booking::pricing_repository::{…}`.

```rust
const MAX_SPECIAL_RANGE_DAYS: i64 = 366;
const MAX_SPECIAL_UPLIFT_PCT: f64 = 500.0;

pub struct SaveSpecialDateRange {
    /// Những ngày phải xoá **cùng** transaction với lần ghi này. Rỗng là khai
    /// mới; có giá trị là đang sửa một cụm cho ngắn lại. Đây là lý do lệnh xoá
    /// và lệnh ghi không tách rời: xoá xong mà ghi hỏng là mất hẳn mấy ngày đã
    /// khai, và không ai biết.
    pub remove: Vec<String>,
    pub from: String,
    pub to: String,
    pub label: String,
    pub uplift_pct: f64,
}

fn parse_date_only(value: &str, field: &str) -> CommandResult<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
        CommandError::user(
            codes::VALIDATION_INVALID_INPUT,
            format!("{field} phải có dạng YYYY-MM-DD"),
        )
    })
}

fn invalid_input(message: impl Into<String>) -> CommandError {
    CommandError::user(codes::VALIDATION_INVALID_INPUT, message)
}

/// Khai một khoảng ngày là mùa cao điểm, một dòng cho một ngày.
///
/// `id_base` cấp mã cho từng ngày mới theo dạng `{id_base}-{chỉ số}`. Lệnh
/// truyền vào một uuid nên trong thực tế không bao giờ đụng; test thì truyền
/// một gốc đã có trong bảng để ép lỗi khoá chính giữa transaction — đó là chỗ
/// hỏng-được-theo-ý duy nhất, vì `ON CONFLICT(date)` đã nuốt mất ràng buộc trên
/// `date`.
pub async fn save_special_date_range(
    pool: &Pool<Sqlite>,
    request: SaveSpecialDateRange,
    id_base: String,
    now: String,
) -> CommandResult<()> {
    let from = parse_date_only(&request.from, "Ngày bắt đầu")?;
    let to = parse_date_only(&request.to, "Ngày kết thúc")?;
    if to < from {
        return Err(invalid_input("Ngày kết thúc không được trước ngày bắt đầu"));
    }

    let day_count = (to - from).num_days() + 1;
    if day_count > MAX_SPECIAL_RANGE_DAYS {
        return Err(invalid_input(format!(
            "Khoảng ngày tối đa {MAX_SPECIAL_RANGE_DAYS} ngày"
        )));
    }

    if !(0.0..=MAX_SPECIAL_UPLIFT_PCT).contains(&request.uplift_pct) {
        return Err(invalid_input(format!(
            "Mức phụ thu phải trong khoảng 0–{MAX_SPECIAL_UPLIFT_PCT:.0}%"
        )));
    }

    let label = request.label.trim();
    if label.is_empty() {
        return Err(invalid_input("Tên đợt cao điểm không được để trống"));
    }

    if request.remove.len() as i64 > MAX_SPECIAL_RANGE_DAYS {
        return Err(invalid_input(format!(
            "Chỉ xoá được tối đa {MAX_SPECIAL_RANGE_DAYS} ngày một lần"
        )));
    }
    for date in &request.remove {
        let removed = parse_date_only(date, "Ngày cần xoá")?;
        if removed >= from && removed <= to {
            return Err(invalid_input(
                "Ngày cần xoá không được nằm trong chính khoảng đang khai",
            ));
        }
    }

    let mut tx = pool.begin().await.map_err(|error| {
        crate::app_error::log_system_error(
            "save_special_date_range",
            error.to_string(),
            serde_json::json!({ "step": "begin" }),
        )
    })?;

    // Xoá trước, ghi sau. Ngược lại thì một ngày vừa nằm trong `remove` vừa
    // nằm trong khoảng sẽ bị xoá mất ngay bên trong transaction dựng lên để
    // đừng mất nó. Đã chặn chồng lấn ở trên, nhưng thứ tự vẫn phải đúng.
    delete_special_dates_tx(&mut tx, &request.remove)
        .await
        .map_err(|error| {
            crate::app_error::log_system_error(
                "save_special_date_range",
                error.to_string(),
                serde_json::json!({ "step": "delete_special_dates_tx" }),
            )
        })?;

    let mut date = from;
    for index in 0..day_count {
        let id = format!("{id_base}-{index}");
        let date_key = date.format("%Y-%m-%d").to_string();
        upsert_special_date_tx(
            &mut tx,
            &SpecialDateUpsert {
                id: &id,
                date: &date_key,
                label,
                uplift_pct: request.uplift_pct,
                now: &now,
            },
        )
        .await
        .map_err(|error| {
            crate::app_error::log_system_error(
                "save_special_date_range",
                error.to_string(),
                serde_json::json!({ "step": "upsert_special_date_tx", "date": &date_key }),
            )
        })?;
        date = date.succ_opt().unwrap_or(date);
    }

    tx.commit().await.map_err(|error| {
        crate::app_error::log_system_error(
            "save_special_date_range",
            error.to_string(),
            serde_json::json!({ "step": "commit" }),
        )
    })
}
```

- [ ] **Bước 4: Chạy cho xanh**

```bash
cd mhm/src-tauri && cargo test -p capyinn pricing_service
```

Kỳ vọng: **PASS** toàn bộ, kể cả năm test mới.

- [ ] **Bước 5: Thêm lệnh Tauri**

Trong `commands/pricing.rs`, thêm (chỗ `save_special_date` vừa bị xoá ở Task 3):

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
) -> Result<(), String> {
    require_admin(&state)?;

    pricing_service::save_special_date_range(
        &state.db,
        pricing_service::SaveSpecialDateRange {
            remove,
            from,
            to,
            label,
            uplift_pct,
        },
        uuid::Uuid::new_v4().to_string(),
        chrono::Local::now().to_rfc3339(),
    )
    .await?;

    emit_db_update(&app, "pricing");
    Ok(())
}
```

Xác nhận `emit_db_update`, `require_admin`, `pricing_service` đều đã có trong `use` của file; `save_pricing_rule` ngay trên đó dùng cả ba.

Trong `lib.rs`, thêm vào `invoke_handler`, ngay dưới `commands::pricing::get_special_dates,`:

```rust
            commands::pricing::save_special_date_range,
```

- [ ] **Bước 6: Kiểm tra biên dịch**

```bash
cd mhm/src-tauri && cargo check --all-targets 2>&1 | tail -20
```

Kỳ vọng: sạch, không warning.

- [ ] **Bước 7: Commit**

```bash
cd mhm/src-tauri && cargo fmt
cd /Users/binhan/HotelManager/.worktrees/peak-season
git add mhm/src-tauri/src/services/booking/pricing_service.rs mhm/src-tauri/src/commands/pricing.rs mhm/src-tauri/src/lib.rs
git commit -m "feat(pricing): declare a peak season as one date range

The removals ride along with the write instead of taking a command of their
own, so shortening a season cannot lose days to a failure between two
transactions. The id comes from the command, matching save_pricing_rule — which
is also what makes the mid-transaction failure testable at all."
```

---

### Task 5: Lệnh xoá

**Files:**
- Modify: `src-tauri/src/services/booking/pricing_service.rs`
- Modify: `src-tauri/src/commands/pricing.rs`
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: `delete_special_dates_tx` từ Task 3, `parse_date_only`/`invalid_input` từ Task 4
- Produces: `pub async fn delete_special_dates(pool: &Pool<Sqlite>, dates: Vec<String>) -> CommandResult<()>`; lệnh Tauri `delete_special_dates` với tham số `dates`

- [ ] **Bước 1: Viết test đỏ**

Trong `mod tests` của `pricing_service.rs`:

```rust
    #[tokio::test]
    async fn delete_special_dates_removes_a_whole_cluster_at_once() {
        let pool = special_dates_pool().await;
        save_special_date_range(
            &pool,
            SaveSpecialDateRange {
                remove: Vec::new(),
                from: "2026-02-14".to_string(),
                to: "2026-02-22".to_string(),
                label: "Tết".to_string(),
                uplift_pct: 40.0,
            },
            "base".to_string(),
            "2026-01-01T00:00:00+07:00".to_string(),
        )
        .await
        .unwrap();

        let dates: Vec<String> = (14..=22).map(|day| format!("2026-02-{day:02}")).collect();
        delete_special_dates(&pool, dates).await.unwrap();

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM special_dates")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 0);
    }

    #[tokio::test]
    async fn delete_special_dates_rejects_an_empty_list_and_a_bad_date() {
        let pool = special_dates_pool().await;

        delete_special_dates(&pool, Vec::new())
            .await
            .expect_err("danh sách rỗng là lệnh vô nghĩa");
        delete_special_dates(&pool, vec!["14/02/2026".to_string()])
            .await
            .expect_err("ngày sai định dạng");
    }
```

- [ ] **Bước 2: Chạy cho thấy đỏ**

```bash
cd mhm/src-tauri && cargo test -p capyinn delete_special_dates 2>&1 | head -20
```

Kỳ vọng: **FAIL biên dịch** — `cannot find function 'delete_special_dates'`.

- [ ] **Bước 3: Viết service**

Thêm vào `pricing_service.rs`, ngay dưới `save_special_date_range`:

```rust
/// Xoá hẳn một số ngày khỏi bảng mùa cao điểm.
///
/// Danh sách rỗng bị từ chối — khác hẳn `SaveSpecialDateRange::remove`, nơi
/// rỗng chính là ca khai mới. Ở đây rỗng nghĩa là gọi nhầm.
pub async fn delete_special_dates(pool: &Pool<Sqlite>, dates: Vec<String>) -> CommandResult<()> {
    if dates.is_empty() {
        return Err(invalid_input("Chưa chọn ngày nào để xoá"));
    }
    if dates.len() as i64 > MAX_SPECIAL_RANGE_DAYS {
        return Err(invalid_input(format!(
            "Chỉ xoá được tối đa {MAX_SPECIAL_RANGE_DAYS} ngày một lần"
        )));
    }
    for date in &dates {
        parse_date_only(date, "Ngày cần xoá")?;
    }

    let mut tx = pool.begin().await.map_err(|error| {
        crate::app_error::log_system_error(
            "delete_special_dates",
            error.to_string(),
            serde_json::json!({ "step": "begin" }),
        )
    })?;

    delete_special_dates_tx(&mut tx, &dates)
        .await
        .map_err(|error| {
            crate::app_error::log_system_error(
                "delete_special_dates",
                error.to_string(),
                serde_json::json!({ "step": "delete_special_dates_tx" }),
            )
        })?;

    tx.commit().await.map_err(|error| {
        crate::app_error::log_system_error(
            "delete_special_dates",
            error.to_string(),
            serde_json::json!({ "step": "commit" }),
        )
    })
}
```

- [ ] **Bước 4: Chạy cho xanh**

```bash
cd mhm/src-tauri && cargo test -p capyinn pricing_service
```

Kỳ vọng: **PASS**.

- [ ] **Bước 5: Thêm lệnh Tauri và đăng ký**

Trong `commands/pricing.rs`, ngay dưới `save_special_date_range`:

```rust
#[tauri::command]
pub async fn delete_special_dates(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    dates: Vec<String>,
) -> Result<(), String> {
    require_admin(&state)?;

    pricing_service::delete_special_dates(&state.db, dates).await?;

    emit_db_update(&app, "pricing");
    Ok(())
}
```

Trong `lib.rs`, thêm dưới dòng vừa thêm ở Task 4:

```rust
            commands::pricing::delete_special_dates,
```

- [ ] **Bước 6: Chạy toàn bộ Rust**

```bash
cd mhm/src-tauri && cargo test -p capyinn 2>&1 | tail -10 && cargo check --all-targets 2>&1 | tail -10
```

Kỳ vọng: PASS toàn bộ, `cargo check` sạch.

- [ ] **Bước 7: Commit**

```bash
cd mhm/src-tauri && cargo fmt
cd /Users/binhan/HotelManager/.worktrees/peak-season
git add mhm/src-tauri/src/services/booking/pricing_service.rs mhm/src-tauri/src/commands/pricing.rs mhm/src-tauri/src/lib.rs
git commit -m "feat(pricing): let a declared peak season be deleted

There was no delete command at all, so a mistyped season was permanent."
```

---

### Task 6: Gom ngày liền nhau thành khoảng

Hàm thuần, không React, để thử được mà không dựng component.

**Files:**
- Create: `src/lib/specialDateRanges.ts`
- Test: `src/lib/specialDateRanges.test.ts`

**Interfaces:**
- Consumes: không có
- Produces:
  - `export type SpecialDateRow = { id: string; date: string; label: string; uplift_pct: number }`
  - `export type SpecialDateRange = { from: string; to: string; days: number; label: string; uplift_pct: number; dates: string[] }`
  - `export function groupSpecialDates(rows: SpecialDateRow[]): SpecialDateRange[]`
  - `export function overlappingDates(rows: SpecialDateRow[], from: string, to: string, exclude?: string[]): SpecialDateRow[]`

- [ ] **Bước 1: Viết test đỏ**

Tạo `src/lib/specialDateRanges.test.ts`:

```ts
import { describe, expect, it } from "vitest";

import { groupSpecialDates, overlappingDates, type SpecialDateRow } from "./specialDateRanges";

function row(date: string, label = "Tết", uplift_pct = 40): SpecialDateRow {
    return { id: `id-${date}`, date, label, uplift_pct };
}

describe("groupSpecialDates", () => {
    it("gom chín ngày liền nhau thành một khoảng", () => {
        const rows = Array.from({ length: 9 }, (_, index) => row(`2026-02-${14 + index}`));

        const ranges = groupSpecialDates(rows);

        expect(ranges).toHaveLength(1);
        expect(ranges[0].from).toBe("2026-02-14");
        expect(ranges[0].to).toBe("2026-02-22");
        expect(ranges[0].days).toBe(9);
        expect(ranges[0].dates).toHaveLength(9);
    });

    it("tách khi hở một ngày", () => {
        const ranges = groupSpecialDates([row("2026-02-14"), row("2026-02-16")]);

        expect(ranges).toHaveLength(2);
        expect(ranges.map((range) => range.days)).toEqual([1, 1]);
    });

    it("tách khi liền ngày nhưng khác nhãn", () => {
        const ranges = groupSpecialDates([
            row("2026-02-14", "Tết"),
            row("2026-02-15", "Hè"),
        ]);

        expect(ranges).toHaveLength(2);
    });

    it("tách khi liền ngày, cùng nhãn, nhưng khác mức", () => {
        const ranges = groupSpecialDates([
            row("2026-02-14", "Tết", 40),
            row("2026-02-15", "Tết", 25),
        ]);

        expect(ranges).toHaveLength(2);
    });

    it("gom đúng dù đầu vào không theo thứ tự ngày", () => {
        const ranges = groupSpecialDates([
            row("2026-02-16"),
            row("2026-02-14"),
            row("2026-02-15"),
        ]);

        expect(ranges).toHaveLength(1);
        expect(ranges[0].dates).toEqual(["2026-02-14", "2026-02-15", "2026-02-16"]);
    });

    it("vắt qua ranh giới tháng", () => {
        const ranges = groupSpecialDates([row("2026-02-28"), row("2026-03-01")]);

        expect(ranges).toHaveLength(1);
        expect(ranges[0].days).toBe(2);
    });

    it("vắt qua ranh giới năm", () => {
        const ranges = groupSpecialDates([row("2026-12-31"), row("2027-01-01")]);

        expect(ranges).toHaveLength(1);
        expect(ranges[0].days).toBe(2);
    });

    it("đầu vào rỗng trả mảng rỗng", () => {
        expect(groupSpecialDates([])).toEqual([]);
    });

    it("bỏ qua ngày sai định dạng thay vì sập", () => {
        // Cột `date` không có ràng buộc CHECK dưới DB, nên rác là có thật.
        const ranges = groupSpecialDates([row("khong-phai-ngay"), row("2026-02-14")]);

        expect(ranges).toHaveLength(1);
        expect(ranges[0].from).toBe("2026-02-14");
    });
});

describe("overlappingDates", () => {
    it("chỉ trả những ngày đã khai nằm trong khoảng mới", () => {
        const rows = [row("2026-02-19"), row("2026-02-20"), row("2026-02-21")];

        const clashes = overlappingDates(rows, "2026-02-20", "2026-02-28");

        expect(clashes.map((clash) => clash.date)).toEqual(["2026-02-20", "2026-02-21"]);
    });

    it("không tính ngày của chính cụm đang sửa là trùng", () => {
        const rows = [row("2026-02-20"), row("2026-02-21")];

        const clashes = overlappingDates(rows, "2026-02-20", "2026-02-28", [
            "2026-02-20",
            "2026-02-21",
        ]);

        expect(clashes).toEqual([]);
    });
});
```

- [ ] **Bước 2: Chạy cho thấy đỏ**

```bash
cd mhm && npx vitest run src/lib/specialDateRanges.test.ts
```

Kỳ vọng: **FAIL** — `Failed to resolve import "./specialDateRanges"`.

- [ ] **Bước 3: Viết module**

Tạo `src/lib/specialDateRanges.ts`:

```ts
/**
 * Dưới DB, `special_dates` là một dòng cho một ngày. Chủ nhà thì nghĩ theo kỳ
 * nghỉ. Module này bắc cầu giữa hai cách nhìn ấy, và chỉ làm việc đó.
 */

export type SpecialDateRow = {
    id: string;
    date: string;
    label: string;
    uplift_pct: number;
};

export type SpecialDateRange = {
    from: string;
    to: string;
    days: number;
    label: string;
    uplift_pct: number;
    /** Mọi ngày trong cụm — thứ phải gửi đi khi xoá. */
    dates: string[];
};

const DATE_ONLY = /^\d{4}-\d{2}-\d{2}$/;

/**
 * Ngày kế tiếp của một `YYYY-MM-DD`.
 *
 * Tính trên UTC rồi cắt mười ký tự đầu. Tuyệt đối không dùng
 * `new Date("2026-02-14T00:00:00")` — nó phân tích theo giờ địa phương rồi in
 * ra UTC, và ở UTC+7 thì lệch mất một ngày.
 *
 * Hàm này cố ý để cục bộ chứ không tách sang `src/lib/`:
 * `ReservationSheet.tsx` đã có một `addDays` nhận và trả `string`, còn nhánh
 * `refactor/pricing-preview-honesty` đang thêm một `addDays` nhận và trả
 * `Date`. Dựng thêm một module dùng chung lúc này là chuốc lấy đụng độ trùng
 * tên với kiểu không tương thích.
 */
function nextDay(date: string): string {
    const [year, month, day] = date.split("-").map(Number);
    return new Date(Date.UTC(year, month - 1, day + 1)).toISOString().slice(0, 10);
}

/**
 * Gom ngày liền nhau, cùng nhãn, cùng mức thành một khoảng.
 *
 * Đây là suy đoán chứ không phải dữ liệu: hai kỳ khác nhau tình cờ liền ngày,
 * cùng nhãn, cùng mức sẽ hiện thành một dòng. Vô hại — xoá cụm ấy đúng là xoá
 * chừng đó ngày — nhưng đừng coi đó là lỗi.
 */
export function groupSpecialDates(rows: SpecialDateRow[]): SpecialDateRange[] {
    const sorted = rows
        .filter((row) => DATE_ONLY.test(row.date))
        .slice()
        .sort((left, right) => left.date.localeCompare(right.date));

    const ranges: SpecialDateRange[] = [];
    for (const row of sorted) {
        const open = ranges[ranges.length - 1];
        const joinsOpenRange =
            open !== undefined &&
            open.label === row.label &&
            open.uplift_pct === row.uplift_pct &&
            nextDay(open.to) === row.date;

        if (open !== undefined && joinsOpenRange) {
            open.to = row.date;
            open.days += 1;
            open.dates.push(row.date);
        } else {
            ranges.push({
                from: row.date,
                to: row.date,
                days: 1,
                label: row.label,
                uplift_pct: row.uplift_pct,
                dates: [row.date],
            });
        }
    }

    return ranges;
}

/**
 * Những ngày đã khai mà một khoảng mới sẽ ghi đè lên.
 *
 * `exclude` là các ngày của chính cụm đang sửa — chúng không phải là trùng.
 * So sánh bằng chuỗi vì `YYYY-MM-DD` xếp theo từ điển đúng bằng xếp theo thời
 * gian.
 */
export function overlappingDates(
    rows: SpecialDateRow[],
    from: string,
    to: string,
    exclude: string[] = [],
): SpecialDateRow[] {
    const excluded = new Set(exclude);

    return rows
        .filter(
            (row) =>
                DATE_ONLY.test(row.date) &&
                !excluded.has(row.date) &&
                row.date >= from &&
                row.date <= to,
        )
        .sort((left, right) => left.date.localeCompare(right.date));
}
```

- [ ] **Bước 4: Chạy cho xanh**

```bash
cd mhm && npx vitest run src/lib/specialDateRanges.test.ts
```

Kỳ vọng: **PASS**, 11 test, không warning.

- [ ] **Bước 5: Commit**

```bash
cd /Users/binhan/HotelManager/.worktrees/peak-season
git add mhm/src/lib/specialDateRanges.ts mhm/src/lib/specialDateRanges.test.ts
git commit -m "feat(pricing): group contiguous declared days into a season

The table stores one row per day; nobody thinks about Tet as nine separate
facts. Contiguous plus same label plus same percentage is enough to recover the
range, so the display can show one line without the schema changing."
```

---

### Task 7: Màn hình khai báo

**Files:**
- Create: `src/pages/settings/SpecialDatesSection.tsx`
- Test: `src/pages/settings/SpecialDatesSection.test.tsx`
- Modify: `src/pages/settings/index.tsx:33` (kiểu), `:68` (mục thanh bên), `:132` (dòng render), và khối `import`
- Modify: `src/__mocks__/tauri-core.ts` (thêm vào `defaults`)
- Modify: `tests/frontend-invoke-wrapper-guardrails.test.ts` (cho phép `get_special_dates`)

**Interfaces:**
- Consumes: `groupSpecialDates`, `overlappingDates`, `SpecialDateRow`, `SpecialDateRange` từ Task 6; lệnh `save_special_date_range`, `delete_special_dates` từ Task 4 và 5
- Produces: `export default function SpecialDatesSection()`

- [ ] **Bước 1: Mở đường cho hạ tầng test trước, nếu không sẽ đỏ vì lý do sai**

`src/__mocks__/tauri-core.ts` **không cần sửa**: `get_special_dates: []` đã có sẵn ở `:117`, và hai lệnh ghi mới không cần mặc định vì test dưới đây mock thẳng `@/lib/invokeCommand` — đúng lối `PricingSection.test.tsx` đang làm. Đừng thêm gì vào file ấy.

Chỉ sửa `tests/frontend-invoke-wrapper-guardrails.test.ts`: thêm vào `RAW_INVOKE_ALLOWED_COMMANDS`, giữ đúng thứ tự chữ cái:

```ts
  get_special_dates: "read-only peak-season lookup",
```

Test ở `:260-290` đỏ với bất kỳ `invoke` trần nào không có trong danh sách này, và hôm nay `get_special_dates` chưa có vì chưa ai gọi.

- [ ] **Bước 2: Viết test đỏ cho component**

Tạo `src/pages/settings/SpecialDatesSection.test.tsx`.

Lối mock dưới đây là chép từ `PricingSection.test.tsx` — `vi.hoisted` rồi `vi.mock` cho ba module. File ấy còn thay các nguyên hàm shadcn (`Button`, `Input`, `Label`) bằng thẻ HTML trần. **Nếu** một khẳng định dưới đây tìm không ra nút hay ô nhập vì component thật cư xử lạ trong jsdom, chép luôn khối thay nguyên hàm ấy sang; đừng nới lỏng khẳng định.

```tsx
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import SpecialDatesSection from "./SpecialDatesSection";

const invokeWriteCommand = vi.hoisted(() => vi.fn());
vi.mock("@/lib/invokeCommand", () => ({ invokeWriteCommand }));

const invoke = vi.hoisted(() => vi.fn());
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

function tetRows() {
    return Array.from({ length: 9 }, (_, index) => ({
        id: `id-${index}`,
        date: `2026-02-${14 + index}`,
        label: "Tết Nguyên đán",
        uplift_pct: 40,
    }));
}

describe("SpecialDatesSection", () => {
    beforeEach(() => {
        invokeWriteCommand.mockReset().mockResolvedValue(undefined);
        invoke.mockReset().mockResolvedValue(tetRows());
    });

    it("hiện chín ngày Tết thành một dòng", async () => {
        render(<SpecialDatesSection />);

        expect(await screen.findByText("Tết Nguyên đán")).toBeInTheDocument();
        expect(screen.getByText(/9 ngày/)).toBeInTheDocument();
        expect(screen.getAllByRole("button", { name: "Xoá" })).toHaveLength(1);
    });

    it("khai khoảng không trùng thì gọi thẳng lệnh ghi", async () => {
        invoke.mockResolvedValue([]);
        render(<SpecialDatesSection />);
        await screen.findByText(/Chưa khai đợt cao điểm nào/);

        fireEvent.change(screen.getByLabelText("Tên đợt"), { target: { value: "Lễ 30/4" } });
        fireEvent.change(screen.getByLabelText("Từ ngày"), { target: { value: "2026-04-30" } });
        fireEvent.change(screen.getByLabelText("Đến ngày"), { target: { value: "2026-05-03" } });
        fireEvent.change(screen.getByLabelText("% phụ thu"), { target: { value: "30" } });
        fireEvent.click(screen.getByRole("button", { name: "Thêm" }));

        await waitFor(() => expect(invokeWriteCommand).toHaveBeenCalledTimes(1));
        expect(invokeWriteCommand).toHaveBeenCalledWith("save_special_date_range", {
            remove: [],
            from: "2026-04-30",
            to: "2026-05-03",
            label: "Lễ 30/4",
            upliftPct: 30,
        });
    });

    it("khai đè lên ngày đã có thì hỏi trước, và huỷ thì không ghi gì", async () => {
        render(<SpecialDatesSection />);
        await screen.findByText("Tết Nguyên đán");

        fireEvent.change(screen.getByLabelText("Tên đợt"), { target: { value: "Hè" } });
        fireEvent.change(screen.getByLabelText("Từ ngày"), { target: { value: "2026-02-20" } });
        fireEvent.change(screen.getByLabelText("Đến ngày"), { target: { value: "2026-02-28" } });
        fireEvent.change(screen.getByLabelText("% phụ thu"), { target: { value: "25" } });
        fireEvent.click(screen.getByRole("button", { name: "Thêm" }));

        expect(await screen.findByText(/3 ngày đã khai sẽ bị ghi đè/)).toBeInTheDocument();
        expect(screen.getByText(/2026-02-20/)).toBeInTheDocument();

        fireEvent.click(screen.getByRole("button", { name: "Huỷ" }));

        expect(invokeWriteCommand).not.toHaveBeenCalled();
    });

    it("bấm tiếp tục ở hộp trùng thì mới ghi", async () => {
        render(<SpecialDatesSection />);
        await screen.findByText("Tết Nguyên đán");

        fireEvent.change(screen.getByLabelText("Tên đợt"), { target: { value: "Hè" } });
        fireEvent.change(screen.getByLabelText("Từ ngày"), { target: { value: "2026-02-20" } });
        fireEvent.change(screen.getByLabelText("Đến ngày"), { target: { value: "2026-02-28" } });
        fireEvent.change(screen.getByLabelText("% phụ thu"), { target: { value: "25" } });
        fireEvent.click(screen.getByRole("button", { name: "Thêm" }));
        fireEvent.click(await screen.findByRole("button", { name: "Tiếp tục" }));

        await waitFor(() => expect(invokeWriteCommand).toHaveBeenCalledTimes(1));
    });

    it("xoá một cụm gửi đúng chín ngày trong một lần gọi", async () => {
        render(<SpecialDatesSection />);
        await screen.findByText("Tết Nguyên đán");

        fireEvent.click(screen.getByRole("button", { name: "Xoá" }));
        fireEvent.click(await screen.findByRole("button", { name: "Xoá đợt này" }));

        await waitFor(() => expect(invokeWriteCommand).toHaveBeenCalledTimes(1));
        const [command, args] = invokeWriteCommand.mock.calls[0];
        expect(command).toBe("delete_special_dates");
        expect(args.dates).toHaveLength(9);
    });

    it("sửa cụm cho ngắn lại thì ngày rơi ra đi trong `remove`, không có lệnh xoá riêng", async () => {
        render(<SpecialDatesSection />);
        await screen.findByText("Tết Nguyên đán");

        fireEvent.click(screen.getByRole("button", { name: "Sửa" }));
        fireEvent.change(screen.getByLabelText("Đến ngày"), { target: { value: "2026-02-19" } });
        fireEvent.click(screen.getByRole("button", { name: "Cập nhật" }));

        await waitFor(() => expect(invokeWriteCommand).toHaveBeenCalledTimes(1));
        const [command, args] = invokeWriteCommand.mock.calls[0];
        expect(command).toBe("save_special_date_range");
        // 20, 21, 22 rơi ra khỏi khoảng mới 14–19.
        expect(args.remove).toEqual(["2026-02-20", "2026-02-21", "2026-02-22"]);
        expect(
            invokeWriteCommand.mock.calls.some(([name]) => name === "delete_special_dates"),
        ).toBe(false);
    });

    it("lệnh lỗi thì báo toast và không đổi danh sách ngầm", async () => {
        const { toast } = await import("sonner");
        invokeWriteCommand.mockRejectedValue(new Error("Không đủ quyền"));
        render(<SpecialDatesSection />);
        await screen.findByText("Tết Nguyên đán");

        fireEvent.click(screen.getByRole("button", { name: "Xoá" }));
        fireEvent.click(await screen.findByRole("button", { name: "Xoá đợt này" }));

        await waitFor(() => expect(toast.error).toHaveBeenCalled());
        expect(screen.getByText("Tết Nguyên đán")).toBeInTheDocument();
    });
});
```

- [ ] **Bước 3: Chạy cho thấy đỏ**

```bash
cd mhm && npx vitest run src/pages/settings/SpecialDatesSection.test.tsx
```

Kỳ vọng: **FAIL** — `Failed to resolve import "./SpecialDatesSection"`.

- [ ] **Bước 4: Viết component**

Tạo `src/pages/settings/SpecialDatesSection.tsx`:

```tsx
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { CalendarDays } from "lucide-react";
import { toast } from "sonner";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { invokeWriteCommand } from "@/lib/invokeCommand";
import {
    groupSpecialDates,
    overlappingDates,
    type SpecialDateRange,
    type SpecialDateRow,
} from "@/lib/specialDateRanges";

/** Quá ngần này thì liệt kê hết chỉ tổ rối; phần còn lại đếm số. */
const MAX_LISTED_CLASHES = 10;

type PendingWrite = {
    remove: string[];
    from: string;
    to: string;
    label: string;
    upliftPct: number;
};

export default function SpecialDatesSection() {
    const [rows, setRows] = useState<SpecialDateRow[]>([]);
    const [editing, setEditing] = useState<SpecialDateRange | null>(null);
    const [label, setLabel] = useState("");
    const [from, setFrom] = useState("");
    const [to, setTo] = useState("");
    const [upliftPct, setUpliftPct] = useState("30");
    const [clashes, setClashes] = useState<SpecialDateRow[] | null>(null);
    const [pending, setPending] = useState<PendingWrite | null>(null);
    const [deleting, setDeleting] = useState<SpecialDateRange | null>(null);

    const reload = useCallback(() => {
        invoke<SpecialDateRow[]>("get_special_dates")
            .then(setRows)
            .catch(() => setRows([]));
    }, []);

    useEffect(reload, [reload]);

    const ranges = groupSpecialDates(rows);

    const resetForm = () => {
        setEditing(null);
        setLabel("");
        setFrom("");
        setTo("");
        setUpliftPct("30");
    };

    const startEdit = (range: SpecialDateRange) => {
        setEditing(range);
        setLabel(range.label);
        setFrom(range.from);
        setTo(range.to);
        setUpliftPct(String(range.uplift_pct));
    };

    const write = async (request: PendingWrite) => {
        try {
            await invokeWriteCommand("save_special_date_range", request);
            toast.success("Đã lưu đợt cao điểm");
            resetForm();
            reload();
        } catch (error) {
            toast.error(error instanceof Error ? error.message : String(error));
        }
    };

    const handleSave = () => {
        const trimmed = label.trim();
        if (!trimmed || !from || !to || to < from) {
            toast.error("Điền tên đợt và khoảng ngày hợp lệ");
            return;
        }

        // Ngày rơi ra khỏi khoảng mới phải bị xoá thật, nếu không nó nằm lại
        // thành một ngày lễ mồ côi. Chúng đi kèm lệnh ghi chứ không thành một
        // lệnh xoá riêng, để cả hai nửa chung một transaction.
        const remove = (editing?.dates ?? []).filter((date) => date < from || date > to);
        const request: PendingWrite = {
            remove,
            from,
            to,
            label: trimmed,
            upliftPct: Number(upliftPct),
        };

        const conflicts = overlappingDates(rows, from, to, editing?.dates ?? []);
        if (conflicts.length > 0) {
            setClashes(conflicts);
            setPending(request);
            return;
        }

        void write(request);
    };

    const handleDelete = async (range: SpecialDateRange) => {
        try {
            await invokeWriteCommand("delete_special_dates", { dates: range.dates });
            toast.success("Đã xoá đợt cao điểm");
            resetForm();
            reload();
        } catch (error) {
            toast.error(error instanceof Error ? error.message : String(error));
        } finally {
            setDeleting(null);
        }
    };

    return (
        <div className="space-y-6">
            <div>
                <h3 className="text-lg font-bold mb-1 flex items-center gap-2">
                    <CalendarDays size={20} className="text-emerald-500" />
                    Mùa cao điểm
                </h3>
                <p className="text-sm text-brand-muted">
                    Khai những đợt tăng giá theo ngày — Tết, lễ, mùa du lịch. Giá phòng tự cộng
                    thêm cho đúng những đêm nằm trong đợt.
                </p>
            </div>

            {ranges.length === 0 ? (
                <p className="text-sm text-brand-muted">
                    Chưa khai đợt cao điểm nào. Thêm một đợt ở dưới.
                </p>
            ) : (
                <div className="space-y-2">
                    {ranges.map((range) => (
                        <div
                            key={`${range.from}-${range.label}`}
                            className="flex items-center justify-between p-4 bg-slate-50 rounded-xl"
                        >
                            <div>
                                <p className="font-semibold text-sm">{range.label}</p>
                                <p className="text-xs text-brand-muted">
                                    {range.from} – {range.to} ({range.days} ngày) &nbsp;|&nbsp; +
                                    {range.uplift_pct}%
                                </p>
                            </div>
                            <div className="flex gap-2">
                                <Button
                                    variant="outline"
                                    size="sm"
                                    className="rounded-lg"
                                    onClick={() => startEdit(range)}
                                >
                                    Sửa
                                </Button>
                                <Button
                                    variant="outline"
                                    size="sm"
                                    className="rounded-lg text-red-600"
                                    onClick={() => setDeleting(range)}
                                >
                                    Xoá
                                </Button>
                            </div>
                        </div>
                    ))}
                </div>
            )}

            <div className="p-5 bg-slate-50 rounded-2xl space-y-4">
                <h4 className="font-bold text-sm">
                    {editing ? `Sửa: ${editing.label}` : "Thêm đợt cao điểm"}
                </h4>
                <div className="grid grid-cols-2 gap-3">
                    <div>
                        <Label htmlFor="special-label">Tên đợt</Label>
                        <Input
                            id="special-label"
                            value={label}
                            onChange={(event) => setLabel(event.target.value)}
                            placeholder="Tết Nguyên đán"
                            className="mt-1.5"
                        />
                    </div>
                    <div>
                        <Label htmlFor="special-uplift">% phụ thu</Label>
                        <Input
                            id="special-uplift"
                            type="number"
                            min={0}
                            max={500}
                            value={upliftPct}
                            onChange={(event) => setUpliftPct(event.target.value)}
                            className="mt-1.5 w-24"
                        />
                    </div>
                    <div>
                        <Label htmlFor="special-from">Từ ngày</Label>
                        <Input
                            id="special-from"
                            type="date"
                            value={from}
                            onChange={(event) => setFrom(event.target.value)}
                            className="mt-1.5"
                        />
                    </div>
                    <div>
                        <Label htmlFor="special-to">Đến ngày</Label>
                        <Input
                            id="special-to"
                            type="date"
                            min={from || undefined}
                            value={to}
                            onChange={(event) => setTo(event.target.value)}
                            className="mt-1.5"
                        />
                    </div>
                </div>
                <div className="flex gap-2">
                    <Button
                        onClick={handleSave}
                        className="bg-brand-primary text-white rounded-xl"
                    >
                        {editing ? "Cập nhật" : "Thêm"}
                    </Button>
                    {editing && (
                        <Button variant="outline" className="rounded-xl" onClick={resetForm}>
                            Huỷ sửa
                        </Button>
                    )}
                </div>
            </div>

            {clashes && pending && (
                <div className="p-5 border border-amber-300 bg-amber-50 rounded-2xl space-y-3">
                    <p className="text-sm font-semibold">
                        {clashes.length} ngày đã khai sẽ bị ghi đè
                    </p>
                    <ul className="text-xs text-brand-muted space-y-0.5">
                        {clashes.slice(0, MAX_LISTED_CLASHES).map((clash) => (
                            <li key={clash.date}>
                                {clash.date} — {clash.label} +{clash.uplift_pct}% → {pending.label} +
                                {pending.upliftPct}%
                            </li>
                        ))}
                        {clashes.length > MAX_LISTED_CLASHES && (
                            <li>…và {clashes.length - MAX_LISTED_CLASHES} ngày nữa</li>
                        )}
                    </ul>
                    <div className="flex gap-2">
                        <Button
                            className="bg-brand-primary text-white rounded-xl"
                            onClick={() => {
                                const request = pending;
                                setClashes(null);
                                setPending(null);
                                void write(request);
                            }}
                        >
                            Tiếp tục
                        </Button>
                        <Button
                            variant="outline"
                            className="rounded-xl"
                            onClick={() => {
                                setClashes(null);
                                setPending(null);
                            }}
                        >
                            Huỷ
                        </Button>
                    </div>
                </div>
            )}

            {deleting && (
                <div className="p-5 border border-red-300 bg-red-50 rounded-2xl space-y-3">
                    <p className="text-sm font-semibold">
                        Xoá &quot;{deleting.label}&quot; ({deleting.days} ngày)?
                    </p>
                    <div className="flex gap-2">
                        <Button
                            className="bg-red-600 text-white rounded-xl"
                            onClick={() => void handleDelete(deleting)}
                        >
                            Xoá đợt này
                        </Button>
                        <Button
                            variant="outline"
                            className="rounded-xl"
                            onClick={() => setDeleting(null)}
                        >
                            Giữ lại
                        </Button>
                    </div>
                </div>
            )}
        </div>
    );
}
```

- [ ] **Bước 5: Chạy cho xanh**

```bash
cd mhm && npx vitest run src/pages/settings/SpecialDatesSection.test.tsx
```

Kỳ vọng: **PASS**, 7 test. Nếu một test đỏ vì tìm không ra nhãn hoặc nút, sửa **test hay component cho khớp nhau**, đừng nới lỏng khẳng định.

- [ ] **Bước 6: Đấu vào thanh bên Cài đặt**

Trong `src/pages/settings/index.tsx`:

Thêm `CalendarDays` vào khối `import { … } from "lucide-react"` (giữ thứ tự chữ cái).

Thêm import:
```tsx
import SpecialDatesSection from "./SpecialDatesSection";
```

Trong `type SettingsSectionKey`, thêm một nhánh:
```tsx
  | "peak-season"
```

Trong mảng `sections`, khối `isCurrentAdmin` (`:68-73`), thêm ngay dưới dòng `pricing`:
```tsx
        { key: "peak-season" as const, label: "Peak Season", icon: CalendarDays },
```

Trong phần render, ngay dưới dòng `{activeSection === "pricing" && …}` (`:132`):
```tsx
        {activeSection === "peak-season" && isCurrentAdmin && <SpecialDatesSection />}
```

- [ ] **Bước 7: Chạy toàn bộ cổng**

```bash
cd mhm && npm run verify:full 2>&1 | tail -30
```

Kỳ vọng: **PASS** toàn bộ, kể cả `frontend-invoke-wrapper-guardrails`. Nếu nó đỏ ở `get_special_dates`, xem lại Bước 1.

- [ ] **Bước 8: Commit**

```bash
cd /Users/binhan/HotelManager/.worktrees/peak-season
git add mhm/src/pages/settings/SpecialDatesSection.tsx mhm/src/pages/settings/SpecialDatesSection.test.tsx mhm/src/pages/settings/index.tsx mhm/src/__mocks__/tauri-core.ts mhm/tests/frontend-invoke-wrapper-guardrails.test.ts
git commit -m "feat(settings): a screen for declaring peak seasons

The special_dates table has existed since the first migration with a backend
behind it and nothing in the UI that reached it — the only reference in src/ was
a line in the test mock. Declaring a season by range now works, overwrites ask
first instead of happening silently, and a mistyped season can be deleted."
```

---

## Cổng cuối cùng

Chạy sau khi cả bảy task xong, trước khi báo hoàn thành:

```bash
cd mhm/src-tauri && cargo fmt --check && cargo check --all-targets && cargo test -p capyinn 2>&1 | tail -5
```

```bash
cd mhm && npm run verify:full 2>&1 | tail -20
```

Cả ba lệnh phải xanh. `cargo fmt --check` là cái hay bị quên nhất: CI gác nó nhưng `verify:full` không chạy.
