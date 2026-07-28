# Đặt phòng trước — Giá theo số khách — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Sheet "Đặt phòng trước" bấm được lịch cho ngày đi, bỏ ô Số đêm, và tính tiền phòng theo số khách qua `rooms.extra_person_fee` thay vì để người dùng bó tay với giá cứng.

**Architecture:** Engine tính giá vẫn là nơi duy nhất quyết định một kỳ lưu trú giá bao nhiêu. Số khách được lưu trên booking (`bookings.guests`), luồn qua `calculate_stay_price_tx` vào `StayPricingInputs`, và biến thành một dòng phụ thu phẳng theo đêm trong `PricingResult.breakdown`. Không có cột giá đè, không có nhánh nào bỏ qua engine.

**Tech Stack:** Rust + Tauri 2 + sqlx/SQLite (backend), React + TypeScript + Vite + Vitest (frontend), `cargo test` cho backend.

**Spec:** `docs/superpowers/specs/2026-07-28-dat-phong-truoc-gia-theo-so-khach-design.md`

## Global Constraints

- **Nhánh làm việc:** `feature/reservation-guest-pricing`, worktree `/Users/binhan/HotelManager/.worktrees/reservation-guest-pricing`. Mọi đường dẫn dưới đây tính từ gốc worktree.
- **Tiền tệ là số nguyên VNĐ.** Kiểu `MoneyVnd = i64`. Cấm `f64` cho mọi giá trị tiền — `scripts/verify/no-float-money.mjs` quét theo tên biến và sẽ chặn.
- **`bookings.guests` là số đếm, KHÔNG phải cột tiền.** Không đăng ký nó vào `MONEY_COLUMNS` / `MONEY_TABLES` trong `mhm/src-tauri/src/money_migration.rs`, cũng không thêm vào danh sách trong `scripts/verify/no-float-money.mjs`.
- **Phụ thu thêm người không bị nhân %.** Nó nằm ngoài `base_amount`, nên `weekend_uplift_pct` và uplift ngày lễ không chạm tới.
- **Booking cũ không được đổi giá.** `guests` rỗng ⇒ phụ thu bằng 0 ⇒ tổng đúng bằng kết quả trước khi làm việc này.
- **Số khách không có trần.** `rooms.max_guests` là mốc tính giá base, không phải sức chứa.
- **Rust build phải sạch dưới `-D warnings`.** Dự án bật cảnh báo thành lỗi.
- Chạy lệnh Rust từ `mhm/src-tauri/`, lệnh npm từ `mhm/`.

---

### Task 1: Engine biết tính phụ thu thêm người

Đây là task nền. Nó đổi chữ ký `calculate_stay_price_tx`, nên cả sáu nơi gọi phải sửa theo trong cùng task này để cây build còn xanh — nhưng tất cả đều truyền `None`, tức **không đổi hành vi**. Giá trị thật đến ở Task 2–5.

**Files:**
- Modify: `mhm/src-tauri/src/domain/booking/pricing.rs`
- Modify: `mhm/src-tauri/src/queries/booking/pricing_queries.rs`
- Modify: `mhm/src-tauri/src/services/booking/pricing_service.rs:26-50`
- Modify: `mhm/src-tauri/src/services/booking/reservation_lifecycle.rs:240,609,758`
- Modify: `mhm/src-tauri/src/services/booking/stay_lifecycle.rs:318,680,1074`
- Modify: `mhm/src-tauri/src/services/booking/group_lifecycle.rs:441`
- Modify: `mhm/src-tauri/src/commands/pricing.rs:96-116`
- Test: `mhm/src-tauri/src/domain/booking/pricing.rs` (khối `#[cfg(test)] mod tests` sẵn có)

**Interfaces:**
- Consumes: không có (task đầu).
- Produces:
  - `StayPricingInputs` thêm ba trường: `guests: Option<i32>`, `base_guests: i32`, `extra_person_fee: MoneyVnd`
  - `pricing_service::calculate_stay_price_tx(tx, room_id: &str, check_in: &str, check_out: &str, pricing_type: &str, guests: Option<i32>) -> BookingResult<PricingResult>`
  - `pricing_service::calculate_price_preview(pool, room_type: &str, check_in: &str, check_out: &str, pricing_type: &str, guests: Option<i32>) -> BookingResult<PricingResult>`
  - `queries::booking::pricing_queries::load_stay_pricing_inputs_tx(tx, room_id, check_in, check_out, pricing_type, guests: Option<i32>)`
  - `queries::booking::pricing_queries::load_stay_pricing_inputs_for_room_type(pool, room_type, check_in, check_out, pricing_type, guests: Option<i32>)`

- [ ] **Step 1: Viết test thất bại cho phần tính phụ thu**

Trong `mhm/src-tauri/src/domain/booking/pricing.rs`, sửa `sample_inputs()` trong `mod tests` để có ba trường mới, rồi thêm bốn test.

Sửa helper sẵn có (khoảng dòng 101):

```rust
    fn sample_inputs() -> StayPricingInputs {
        StayPricingInputs {
            room_type: "standard".to_string(),
            stored_rule: None,
            fallback_base_price: None,
            special_uplift_pct: 0.0,
            check_in: "2026-04-20".to_string(),
            check_out: "2026-04-22".to_string(),
            pricing_type: "nightly".to_string(),
            guests: None,
            base_guests: 2,
            extra_person_fee: 0,
        }
    }
```

Thêm vào cuối `mod tests`:

```rust
    /// 500.000₫ × 2 đêm = 1.000.000₫, cộng 2 khách vượt mốc × 50.000₫ × 2 đêm.
    /// Đây đúng con số người dùng mô tả: 600.000₫/đêm cho 4 khách.
    #[test]
    fn calculate_from_loaded_inputs_adds_flat_extra_guest_line() {
        let mut inputs = sample_inputs();
        inputs.fallback_base_price = Some(500_000);
        inputs.guests = Some(4);
        inputs.base_guests = 2;
        inputs.extra_person_fee = 50_000;

        let pricing = calculate_from_loaded_inputs(&inputs).unwrap();

        assert_eq!(pricing.base_amount, 1_000_000);
        assert_eq!(pricing.total, 1_200_000);
        assert_eq!(pricing.breakdown.len(), 2);
        assert_eq!(pricing.breakdown[1].label, "Phụ thu 2 khách");
        assert_eq!(pricing.breakdown[1].amount, 200_000);
    }

    #[test]
    fn calculate_from_loaded_inputs_charges_nothing_within_base_guests() {
        let mut inputs = sample_inputs();
        inputs.fallback_base_price = Some(500_000);
        inputs.guests = Some(2);
        inputs.base_guests = 2;
        inputs.extra_person_fee = 50_000;

        let pricing = calculate_from_loaded_inputs(&inputs).unwrap();

        assert_eq!(pricing.total, 1_000_000);
        assert_eq!(pricing.breakdown.len(), 1);
    }

    /// Booking cũ: cột `guests` rỗng ⇒ giá y hệt trước khi có tính năng này.
    /// Phòng chưa khai phụ thu ⇒ số khách bao nhiêu cũng không đổi giá.
    #[test]
    fn calculate_from_loaded_inputs_charges_nothing_when_fee_is_zero() {
        let mut inputs = sample_inputs();
        inputs.fallback_base_price = Some(500_000);
        inputs.guests = Some(6);
        inputs.base_guests = 2;
        inputs.extra_person_fee = 0;

        let pricing = calculate_from_loaded_inputs(&inputs).unwrap();

        assert_eq!(pricing.total, 1_000_000);
        assert_eq!(pricing.breakdown.len(), 1);
    }

    #[test]
    fn calculate_from_loaded_inputs_ignores_extra_fee_when_guests_unknown() {
        let mut inputs = sample_inputs();
        inputs.fallback_base_price = Some(500_000);
        inputs.guests = None;
        inputs.extra_person_fee = 50_000;

        let pricing = calculate_from_loaded_inputs(&inputs).unwrap();

        assert_eq!(pricing.total, 1_000_000);
        assert_eq!(pricing.breakdown.len(), 1);
    }

    /// Phụ thu là khoản phẳng: uplift ngày lễ chỉ ăn vào `base_amount`,
    /// không nhân lên phần thêm người.
    #[test]
    fn extra_guest_line_is_not_multiplied_by_uplift() {
        let mut inputs = sample_inputs();
        inputs.fallback_base_price = Some(500_000);
        inputs.special_uplift_pct = 10.0;
        inputs.guests = Some(3);
        inputs.base_guests = 2;
        inputs.extra_person_fee = 50_000;

        let pricing = calculate_from_loaded_inputs(&inputs).unwrap();

        assert_eq!(pricing.base_amount, 1_000_000);
        assert_eq!(pricing.surcharge_amount, 100_000);
        assert_eq!(pricing.total, 1_200_000);
        assert_eq!(pricing.breakdown[2].amount, 100_000);
    }
```

- [ ] **Step 2: Chạy test để xác nhận nó hỏng**

Chạy từ `mhm/src-tauri/`:

```bash
cargo test --lib domain::booking::pricing
```

Kỳ vọng: **FAIL** khi biên dịch, báo `struct StayPricingInputs has no field named guests`.

- [ ] **Step 3: Thêm ba trường vào `StayPricingInputs`**

Trong `mhm/src-tauri/src/domain/booking/pricing.rs`, sửa struct ở dòng 14:

```rust
/// Everything the pricing rules need, already read out of the database.
#[derive(Debug, Clone)]
pub(crate) struct StayPricingInputs {
    pub(crate) room_type: String,
    pub(crate) stored_rule: Option<StoredPricingRule>,
    pub(crate) fallback_base_price: Option<MoneyVnd>,
    pub(crate) special_uplift_pct: f64,
    pub(crate) check_in: String,
    pub(crate) check_out: String,
    pub(crate) pricing_type: String,
    /// Số khách trên booking. `None` là booking chưa khai số khách — booking cũ,
    /// hoặc một nơi gọi không quan tâm — và có nghĩa là không phụ thu.
    pub(crate) guests: Option<i32>,
    /// `rooms.max_guests`: số khách đã nằm trong giá base.
    pub(crate) base_guests: i32,
    /// `rooms.extra_person_fee`: phụ thu mỗi khách vượt mốc, mỗi đêm.
    pub(crate) extra_person_fee: MoneyVnd,
}
```

- [ ] **Step 4: Viết phần tính phụ thu**

Trong cùng file, thêm `use chrono::NaiveDate;` vào đầu file (cạnh `use super::{BookingError, BookingResult};`), rồi thay hàm `calculate_from_loaded_inputs` ở dòng 78 và thêm hai hàm phụ ngay trên nó:

```rust
/// Số đêm giữa hai mốc, chấp nhận cả `YYYY-MM-DD` lẫn RFC3339 (cắt 10 ký tự đầu).
/// Trả 0 khi ngày đi không sau ngày đến — đúng lúc `calculate_price` cũng trả 0.
fn nights_between(check_in: &str, check_out: &str) -> BookingResult<i64> {
    let parse = |value: &str| {
        let head = &value[..value.len().min(10)];
        NaiveDate::parse_from_str(head, "%Y-%m-%d")
            .map_err(|error| BookingError::datetime_parse(error.to_string()))
    };
    Ok((parse(check_out)? - parse(check_in)?).num_days().max(0))
}

/// Phụ thu thêm người: khoản phẳng theo đêm.
///
/// Cố ý **không** đi qua `percentage_money_line` và cố ý nằm ngoài `base_amount`,
/// nên uplift cuối tuần / ngày lễ không nhân lên nó. Thêm một người là thêm một
/// khoản cố định, giải thích với khách được và dò ngược trên hoá đơn được.
fn extra_guest_charge(inputs: &StayPricingInputs, nights: i64) -> BookingResult<(i64, MoneyVnd)> {
    let Some(guests) = inputs.guests else {
        return Ok((0, 0));
    };
    let extra_guests = i64::from(guests.saturating_sub(inputs.base_guests)).max(0);
    if extra_guests == 0 || inputs.extra_person_fee <= 0 || nights <= 0 {
        return Ok((0, 0));
    }

    let amount = inputs
        .extra_person_fee
        .checked_mul(extra_guests)
        .and_then(|per_night| per_night.checked_mul(nights))
        .ok_or_else(|| {
            BookingError::validation("extra guest charge overflowed MoneyVnd".to_string())
        })?;

    Ok((extra_guests, amount))
}

pub(crate) fn calculate_from_loaded_inputs(
    inputs: &StayPricingInputs,
) -> BookingResult<crate::pricing::PricingResult> {
    let rule = build_effective_pricing_rule(inputs);

    let mut result = crate::pricing::calculate_price(
        &rule,
        &inputs.check_in,
        &inputs.check_out,
        &inputs.pricing_type,
        inputs.special_uplift_pct,
    )
    .map_err(BookingError::datetime_parse)?;

    let nights = nights_between(&inputs.check_in, &inputs.check_out)?;
    let (extra_guests, extra_amount) = extra_guest_charge(inputs, nights)?;
    if extra_amount > 0 {
        result.breakdown.push(crate::pricing::PricingLine {
            label: format!("Phụ thu {} khách", extra_guests),
            amount: extra_amount,
        });
        result.total = result.total.checked_add(extra_amount).ok_or_else(|| {
            BookingError::validation("stay total overflowed MoneyVnd".to_string())
        })?;
    }

    Ok(result)
}
```

- [ ] **Step 5: Luồn `guests` qua tầng nạp dữ liệu**

Trong `mhm/src-tauri/src/queries/booking/pricing_queries.rs`, thêm câu lệnh và hàm đọc thông tin khách của phòng. Đặt hằng cạnh `FALLBACK_BASE_PRICE_SQL` (dòng 25):

```rust
const ROOM_GUEST_PRICING_SQL: &str =
    "SELECT COALESCE(max_guests, 2) AS max_guests,
            COALESCE(extra_person_fee, 0) AS extra_person_fee
     FROM rooms WHERE id = ?";

const ROOM_GUEST_PRICING_BY_TYPE_SQL: &str =
    "SELECT COALESCE(max_guests, 2) AS max_guests,
            COALESCE(extra_person_fee, 0) AS extra_person_fee
     FROM rooms WHERE LOWER(type) = ? LIMIT 1";

/// Mốc tính giá của một phòng. Phòng không tìm thấy thì dùng mặc định của schema
/// (`max_guests` mặc định 2, `extra_person_fee` mặc định 0) — tức không phụ thu.
struct RoomGuestPricing {
    base_guests: i32,
    extra_person_fee: MoneyVnd,
}

impl Default for RoomGuestPricing {
    fn default() -> Self {
        Self {
            base_guests: 2,
            extra_person_fee: 0,
        }
    }
}

fn room_guest_pricing_from_row(row: &sqlx::sqlite::SqliteRow) -> RoomGuestPricing {
    RoomGuestPricing {
        base_guests: row.get("max_guests"),
        extra_person_fee: get_money_vnd(row, "extra_person_fee"),
    }
}

async fn load_room_guest_pricing_tx(
    tx: &mut Transaction<'_, Sqlite>,
    room_id: &str,
) -> BookingResult<RoomGuestPricing> {
    let row = sqlx::query(ROOM_GUEST_PRICING_SQL)
        .bind(room_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_error)?;

    Ok(row
        .as_ref()
        .map(room_guest_pricing_from_row)
        .unwrap_or_default())
}

async fn load_room_guest_pricing_for_type(
    pool: &Pool<Sqlite>,
    room_type: &str,
) -> BookingResult<RoomGuestPricing> {
    let row = sqlx::query(ROOM_GUEST_PRICING_BY_TYPE_SQL)
        .bind(room_type.to_lowercase())
        .fetch_optional(pool)
        .await
        .map_err(database_error)?;

    Ok(row
        .as_ref()
        .map(room_guest_pricing_from_row)
        .unwrap_or_default())
}
```

Sửa `load_stay_pricing_inputs_tx` (dòng 88) — thêm tham số `guests` và ba trường mới:

```rust
pub(crate) async fn load_stay_pricing_inputs_tx(
    tx: &mut Transaction<'_, Sqlite>,
    room_id: &str,
    check_in: &str,
    check_out: &str,
    pricing_type: &str,
    guests: Option<i32>,
) -> BookingResult<StayPricingInputs> {
    let room_type = load_room_type_tx(tx, room_id).await?;
    let stored_rule = load_stored_pricing_rule_tx(tx, &room_type).await?;
    let fallback_base_price = if stored_rule.is_none() {
        load_fallback_base_price_tx(tx, &room_type).await?
    } else {
        None
    };
    let special_uplift_pct = load_special_uplift_tx(tx, check_in).await?;
    let guest_pricing = load_room_guest_pricing_tx(tx, room_id).await?;

    Ok(StayPricingInputs {
        room_type,
        stored_rule,
        fallback_base_price,
        special_uplift_pct,
        check_in: check_in.to_string(),
        check_out: check_out.to_string(),
        pricing_type: pricing_type.to_string(),
        guests,
        base_guests: guest_pricing.base_guests,
        extra_person_fee: guest_pricing.extra_person_fee,
    })
}
```

Sửa `load_stay_pricing_inputs_for_room_type` (dòng 123) y hệt, chỉ khác nguồn:

```rust
pub(crate) async fn load_stay_pricing_inputs_for_room_type(
    pool: &Pool<Sqlite>,
    room_type: &str,
    check_in: &str,
    check_out: &str,
    pricing_type: &str,
    guests: Option<i32>,
) -> BookingResult<StayPricingInputs> {
    let stored_rule = load_stored_pricing_rule(pool, room_type).await?;
    let fallback_base_price = if stored_rule.is_none() {
        load_fallback_base_price(pool, room_type).await?
    } else {
        None
    };
    let special_uplift_pct = load_special_uplift(pool, check_in).await.unwrap_or(0.0);
    let guest_pricing = load_room_guest_pricing_for_type(pool, room_type).await?;

    Ok(StayPricingInputs {
        room_type: room_type.to_string(),
        stored_rule,
        fallback_base_price,
        special_uplift_pct,
        check_in: check_in.to_string(),
        check_out: check_out.to_string(),
        pricing_type: pricing_type.to_string(),
        guests,
        base_guests: guest_pricing.base_guests,
        extra_person_fee: guest_pricing.extra_person_fee,
    })
}
```

Trong `mhm/src-tauri/src/services/booking/pricing_service.rs`, thêm tham số cho hai hàm (dòng 26 và 40):

```rust
pub async fn calculate_stay_price_tx(
    tx: &mut Transaction<'_, Sqlite>,
    room_id: &str,
    check_in: &str,
    check_out: &str,
    pricing_type: &str,
    guests: Option<i32>,
) -> BookingResult<crate::pricing::PricingResult> {
    let inputs =
        load_stay_pricing_inputs_tx(tx, room_id, check_in, check_out, pricing_type, guests).await?;
    calculate_from_loaded_inputs(&inputs)
}

pub async fn calculate_price_preview(
    pool: &Pool<Sqlite>,
    room_type: &str,
    check_in: &str,
    check_out: &str,
    pricing_type: &str,
    guests: Option<i32>,
) -> BookingResult<crate::pricing::PricingResult> {
    let inputs = load_stay_pricing_inputs_for_room_type(
        pool,
        room_type,
        check_in,
        check_out,
        pricing_type,
        guests,
    )
    .await?;
    calculate_from_loaded_inputs(&inputs)
}
```

- [ ] **Step 6: Cập nhật mọi nơi gọi, truyền `None`**

Thêm `None,` làm tham số cuối ở từng chỗ sau. Không đổi gì khác — hành vi giữ nguyên tuyệt đối.

| File | Dòng | Hàm chứa nó |
|---|---|---|
| `mhm/src-tauri/src/services/booking/reservation_lifecycle.rs` | 240 | `create_reservation_tx` |
| `mhm/src-tauri/src/services/booking/reservation_lifecycle.rs` | 609 | `confirm_reservation_tx` |
| `mhm/src-tauri/src/services/booking/reservation_lifecycle.rs` | 758 | `modify_reservation_tx` |
| `mhm/src-tauri/src/services/booking/stay_lifecycle.rs` | 318 | `check_in_tx` |
| `mhm/src-tauri/src/services/booking/stay_lifecycle.rs` | 680 | `preview_checkout_settlement_tx` |
| `mhm/src-tauri/src/services/booking/stay_lifecycle.rs` | 1074 | `extend_stay_tx` |
| `mhm/src-tauri/src/services/booking/group_lifecycle.rs` | 441 | check-in theo đoàn |
| `mhm/src-tauri/src/commands/pricing.rs` | 103 | `do_calculate_price_preview` |

Ví dụ, `reservation_lifecycle.rs:240` thành:

```rust
    let pricing = calculate_stay_price_tx(
        tx,
        &req.room_id,
        &req.check_in_date,
        &req.check_out_date,
        "nightly",
        None,
    )
    .await?;
```

Các test sẵn có trong `mhm/src-tauri/src/services/booking/tests/pricing.rs` gọi trực tiếp `calculate_stay_price_tx` cũng phải thêm `None`.

- [ ] **Step 7: Chạy test, kỳ vọng xanh**

```bash
cargo test --lib domain::booking::pricing
```

Kỳ vọng: **PASS**, cả bốn test mới.

```bash
cargo test --lib services::booking::tests::pricing
```

Kỳ vọng: **PASS** — mọi test giá cũ giữ nguyên con số, chứng minh truyền `None` không đổi hành vi.

- [ ] **Step 8: Commit**

```bash
git add mhm/src-tauri/src/domain/booking/pricing.rs mhm/src-tauri/src/queries/booking/pricing_queries.rs mhm/src-tauri/src/services/booking/ mhm/src-tauri/src/commands/pricing.rs
git commit -m "feat(pricing): teach the engine a flat extra-guest charge"
```

---

### Task 2: Cột `bookings.guests` và đường tạo đặt phòng

**Files:**
- Modify: `mhm/src-tauri/src/db/core_extensions.rs` (thêm `migrate_v22_booking_guest_count`)
- Modify: `mhm/src-tauri/src/db.rs:310-313` (bậc thang migration)
- Modify: `mhm/src-tauri/src/models.rs:490-501` (`CreateReservationRequest`)
- Modify: `mhm/src-tauri/src/services/booking/reservation_lifecycle.rs:209-290` (`create_reservation_tx`)
- Test: `mhm/src-tauri/src/services/booking/tests/reservations.rs`
- Test: `mhm/src-tauri/src/services/booking/tests/support/seed.rs` (thêm seeder)

**Interfaces:**
- Consumes: `calculate_stay_price_tx(..., guests: Option<i32>)` từ Task 1.
- Produces:
  - Cột `bookings.guests INTEGER` (cho phép NULL)
  - `CreateReservationRequest.guests: Option<i32>`
  - Test seeder `seed_room_with_guest_pricing(pool, room_id, base_price, max_guests, extra_person_fee)`

- [ ] **Step 1: Viết test thất bại**

Thêm seeder vào `mhm/src-tauri/src/services/booking/tests/support/seed.rs`:

```rust
/// Phòng có mốc tính giá theo số khách: `max_guests` là số khách nằm trong giá
/// base, `extra_person_fee` là phụ thu mỗi khách vượt mốc mỗi đêm.
pub async fn seed_room_with_guest_pricing(
    pool: &Pool<Sqlite>,
    room_id: &str,
    base_price: MoneyVnd,
    max_guests: i32,
    extra_person_fee: MoneyVnd,
) -> BookingResult<()> {
    sqlx::query(
        "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status)
         VALUES (?, ?, ?, ?, 0, ?, ?, ?, 'vacant')",
    )
    .bind(room_id)
    .bind(format!("Room {}", room_id))
    .bind("standard")
    .bind(1_i32)
    .bind(base_price)
    .bind(max_guests)
    .bind(extra_person_fee)
    .execute(pool)
    .await?;

    Ok(())
}
```

Thêm test vào cuối `mhm/src-tauri/src/services/booking/tests/reservations.rs`:

```rust
#[tokio::test]
async fn create_reservation_charges_extra_guests() {
    let pool = test_pool().await;
    seed_room_with_guest_pricing(&pool, "R300", 500_000, 2, 50_000)
        .await
        .unwrap();

    let mut tx = pool.begin().await.unwrap();
    let booking_id = reservation_lifecycle::create_reservation_tx(
        &mut tx,
        CreateReservationRequest {
            room_id: "R300".to_string(),
            guest_name: "Nguyễn Nhật Huy".to_string(),
            guest_phone: None,
            guest_doc_number: None,
            check_in_date: "2026-08-06".to_string(),
            check_out_date: "2026-08-08".to_string(),
            nights: 2,
            deposit_amount: None,
            source: Some("phone".to_string()),
            notes: None,
            guests: Some(4),
        },
        None,
    )
    .await
    .unwrap();

    let row = sqlx::query("SELECT total_price, guests FROM bookings WHERE id = ?")
        .bind(&booking_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();

    assert_eq!(row.get::<i64, _>("total_price"), 1_200_000);
    assert_eq!(row.get::<Option<i32>, _>("guests"), Some(4));

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn create_reservation_without_guests_prices_like_before() {
    let pool = test_pool().await;
    seed_room_with_guest_pricing(&pool, "R301", 500_000, 2, 50_000)
        .await
        .unwrap();

    let mut tx = pool.begin().await.unwrap();
    let booking_id = reservation_lifecycle::create_reservation_tx(
        &mut tx,
        CreateReservationRequest {
            room_id: "R301".to_string(),
            guest_name: "Khách cũ".to_string(),
            guest_phone: None,
            guest_doc_number: None,
            check_in_date: "2026-08-06".to_string(),
            check_out_date: "2026-08-08".to_string(),
            nights: 2,
            deposit_amount: None,
            source: None,
            notes: None,
            guests: None,
        },
        None,
    )
    .await
    .unwrap();

    let total: i64 = sqlx::query_scalar("SELECT total_price FROM bookings WHERE id = ?")
        .bind(&booking_id)
        .fetch_one(&mut *tx)
        .await
        .unwrap();

    assert_eq!(total, 1_000_000);

    tx.rollback().await.unwrap();
}
```

- [ ] **Step 2: Chạy test để xác nhận nó hỏng**

```bash
cargo test --lib services::booking::tests::reservations::create_reservation_charges_extra_guests
```

Kỳ vọng: **FAIL** khi biên dịch, báo `struct CreateReservationRequest has no field named guests`.

- [ ] **Step 3: Thêm migration v22**

Trong `mhm/src-tauri/src/db/core_extensions.rs`, thêm vào cuối file:

```rust
/// Số khách trên booking. Là **số đếm, không phải tiền** — cố ý đứng ngoài
/// `money_migration`, và tên cột cố ý không chứa từ khoá tiền tệ nào.
///
/// Cho phép NULL: booking tạo trước phiên bản này không khai số khách, và NULL
/// có nghĩa là không phụ thu, nên giá của chúng không đổi.
pub(super) async fn migrate_v22_booking_guest_count(
    pool: &Pool<Sqlite>,
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    execute_compat_alter(&mut tx, "ALTER TABLE bookings ADD COLUMN guests INTEGER").await?;

    set_schema_version(&mut tx, 22).await?;
    tx.commit().await?;
    Ok(())
}
```

Trong `mhm/src-tauri/src/db.rs`, thêm ngay sau khối `if current < 21 { … }` (dòng 310-312), trước `Ok(())`:

```rust
    // -- V22: số khách trên booking, để tính phụ thu thêm người --
    if current < 22 {
        core_extensions::migrate_v22_booking_guest_count(pool).await?;
    }
```

- [ ] **Step 4: Thêm trường vào request và lưu nó**

Trong `mhm/src-tauri/src/models.rs`, sửa struct ở dòng 490:

```rust
pub struct CreateReservationRequest {
    pub room_id: String,
    pub guest_name: String,
    pub guest_phone: Option<String>,
    pub guest_doc_number: Option<String>,
    pub check_in_date: String,
    pub check_out_date: String,
    pub nights: i32,
    pub deposit_amount: Option<MoneyVnd>,
    pub source: Option<String>,
    pub notes: Option<String>,
    /// Số khách ở thực tế. `None` ⇒ không phụ thu, giá giữ nguyên như cũ.
    /// Kiểu `Option` để gateway và agent không phải sửa theo.
    pub guests: Option<i32>,
}
```

Trong `mhm/src-tauri/src/services/booking/reservation_lifecycle.rs`, hàm `create_reservation_tx`: đổi lời gọi giá ở dòng 240 từ `None` sang `req.guests`, và thêm cột `guests` vào câu INSERT.

```rust
    let pricing = calculate_stay_price_tx(
        tx,
        &req.room_id,
        &req.check_in_date,
        &req.check_out_date,
        "nightly",
        req.guests,
    )
    .await?;
```

Câu INSERT (dòng 262-267) thêm `guests` vào danh sách cột và một `?` nữa vào `VALUES`:

```rust
    sqlx::query(
        "INSERT INTO bookings (
            id, room_id, primary_guest_id, check_in_at, expected_checkout, actual_checkout,
            nights, total_price, paid_amount, status, source, notes, created_by,
            booking_type, pricing_type, deposit_amount, guest_phone, scheduled_checkin,
            scheduled_checkout, pricing_snapshot, guests, created_at
         ) VALUES (?, ?, ?, ?, ?, NULL, ?, ?, 0, ?, ?, ?, NULL, 'reservation', 'nightly', ?, ?, ?, ?, NULL, ?, ?)",
    )
```

và thêm `.bind(req.guests)` **ngay trước** `.bind(&now)` ở cuối chuỗi bind, khớp đúng thứ tự cột.

- [ ] **Step 5: Chạy test, kỳ vọng xanh**

```bash
cargo test --lib services::booking::tests::reservations
```

Kỳ vọng: **PASS**, cả hai test mới.

- [ ] **Step 6: Commit**

```bash
git add mhm/src-tauri/src/db/core_extensions.rs mhm/src-tauri/src/db.rs mhm/src-tauri/src/models.rs mhm/src-tauri/src/services/booking/
git commit -m "feat(reservations): store a guest count and price the reservation with it"
```

---

### Task 3: Giữ số khách khi sửa đặt phòng và khi khách nhận phòng

Đây là hai trong ba chỗ rò giá mà spec nêu. Không có task này, khách đặt 4 người sẽ tụt về giá 2 người ngay khi dời ngày hoặc nhận phòng.

**Files:**
- Modify: `mhm/src-tauri/src/services/booking/reservation_lifecycle.rs:1043-1080` (`load_booked_reservation`), `:1103-1108` (`BookedReservation`), `:609` (confirm), `:758` (modify)
- Modify: `mhm/src-tauri/src/models.rs:503-509` (`ModifyReservationRequest`)
- Test: `mhm/src-tauri/src/services/booking/tests/reservations.rs`

**Interfaces:**
- Consumes: cột `bookings.guests`, `CreateReservationRequest.guests` từ Task 2.
- Produces: `ModifyReservationRequest.new_guests: Option<i32>`; `BookedReservation.guests: Option<i32>`.

- [ ] **Step 1: Viết test thất bại**

Thêm vào `mhm/src-tauri/src/services/booking/tests/reservations.rs`:

```rust
#[tokio::test]
async fn modify_reservation_keeps_the_extra_guest_charge() {
    let pool = test_pool().await;
    seed_room_with_guest_pricing(&pool, "R310", 500_000, 2, 50_000)
        .await
        .unwrap();

    let mut tx = pool.begin().await.unwrap();
    let booking_id = reservation_lifecycle::create_reservation_tx(
        &mut tx,
        CreateReservationRequest {
            room_id: "R310".to_string(),
            guest_name: "Nguyễn Nhật Huy".to_string(),
            guest_phone: None,
            guest_doc_number: None,
            check_in_date: "2026-08-06".to_string(),
            check_out_date: "2026-08-08".to_string(),
            nights: 2,
            deposit_amount: None,
            source: None,
            notes: None,
            guests: Some(4),
        },
        None,
    )
    .await
    .unwrap();

    // Dời sang 3 đêm, không nói gì về số khách.
    let booking = reservation_lifecycle::modify_reservation_tx(
        &mut tx,
        ModifyReservationRequest {
            booking_id: booking_id.clone(),
            new_check_in_date: "2026-08-10".to_string(),
            new_check_out_date: "2026-08-13".to_string(),
            new_nights: 3,
            new_guests: None,
        },
        "R310",
    )
    .await
    .unwrap();

    // 600.000₫/đêm × 3 đêm — không tụt về 500.000₫.
    assert_eq!(booking.total_price, 1_800_000);

    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn confirm_reservation_keeps_the_extra_guest_charge() {
    let pool = test_pool().await;
    seed_room_with_guest_pricing(&pool, "R311", 500_000, 2, 50_000)
        .await
        .unwrap();

    let today = Local::now().date_naive();
    let check_in = today.format("%Y-%m-%d").to_string();
    let check_out = (today + Duration::days(2)).format("%Y-%m-%d").to_string();

    let mut tx = pool.begin().await.unwrap();
    let booking_id = reservation_lifecycle::create_reservation_tx(
        &mut tx,
        CreateReservationRequest {
            room_id: "R311".to_string(),
            guest_name: "Nguyễn Nhật Huy".to_string(),
            guest_phone: None,
            guest_doc_number: None,
            check_in_date: check_in,
            check_out_date: check_out,
            nights: 2,
            deposit_amount: None,
            source: None,
            notes: None,
            guests: Some(4),
        },
        None,
    )
    .await
    .unwrap();

    let booking = reservation_lifecycle::confirm_reservation_tx(&mut tx, &booking_id, "R311", None)
        .await
        .unwrap();

    assert_eq!(booking.total_price, 1_200_000);

    tx.rollback().await.unwrap();
}
```

Thêm `ModifyReservationRequest` vào danh sách `models::{…}` trong khối `prelude` của `mhm/src-tauri/src/services/booking/tests/mod.rs`.

- [ ] **Step 2: Chạy test để xác nhận nó hỏng**

```bash
cargo test --lib services::booking::tests::reservations::modify_reservation_keeps_the_extra_guest_charge
```

Kỳ vọng: **FAIL** khi biên dịch, báo `struct ModifyReservationRequest has no field named new_guests`.

- [ ] **Step 3: Cho `BookedReservation` mang theo số khách**

Trong `mhm/src-tauri/src/services/booking/reservation_lifecycle.rs`, sửa struct ở dòng 1103:

```rust
struct BookedReservation {
    room_id: String,
    paid_amount: MoneyVnd,
    scheduled_checkout: String,
    pricing_type: String,
    guests: Option<i32>,
}
```

Sửa `load_booked_reservation` (dòng 1043): thêm `guests` vào câu SELECT và vào struct trả về.

```rust
    let row = sqlx::query(
        "SELECT room_id, status, paid_amount, scheduled_checkout, pricing_type, guests
         FROM bookings
         WHERE id = ?",
    )
```

```rust
    Ok(BookedReservation {
        room_id: row.get("room_id"),
        paid_amount: read_money_vnd_or_zero(&row, "paid_amount"),
        scheduled_checkout,
        pricing_type: row
            .get::<Option<String>, _>("pricing_type")
            .unwrap_or_else(|| "nightly".to_string()),
        guests: row.get("guests"),
    })
}
```

- [ ] **Step 4: Truyền số khách vào hai chỗ tính giá**

`confirm_reservation_tx`, dòng 609 — đổi `None` thành `reservation.guests`:

```rust
    let pricing = calculate_stay_price_tx(
        tx,
        &reservation.room_id,
        &today.format("%Y-%m-%d").to_string(),
        &effective_checkout,
        &reservation.pricing_type,
        reservation.guests,
    )
    .await?;
```

Trong `mhm/src-tauri/src/models.rs`, sửa struct ở dòng 503:

```rust
#[derive(Debug, Deserialize)]
pub struct ModifyReservationRequest {
    pub booking_id: String,
    pub new_check_in_date: String,
    pub new_check_out_date: String,
    pub new_nights: i32,
    /// `None` ⇒ giữ nguyên số khách đang lưu. Đây **không** phải lệnh xoá:
    /// muốn bỏ phụ thu thì gửi số khách mới nhỏ hơn hoặc bằng `max_guests`.
    pub new_guests: Option<i32>,
}
```

`modify_reservation_tx`, dòng 758 — dùng số khách mới nếu có, không thì giữ số đang lưu, rồi ghi lại vào cột:

```rust
    let effective_guests = req.new_guests.or(reservation.guests);
    let pricing = calculate_stay_price_tx(
        tx,
        &reservation.room_id,
        &req.new_check_in_date,
        &req.new_check_out_date,
        &reservation.pricing_type,
        effective_guests,
    )
    .await?;
    let total_price = pricing.total;
```

Câu UPDATE ngay dưới (dòng 770) thêm `guests = ?`:

```rust
    let result = sqlx::query(
        "UPDATE bookings
         SET check_in_at = ?, expected_checkout = ?, scheduled_checkin = ?, scheduled_checkout = ?, nights = ?, total_price = ?, guests = ?
         WHERE id = ? AND status = ? AND room_id = ?",
    )
```

và thêm `.bind(effective_guests)` ngay sau `.bind(total_price)`.

- [ ] **Step 5: Chạy test, kỳ vọng xanh**

```bash
cargo test --lib services::booking::tests::reservations
```

Kỳ vọng: **PASS**.

- [ ] **Step 6: Commit**

```bash
git add mhm/src-tauri/src/models.rs mhm/src-tauri/src/services/booking/
git commit -m "fix(reservations): stop modify and check-in from dropping the guest charge"
```

---

### Task 4: Giữ số khách khi gia hạn và khi trả phòng sớm

Chỗ rò thứ tư và thứ năm. Hai chỗ này rò cùng một hướng: đều tính thiếu tiền của chủ khách sạn.

**Files:**
- Modify: `mhm/src-tauri/src/services/booking/stay_lifecycle.rs:645-652` (SELECT của settlement), `:680`
- Modify: `mhm/src-tauri/src/services/booking/stay_lifecycle.rs:1010-1013` (SELECT của extend), `:1074`
- Test: `mhm/src-tauri/src/services/booking/tests/extend_stay.rs`, `mhm/src-tauri/src/services/booking/tests/checkout_settlement.rs`

**Interfaces:**
- Consumes: cột `bookings.guests`, `calculate_stay_price_tx(…, guests)`.
- Produces: không có API mới.

- [ ] **Step 1: Viết test thất bại cho gia hạn**

Thêm vào `mhm/src-tauri/src/services/booking/tests/extend_stay.rs`:

```rust
#[tokio::test]
async fn extend_stay_prices_the_extra_night_with_the_guest_charge() {
    let pool = test_pool().await;
    seed_room_with_guest_pricing(&pool, "R320", 500_000, 2, 50_000)
        .await
        .unwrap();
    let booking_id = uuid::Uuid::new_v4().to_string();
    seed_active_booking(&pool, &booking_id, "R320").await.unwrap();

    sqlx::query("UPDATE bookings SET guests = 4, total_price = 1200000, nights = 2 WHERE id = ?")
        .bind(&booking_id)
        .execute(&pool)
        .await
        .unwrap();

    let booking = stay_lifecycle::extend_stay(&pool, &booking_id).await.unwrap();

    // 1.200.000₫ cũ + 600.000₫ cho đêm thêm, không phải + 500.000₫.
    assert_eq!(booking.total_price, 1_800_000);
}
```

- [ ] **Step 2: Chạy test để xác nhận nó hỏng**

```bash
cargo test --lib services::booking::tests::extend_stay::extend_stay_prices_the_extra_night_with_the_guest_charge
```

Kỳ vọng: **FAIL** với `assertion `left == right` failed: left: 1700000, right: 1800000` — đêm thêm bị tính 500.000₫.

- [ ] **Step 3: Sửa đường gia hạn**

Trong `mhm/src-tauri/src/services/booking/stay_lifecycle.rs`, câu SELECT của `extend_stay_tx` (dòng 1010) thêm `guests`:

```rust
    let booking = sqlx::query(
        "SELECT room_id, nights, total_price, expected_checkout, pricing_type, guests, status
         FROM bookings WHERE id = ?",
    )
```

Sau dòng đọc `pricing_type` (khoảng dòng 1040), thêm:

```rust
    let guests: Option<i32> = booking.get("guests");
```

Lời gọi giá ở dòng 1074 đổi `None` thành `guests`:

```rust
    let incremental_pricing = calculate_stay_price_tx(
        tx,
        &room_id,
        &old_expected_checkout,
        &new_expected.to_rfc3339(),
        &pricing_type,
        guests,
    )
    .await?;
```

- [ ] **Step 4: Chạy test gia hạn, kỳ vọng xanh**

```bash
cargo test --lib services::booking::tests::extend_stay
```

Kỳ vọng: **PASS**.

- [ ] **Step 5: Viết test thất bại cho trả phòng sớm**

Thêm vào `mhm/src-tauri/src/services/booking/tests/checkout_settlement.rs`:

```rust
#[tokio::test]
async fn actual_nights_settlement_keeps_the_guest_charge() {
    let pool = test_pool().await;
    seed_room_with_guest_pricing(&pool, "R330", 500_000, 2, 50_000)
        .await
        .unwrap();
    let booking_id = uuid::Uuid::new_v4().to_string();
    seed_active_booking(&pool, &booking_id, "R330").await.unwrap();

    let check_in = Local::now() - Duration::days(2);
    sqlx::query(
        "UPDATE bookings SET guests = 4, nights = 3, total_price = 1800000, check_in_at = ? WHERE id = ?",
    )
    .bind(check_in.to_rfc3339())
    .bind(&booking_id)
    .execute(&pool)
    .await
    .unwrap();

    let settlement = stay_lifecycle::preview_checkout_settlement(
        &pool,
        &CheckoutSettlementPreviewRequest {
            booking_id: booking_id.clone(),
            settlement_mode: CheckoutSettlementMode::ActualNights,
        },
    )
    .await
    .unwrap();

    // Ở 2 đêm thật × 600.000₫, không phải × 500.000₫.
    assert_eq!(settlement.recommended_total, 1_200_000);
}
```

- [ ] **Step 6: Chạy test để xác nhận nó hỏng**

```bash
cargo test --lib services::booking::tests::checkout_settlement::actual_nights_settlement_keeps_the_guest_charge
```

Kỳ vọng: **FAIL** với `left: 1000000, right: 1200000`.

- [ ] **Step 7: Sửa đường trả phòng sớm**

Câu SELECT của `preview_checkout_settlement_tx` (dòng 645) thêm `guests`:

```rust
    let booking = sqlx::query(
        "SELECT room_id, check_in_at, nights, total_price, paid_amount,
                COALESCE(pricing_type, 'nightly') AS pricing_type, guests, status
         FROM bookings WHERE id = ?",
    )
```

Sau dòng `let original_total = read_money_vnd_or_zero(&booking, "total_price");` (khoảng dòng 673), thêm:

```rust
    let guests: Option<i32> = booking.get("guests");
```

Lời gọi giá ở dòng 680 đổi `None` thành `guests`:

```rust
            let pricing = calculate_stay_price_tx(
                tx,
                &room_id,
                &check_in_at,
                &settlement_boundary,
                &pricing_type,
                guests,
            )
            .await?;
```

- [ ] **Step 8: Chạy toàn bộ test backend, kỳ vọng xanh**

```bash
cargo test --lib services::booking
```

Kỳ vọng: **PASS** toàn bộ.

- [ ] **Step 9: Commit**

```bash
git add mhm/src-tauri/src/services/booking/
git commit -m "fix(stays): keep the guest charge through extend-stay and early checkout"
```

---

### Task 5: Xem trước giá theo mã phòng

Form cần một con số **giống hệt** con số sẽ ghi vào sổ. Đường xem trước hiện tra theo *loại phòng*, mà phụ thu lại là thuộc tính của *từng phòng*.

**Files:**
- Modify: `mhm/src-tauri/src/queries/booking/pricing_queries.rs` (thêm `load_stay_pricing_inputs_for_room`)
- Modify: `mhm/src-tauri/src/services/booking/pricing_service.rs` (thêm `calculate_room_price_preview`)
- Modify: `mhm/src-tauri/src/commands/pricing.rs` (thêm lệnh Tauri)
- Modify: `mhm/src-tauri/src/lib.rs:393` (đăng ký lệnh)
- Test: `mhm/src-tauri/src/services/booking/tests/pricing.rs`

**Interfaces:**
- Consumes: `load_room_guest_pricing_tx` và `StayPricingInputs` từ Task 1.
- Produces: lệnh Tauri `calculate_room_price_preview(room_id, check_in, check_out, pricing_type, guests)` trả `PricingResult` — Task 8 gọi nó.

- [ ] **Step 1: Viết test thất bại**

Thêm vào `mhm/src-tauri/src/services/booking/tests/pricing.rs`:

```rust
#[tokio::test]
async fn room_price_preview_matches_what_the_reservation_will_be_charged() {
    let pool = test_pool().await;
    seed_room_with_guest_pricing(&pool, "R340", 500_000, 2, 50_000)
        .await
        .unwrap();

    let preview = pricing_service::calculate_room_price_preview(
        &pool,
        "R340",
        "2026-08-06",
        "2026-08-08",
        "nightly",
        Some(4),
    )
    .await
    .unwrap();

    assert_eq!(preview.total, 1_200_000);
    assert_eq!(preview.breakdown.len(), 2);
    assert_eq!(preview.breakdown[1].label, "Phụ thu 2 khách");
}
```

Thêm `pricing_service` vào danh sách `services::booking::{…}` trong `prelude` của `mhm/src-tauri/src/services/booking/tests/mod.rs`.

- [ ] **Step 2: Chạy test để xác nhận nó hỏng**

```bash
cargo test --lib services::booking::tests::pricing::room_price_preview_matches_what_the_reservation_will_be_charged
```

Kỳ vọng: **FAIL** khi biên dịch, `cannot find function calculate_room_price_preview`.

- [ ] **Step 3: Thêm hàm nạp inputs theo mã phòng**

Trong `mhm/src-tauri/src/queries/booking/pricing_queries.rs`, thêm cạnh `load_stay_pricing_inputs_for_room_type`:

```rust
/// Bản xem trước theo **mã phòng**. Khác `..._for_room_type` ở chỗ phụ thu thêm
/// người là thuộc tính của từng phòng, nên phòng đã chọn rồi thì phải đọc đúng
/// phòng đó — hai phòng cùng loại có thể đặt phụ thu khác nhau.
///
/// Đọc lệnh với ngày đặc biệt giống bản theo loại phòng: lỗi thì coi như 0%,
/// vì đây là xem trước chứ chưa thu tiền.
pub(crate) async fn load_stay_pricing_inputs_for_room(
    pool: &Pool<Sqlite>,
    room_id: &str,
    check_in: &str,
    check_out: &str,
    pricing_type: &str,
    guests: Option<i32>,
) -> BookingResult<StayPricingInputs> {
    let room_type = load_room_type(pool, room_id).await?;
    let stored_rule = load_stored_pricing_rule(pool, &room_type).await?;
    let fallback_base_price = if stored_rule.is_none() {
        load_fallback_base_price(pool, &room_type).await?
    } else {
        None
    };
    let special_uplift_pct = load_special_uplift(pool, check_in).await.unwrap_or(0.0);
    let guest_pricing = load_room_guest_pricing(pool, room_id).await?;

    Ok(StayPricingInputs {
        room_type,
        stored_rule,
        fallback_base_price,
        special_uplift_pct,
        check_in: check_in.to_string(),
        check_out: check_out.to_string(),
        pricing_type: pricing_type.to_string(),
        guests,
        base_guests: guest_pricing.base_guests,
        extra_person_fee: guest_pricing.extra_person_fee,
    })
}

async fn load_room_type(pool: &Pool<Sqlite>, room_id: &str) -> BookingResult<String> {
    sqlx::query_scalar::<_, String>(ROOM_TYPE_SQL)
        .bind(room_id)
        .fetch_optional(pool)
        .await
        .map_err(database_error)?
        .ok_or_else(|| room_not_found(room_id))
}

async fn load_room_guest_pricing(
    pool: &Pool<Sqlite>,
    room_id: &str,
) -> BookingResult<RoomGuestPricing> {
    let row = sqlx::query(ROOM_GUEST_PRICING_SQL)
        .bind(room_id)
        .fetch_optional(pool)
        .await
        .map_err(database_error)?;

    Ok(row
        .as_ref()
        .map(room_guest_pricing_from_row)
        .unwrap_or_default())
}
```

- [ ] **Step 4: Thêm hàm dịch vụ và lệnh Tauri**

Trong `mhm/src-tauri/src/services/booking/pricing_service.rs`, thêm `load_stay_pricing_inputs_for_room` vào khối `use` sẵn có rồi thêm:

```rust
/// Bản xem trước mà form đặt phòng dùng: keyed theo phòng đã chọn, nên con số
/// nó trả về đúng bằng con số `calculate_stay_price_tx` sẽ tính khi ghi sổ.
pub async fn calculate_room_price_preview(
    pool: &Pool<Sqlite>,
    room_id: &str,
    check_in: &str,
    check_out: &str,
    pricing_type: &str,
    guests: Option<i32>,
) -> BookingResult<crate::pricing::PricingResult> {
    let inputs = load_stay_pricing_inputs_for_room(
        pool,
        room_id,
        check_in,
        check_out,
        pricing_type,
        guests,
    )
    .await?;
    calculate_from_loaded_inputs(&inputs)
}
```

Trong `mhm/src-tauri/src/commands/pricing.rs`, thêm sau `calculate_price_preview` (dòng 117):

```rust
#[tauri::command]
pub async fn calculate_room_price_preview(
    state: State<'_, AppState>,
    room_id: String,
    check_in: String,
    check_out: String,
    pricing_type: String,
    guests: Option<i32>,
) -> Result<crate::pricing::PricingResult, String> {
    pricing_service::calculate_room_price_preview(
        &state.db,
        &room_id,
        &check_in,
        &check_out,
        &pricing_type,
        guests,
    )
    .await
    .map_err(|error| error.to_string())
}
```

Trong `mhm/src-tauri/src/lib.rs`, thêm vào danh sách `generate_handler!` ngay dưới `commands::pricing::calculate_price_preview,` (dòng 393):

```rust
            commands::pricing::calculate_room_price_preview,
```

- [ ] **Step 5: Chạy test, kỳ vọng xanh**

```bash
cargo test --lib services::booking::tests::pricing
```

Kỳ vọng: **PASS**.

- [ ] **Step 6: Kiểm tra toàn bộ backend và canh gác tiền tệ**

```bash
cargo test --lib
```

Kỳ vọng: **PASS** toàn bộ.

```bash
npm run verify:money
```

Chạy từ `mhm/`. Kỳ vọng: **PASS** — không phát hiện tiền kiểu số thực.

- [ ] **Step 7: Commit**

```bash
git add mhm/src-tauri/src/queries/booking/pricing_queries.rs mhm/src-tauri/src/services/booking/pricing_service.rs mhm/src-tauri/src/commands/pricing.rs mhm/src-tauri/src/lib.rs
git commit -m "feat(pricing): add a room-keyed preview so the sheet quotes the real price"
```

---

### Task 6: Form — bỏ ô Số đêm, ngày đi bấm được lịch

**Files:**
- Modify: `mhm/src/components/ReservationSheet.tsx:30-87,203-243`
- Test: `mhm/src/components/ReservationSheet.test.tsx`

**Interfaces:**
- Consumes: không có gì từ backend.
- Produces: form không còn state `nights`; số đêm suy ra từ hai ngày qua `nightsBetween(checkInDate, checkOutDate)`.

- [ ] **Step 1: Viết test thất bại**

Thêm vào `mhm/src/components/ReservationSheet.test.tsx`:

```tsx
it("để người dùng chọn ngày đi và không còn ô số đêm", async () => {
  render(<ReservationSheet open onOpenChange={vi.fn()} />);

  expect(screen.queryByLabelText(/số đêm/i)).not.toBeInTheDocument();

  const checkOut = screen.getByLabelText(/ngày đi/i) as HTMLInputElement;
  expect(checkOut.readOnly).toBe(false);

  // Dòng tổng tiền chỉ hiện khi đã chọn phòng — đó là chỗ số đêm hiện ra.
  fireEvent.change(screen.getByLabelText(/^phòng$/i), { target: { value: "R101" } });
  fireEvent.change(screen.getByLabelText(/ngày đến/i), {
    target: { value: "2026-08-06" },
  });
  fireEvent.change(checkOut, { target: { value: "2026-08-09" } });

  await waitFor(() => {
    expect(screen.getByText(/3 đêm/i)).toBeInTheDocument();
  });
});

it("khoá nút đặt phòng khi ngày đi không sau ngày đến", async () => {
  render(<ReservationSheet open onOpenChange={vi.fn()} />);

  // Điền đủ phòng và tên khách trước, để nút bị khoá *chỉ vì* ngày sai —
  // không điền thì nút vốn đã khoá và test không chứng minh được gì.
  fireEvent.change(screen.getByLabelText(/^phòng$/i), { target: { value: "R101" } });
  fireEvent.change(screen.getByPlaceholderText(/họ và tên/i), {
    target: { value: "Nguyễn Nhật Huy" },
  });
  fireEvent.change(screen.getByLabelText(/ngày đến/i), {
    target: { value: "2026-08-06" },
  });

  const submit = screen.getByRole("button", { name: /đặt phòng/i });
  await waitFor(() => expect(submit).toBeEnabled());

  fireEvent.change(screen.getByLabelText(/ngày đi/i), {
    target: { value: "2026-08-06" },
  });

  await waitFor(() => {
    expect(screen.getByText(/ngày đi phải sau ngày đến/i)).toBeInTheDocument();
  });
  expect(submit).toBeDisabled();
});
```

- [ ] **Step 2: Chạy test để xác nhận nó hỏng**

Chạy từ `mhm/`:

```bash
npm test -- ReservationSheet
```

Kỳ vọng: **FAIL** — `getByLabelText(/ngày đi/i)` không tìm thấy nhãn (ô hiện chưa gắn `htmlFor`/`id`), và ô đang `readOnly`.

- [ ] **Step 3: Thay state số đêm bằng hàm suy ra**

Trong `mhm/src/components/ReservationSheet.tsx`, bỏ dòng `const [nights, setNights] = useState(1);` (dòng 32) và hàm `updateCheckout` (dòng 82-87). Thêm ngay trên `export default function`:

```tsx
function nightsBetween(checkIn: string, checkOut: string): number {
    if (!checkIn || !checkOut) return 0;
    const ms = new Date(`${checkOut}T00:00:00`).getTime() - new Date(`${checkIn}T00:00:00`).getTime();
    if (Number.isNaN(ms)) return 0;
    return Math.round(ms / 86_400_000);
}

function addDays(date: string, days: number): string {
    const d = new Date(`${date}T00:00:00`);
    d.setDate(d.getDate() + days);
    return d.toISOString().split("T")[0];
}
```

Trong thân component, thay chỗ dùng `nights` bằng:

```tsx
    const nights = nightsBetween(checkInDate, checkOutDate);
    const datesValid = nights > 0;
```

Trong `useEffect` khởi tạo (dòng 53-76), nhánh tạo mới đổi thành:

```tsx
            } else {
                // Mặc định: nhận phòng ngày mai, ở 1 đêm.
                const tomorrow = addDays(new Date().toISOString().split("T")[0], 1);
                setCheckInDate(tomorrow);
                setCheckOutDate(addDays(tomorrow, 1));
            }
```

Nhánh sửa (`editBooking`) bỏ dòng `setNights(editBooking.nights);`. Hàm `resetForm` bỏ dòng `setNights(1);`.

- [ ] **Step 4: Sửa phần hiển thị hai ô ngày**

Trước hết gắn nhãn cho ô chọn phòng — hiện `<label>` và `<select>` không nối với nhau nên test không tìm được ô này, và trình đọc màn hình cũng vậy. Trong khối `{/* Room Selection */}` (dòng 185-200), thêm `htmlFor` vào `<label>` và `id` vào `<select>`:

```tsx
                        <label htmlFor="reservation-room" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">Phòng</label>
                        <select
                            id="reservation-room"
                            className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200 disabled:opacity-60"
```

Rồi thay cả khối `{/* Dates */}` và khối `{/* Nights */}` (dòng 202-243) bằng:

```tsx
                    {/* Dates */}
                    <div className="grid grid-cols-2 gap-3">
                        <div className="space-y-1.5">
                            <label htmlFor="reservation-check-in" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">Ngày đến</label>
                            <input
                                id="reservation-check-in"
                                type="date"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200"
                                value={checkInDate}
                                min={new Date().toISOString().split("T")[0]}
                                onChange={(e) => {
                                    const nextCheckIn = e.target.value;
                                    setCheckInDate(nextCheckIn);
                                    // Giữ ngày đi hợp lệ: đẩy nó ra sau ngày đến mới.
                                    if (nextCheckIn && nightsBetween(nextCheckIn, checkOutDate) <= 0) {
                                        setCheckOutDate(addDays(nextCheckIn, 1));
                                    }
                                }}
                            />
                        </div>
                        <div className="space-y-1.5">
                            <label htmlFor="reservation-check-out" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">Ngày đi</label>
                            <input
                                id="reservation-check-out"
                                type="date"
                                className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200"
                                value={checkOutDate}
                                min={checkInDate ? addDays(checkInDate, 1) : undefined}
                                onChange={(e) => setCheckOutDate(e.target.value)}
                            />
                        </div>
                    </div>

                    {checkInDate && checkOutDate && !datesValid && (
                        <div className="rounded-xl p-3 text-sm bg-red-50 text-red-700 border border-red-200">
                            Ngày đi phải sau ngày đến ít nhất 1 đêm.
                        </div>
                    )}
```

- [ ] **Step 5: Chặn gửi khi ngày sai**

Trong `handleSubmit`, ngay sau khối kiểm tra `!roomId || !checkInDate || !checkOutDate`, thêm:

```tsx
        if (nights <= 0) {
            toast.error("Ngày đi phải sau ngày đến");
            return;
        }
```

Nút gửi (dòng 388) thêm điều kiện `!datesValid`:

```tsx
                        disabled={loading || (!isEditMode && !guestName) || !roomId || !datesValid || (availability !== null && !availability.available)}
```

- [ ] **Step 6: Chạy test, kỳ vọng xanh**

```bash
npm test -- ReservationSheet
```

Kỳ vọng: **PASS**.

- [ ] **Step 7: Commit**

```bash
git add mhm/src/components/ReservationSheet.tsx mhm/src/components/ReservationSheet.test.tsx
git commit -m "feat(reservation-sheet): let the checkout date open a calendar, drop the nights box"
```

---

### Task 7: Form — ô Số khách

**Files:**
- Modify: `mhm/src-tauri/src/models.rs:344-362` (`BookingWithGuest`)
- Modify: `mhm/src-tauri/src/queries/booking/booking_list_queries.rs:15,66-85`
- Modify: `mhm/src/types/index.ts:18-26` (`Room`), `:177-191` (`EditableBooking`), `:265-284` (`BookingWithGuest`)
- Modify: `mhm/src/components/ReservationSheet.tsx`
- Test: `mhm/src/components/ReservationSheet.test.tsx`

**Interfaces:**
- Consumes: `CreateReservationRequest.guests` (Task 2), `ModifyReservationRequest.new_guests` (Task 3).
- Produces: form gửi `guests` trong `create_reservation` và `new_guests` trong `modify_reservation`; `get_all_bookings` trả thêm `guests`.

- [ ] **Step 1: Viết test thất bại**

Thêm vào `mhm/src/components/ReservationSheet.test.tsx`:

```tsx
it("nạp số khách theo phòng và gửi nó khi đặt phòng", async () => {
  render(<ReservationSheet open onOpenChange={vi.fn()} />);

  fireEvent.change(screen.getByLabelText(/^phòng$/i), { target: { value: "R101" } });

  const guests = screen.getByLabelText(/số khách/i) as HTMLInputElement;
  await waitFor(() => expect(guests.value).toBe("2"));

  fireEvent.change(guests, { target: { value: "4" } });
  fireEvent.change(screen.getByPlaceholderText(/họ và tên/i), {
    target: { value: "Nguyễn Nhật Huy" },
  });
  fireEvent.click(screen.getByRole("button", { name: /đặt phòng/i }));

  await waitFor(() => {
    expect(invokeWriteCommand).toHaveBeenCalledWith(
      "create_reservation",
      expect.objectContaining({
        req: expect.objectContaining({ guests: 4 }),
      }),
      expect.anything(),
    );
  });
});
```

Trong khối `vi.mock("@/stores/useHotelStore", …)` ở đầu file test, thêm `max_guests: 2` và `extra_person_fee: 50000` vào phòng giả `R101`.

- [ ] **Step 2: Chạy test để xác nhận nó hỏng**

```bash
npm test -- ReservationSheet
```

Kỳ vọng: **FAIL** — không tìm thấy nhãn `/số khách/i`.

- [ ] **Step 3: Cho đường đọc trả về số khách**

Không có bước này, màn hình sửa luôn hiện 2 khách kể cả với booking 4 khách — `editBooking` thực chất là `BookingWithGuest` lấy từ lệnh `get_all_bookings`, và câu truy vấn đó không đọc cột `guests`.

Trong `mhm/src-tauri/src/models.rs`, thêm vào cuối struct `BookingWithGuest` (dòng 361, sau `guest_phone`):

```rust
    pub guests: Option<i32>,
```

Trong `mhm/src-tauri/src/queries/booking/booking_list_queries.rs`, câu SELECT ở dòng 15 thêm `b.guests`:

```rust
                b.booking_type, b.deposit_amount, b.scheduled_checkin, b.scheduled_checkout, b.guest_phone, b.guests
```

và `map_booking_with_guest` (dòng 66) thêm dòng cuối, sau `guest_phone`:

```rust
        guests: row.get("guests"),
```

- [ ] **Step 4: Bổ sung kiểu dữ liệu phía giao diện**

Trong `mhm/src/types/index.ts`, sửa `Room` (dòng 18) — backend đã trả sẵn hai trường này qua `get_rooms`:

```ts
export interface Room {
  id: string;
  name: string;
  type: string;
  floor: number;
  has_balcony: boolean;
  base_price: MoneyVnd;
  max_guests: number;
  extra_person_fee: MoneyVnd;
  status: RoomStatus;
}
```

Sửa `EditableBooking` (dòng 177) — thêm sau `nights`:

```ts
  guests: number | null;
```

Sửa `BookingWithGuest` (dòng 265) — thêm sau `guest_phone`, khớp với struct Rust vừa sửa:

```ts
  guests: number | null;
```

- [ ] **Step 5: Thêm ô Số khách vào form**

Trong `mhm/src/components/ReservationSheet.tsx`, thêm state cạnh các state khác:

```tsx
    const [guests, setGuests] = useState(2);
```

Trong `useEffect` khởi tạo, nhánh `editBooking` thêm:

```tsx
                setGuests(editBooking.guests ?? 2);
```

Thêm effect nạp lại số khách khi đổi phòng — mốc tính giá gắn với từng phòng, nên đổi phòng phải nạp lại kể cả khi người dùng đã sửa tay:

```tsx
    useEffect(() => {
        if (isEditMode || !roomId) return;
        const room = rooms.find((r) => r.id === roomId);
        if (room) setGuests(room.max_guests);
    }, [roomId, rooms, isEditMode]);
```

Trong `resetForm`, thêm `setGuests(2);`.

Thêm khối hiển thị vào đúng chỗ ô Số đêm cũ, ngay dưới cặp ngày:

```tsx
                    {/* Guests */}
                    <div className="space-y-1.5">
                        <label htmlFor="reservation-guests" className="text-xs font-semibold text-slate-500 uppercase tracking-wider">Số khách</label>
                        <input
                            id="reservation-guests"
                            type="number"
                            min={1}
                            className="w-full h-10 px-3 rounded-xl border border-slate-200 bg-slate-50 text-sm focus:outline-none focus:ring-2 focus:ring-blue-200"
                            value={guests}
                            onChange={(e) => setGuests(Math.max(1, parseInt(e.target.value) || 1))}
                        />
                    </div>
```

- [ ] **Step 6: Gửi số khách xuống backend**

Trong `handleSubmit`, nhánh sửa thêm `new_guests`:

```tsx
                await invokeWriteCommand("modify_reservation", {
                    req: {
                        booking_id: editBooking.id,
                        new_check_in_date: checkInDate,
                        new_check_out_date: checkOutDate,
                        new_nights: nights,
                        new_guests: guests,
                    },
                }, {
```

Nhánh tạo mới thêm `guests` vào `req`, ngay sau `nights`:

```tsx
                        nights,
                        guests,
```

- [ ] **Step 7: Chạy test, kỳ vọng xanh**

```bash
npm test -- ReservationSheet
```

Kỳ vọng: **PASS**.

```bash
cargo test --lib queries::booking
```

Chạy từ `mhm/src-tauri/`. Kỳ vọng: **PASS** — câu truy vấn danh sách booking vẫn chạy sau khi thêm cột.

- [ ] **Step 8: Commit**

```bash
git add mhm/src-tauri/src/models.rs mhm/src-tauri/src/queries/booking/booking_list_queries.rs mhm/src/types/index.ts mhm/src/components/ReservationSheet.tsx mhm/src/components/ReservationSheet.test.tsx
git commit -m "feat(reservation-sheet): add a guest count and send it with the booking"
```

---

### Task 8: Form — dòng tổng tiền lấy từ engine

Bước cuối, và là bước đóng lại lỗi "số trên màn hình khác số trong sổ".

**Files:**
- Create: `mhm/src/hooks/usePricePreview.ts`
- Modify: `mhm/src/components/ReservationSheet.tsx:353-369`
- Test: `mhm/src/components/ReservationSheet.test.tsx`

**Interfaces:**
- Consumes: lệnh Tauri `calculate_room_price_preview` (Task 5); state `guests`, `checkInDate`, `checkOutDate`, `roomId`.
- Produces: `usePricePreview({ roomId, checkIn, checkOut, guests, debounceMs })` trả `{ preview, loading }`, `preview` kiểu `PricingResult | null`.

- [ ] **Step 1: Viết test thất bại**

Thêm vào `mhm/src/components/ReservationSheet.test.tsx`:

```tsx
it("hiện tổng tiền do engine trả về, kèm dòng phụ thu", async () => {
  invoke.mockImplementation(async (command: string) => {
    if (command === "calculate_room_price_preview") {
      return {
        pricing_type: "nightly",
        base_amount: 1_000_000,
        surcharge_amount: 0,
        weekend_amount: 0,
        total: 1_200_000,
        capped: false,
        breakdown: [
          { label: "2 night(s) x 500.000", amount: 1_000_000 },
          { label: "Phụ thu 2 khách", amount: 200_000 },
        ],
      };
    }
    return { available: true, conflicts: [], max_nights: null };
  });

  render(<ReservationSheet open onOpenChange={vi.fn()} />);
  fireEvent.change(screen.getByLabelText(/^phòng$/i), { target: { value: "R101" } });
  fireEvent.change(screen.getByLabelText(/số khách/i), { target: { value: "4" } });

  await waitFor(() => {
    expect(screen.getByText("Phụ thu 2 khách")).toBeInTheDocument();
    expect(screen.getByText("1.200.000₫")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Chạy test để xác nhận nó hỏng**

```bash
npm test -- ReservationSheet
```

Kỳ vọng: **FAIL** — không thấy chữ "Phụ thu 2 khách"; form vẫn tự nhân giá.

- [ ] **Step 3: Viết hook**

Tạo `mhm/src/hooks/usePricePreview.ts`. Nó theo đúng khuôn `useAvailability`: chống đua bằng số thứ tự yêu cầu, có trễ, huỷ khi unmount.

```ts
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

import type { PricingResult } from "@/types";

interface UsePricePreviewOptions {
    roomId: string;
    checkIn: string;
    checkOut: string;
    guests: number;
    debounceMs?: number;
}

/// Con số hiển thị phải do engine trả về, không phải do giao diện tự nhân —
/// đó là cách duy nhất để số trên màn hình bằng số ghi vào sổ.
export function usePricePreview({
    roomId,
    checkIn,
    checkOut,
    guests,
    debounceMs = 0,
}: UsePricePreviewOptions) {
    const [preview, setPreview] = useState<PricingResult | null>(null);
    const [loading, setLoading] = useState(false);
    const requestIdRef = useRef(0);

    useEffect(() => {
        if (!roomId || !checkIn || !checkOut) {
            requestIdRef.current += 1;
            setPreview(null);
            setLoading(false);
            return;
        }

        const requestId = requestIdRef.current + 1;
        requestIdRef.current = requestId;
        let active = true;

        const run = async () => {
            setLoading(true);
            try {
                const result = await invoke<PricingResult>("calculate_room_price_preview", {
                    roomId,
                    checkIn,
                    checkOut,
                    pricingType: "nightly",
                    guests,
                });
                if (active && requestIdRef.current === requestId) {
                    setPreview(result);
                }
            } catch {
                if (active && requestIdRef.current === requestId) {
                    setPreview(null);
                }
            } finally {
                if (active && requestIdRef.current === requestId) {
                    setLoading(false);
                }
            }
        };

        const timer = debounceMs > 0 ? window.setTimeout(run, debounceMs) : null;
        if (timer == null) {
            void run();
        }

        return () => {
            active = false;
            if (timer != null) {
                clearTimeout(timer);
            }
        };
    }, [checkIn, checkOut, debounceMs, guests, roomId]);

    return { preview, loading };
}
```

Nếu `mhm/src/types/index.ts` chưa có `PricingResult`, thêm:

```ts
export interface PricingLine {
  label: string;
  amount: MoneyVnd;
}

export interface PricingResult {
  pricing_type: string;
  base_amount: MoneyVnd;
  surcharge_amount: MoneyVnd;
  weekend_amount: MoneyVnd;
  total: MoneyVnd;
  breakdown: PricingLine[];
  capped: boolean;
}
```

- [ ] **Step 4: Thay khối tổng tiền trong form**

Trong `mhm/src/components/ReservationSheet.tsx`, thêm import và lời gọi hook cạnh `useAvailability`:

```tsx
import { usePricePreview } from "@/hooks/usePricePreview";
```

```tsx
    const { preview, loading: pricing } = usePricePreview({
        roomId,
        checkIn: checkInDate,
        checkOut: checkOutDate,
        guests,
        debounceMs: 300,
    });
```

Thay toàn bộ khối `{/* Price Estimate */}` (dòng 353-369) bằng:

```tsx
                    {/* Price Estimate — con số do engine tính, không phải phép nhân ở đây */}
                    {roomId && datesValid && (
                        <div className="bg-blue-50 rounded-xl p-4 space-y-1">
                            {pricing && !preview ? (
                                <div className="text-sm text-slate-500">Đang tính giá...</div>
                            ) : preview ? (
                                <>
                                    {preview.breakdown.map((line, i) => (
                                        <div key={i} className="flex justify-between text-sm">
                                            <span className="text-slate-600">{line.label}</span>
                                            <span className="text-slate-700">{fmtNumber(line.amount)}₫</span>
                                        </div>
                                    ))}
                                    <div className="flex justify-between text-sm pt-1 border-t border-blue-100">
                                        <span className="text-slate-600">Tổng</span>
                                        <span className="font-bold text-slate-800">{fmtNumber(preview.total)}₫</span>
                                    </div>
                                    {deposit && parseFloat(deposit) > 0 && (
                                        <div className="flex justify-between text-sm">
                                            <span className="text-slate-600">Tiền cọc</span>
                                            <span className="font-semibold text-emerald-700">-{fmtNumber(parseFloat(deposit))}₫</span>
                                        </div>
                                    )}
                                </>
                            ) : null}
                        </div>
                    )}
```

- [ ] **Step 5: Chạy test, kỳ vọng xanh**

```bash
npm test -- ReservationSheet
```

Kỳ vọng: **PASS**.

- [ ] **Step 6: Chạy toàn bộ kiểm chứng**

Chạy từ `mhm/`:

```bash
npm run verify:full
```

Kỳ vọng: **PASS** — lint, typecheck, test frontend, test Rust, và canh gác tiền tệ.

- [ ] **Step 7: Commit**

```bash
git add mhm/src/hooks/usePricePreview.ts mhm/src/types/index.ts mhm/src/components/ReservationSheet.tsx mhm/src/components/ReservationSheet.test.tsx
git commit -m "feat(reservation-sheet): quote the price the engine will actually charge"
```

---

## Ngoài phạm vi

Spec ghi rõ hai việc **không** làm trong kế hoạch này:

1. Màn hình khai báo mùa cao điểm (`special_dates` đã có bảng, engine đã đọc, thiếu chỗ nhập).
2. Ô số khách cho khách vãng lai check-in thẳng (`stay_lifecycle.rs:258`).

Ở Task 1, `check_in_tx` (dòng 318) và `group_lifecycle.rs:441` cố ý giữ `None` — đó là hai đường vào của khách vãng lai và khách đoàn, chưa có số khách để truyền. Chúng giữ nguyên hành vi cũ.
