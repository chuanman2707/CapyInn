//! Pure stay-pricing rules.
//!
//! This module owns the decision of *what a stay costs given its inputs*, and
//! nothing else. Loading those inputs is `queries::booking::pricing_queries`;
//! sequencing load-then-calculate is `services::booking::pricing_service`.
//! Keeping the rules free of SQL is what makes them testable without a
//! database, and it is enforced by `architecture_guard`.

use super::{BookingError, BookingResult};
use crate::money::MoneyVnd;
use chrono::NaiveDate;

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

/// A row of `pricing_rules`, decoded but not yet interpreted.
#[derive(Debug, Clone)]
pub(crate) struct StoredPricingRule {
    pub(crate) room_type: String,
    pub(crate) hourly_rate: MoneyVnd,
    pub(crate) overnight_rate: MoneyVnd,
    pub(crate) daily_rate: MoneyVnd,
    pub(crate) overnight_start: String,
    pub(crate) overnight_end: String,
    pub(crate) daily_checkin: String,
    pub(crate) daily_checkout: String,
    pub(crate) early_checkin_surcharge_pct: f64,
    pub(crate) late_checkout_surcharge_pct: f64,
    pub(crate) weekend_uplift_pct: f64,
}

impl StoredPricingRule {
    fn to_pricing_rule(&self) -> crate::pricing::PricingRule {
        crate::pricing::PricingRule {
            room_type: self.room_type.clone(),
            hourly_rate: self.hourly_rate,
            overnight_rate: self.overnight_rate,
            daily_rate: self.daily_rate,
            overnight_start: self.overnight_start.clone(),
            overnight_end: self.overnight_end.clone(),
            daily_checkin: self.daily_checkin.clone(),
            daily_checkout: self.daily_checkout.clone(),
            early_checkin_surcharge_pct: self.early_checkin_surcharge_pct,
            late_checkout_surcharge_pct: self.late_checkout_surcharge_pct,
            weekend_uplift_pct: self.weekend_uplift_pct,
        }
    }
}

/// A configured rule wins outright; otherwise derive one from the room's base
/// price, falling back to a house default when the room has no price either.
pub(crate) fn build_effective_pricing_rule(
    inputs: &StayPricingInputs,
) -> crate::pricing::PricingRule {
    if let Some(stored_rule) = &inputs.stored_rule {
        return stored_rule.to_pricing_rule();
    }

    let fallback_price = inputs.fallback_base_price.unwrap_or(350_000);

    crate::pricing::PricingRule {
        room_type: inputs.room_type.clone(),
        hourly_rate: fallback_price / 5,
        overnight_rate: fallback_price * 75 / 100,
        daily_rate: fallback_price,
        ..Default::default()
    }
}

/// Số đêm giữa hai mốc, chấp nhận cả `YYYY-MM-DD` lẫn RFC3339 (cắt 10 ký tự đầu).
/// Trả 0 khi ngày đi không sau ngày đến — đúng lúc `calculate_price` cũng trả 0.
fn nights_between(check_in: &str, check_out: &str) -> BookingResult<i64> {
    let parse = |value: &str| {
        let head = value.get(..10).unwrap_or(value);
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

    // Goes through the same transport-safe guard as every other money line in
    // `crate::pricing` (`checked_mul_money` ends in `validate_transport_money_vnd`),
    // instead of a raw `checked_mul` that would let a fat-fingered guest count
    // write an unsafe-integer total straight to `bookings.total_price`.
    let per_night =
        crate::pricing::checked_mul_money(inputs.extra_person_fee, extra_guests, "extra_guest_charge")
            .map_err(BookingError::validation)?;
    let amount = crate::pricing::checked_mul_money(per_night, nights, "extra_guest_charge")
        .map_err(BookingError::validation)?;

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
        result.total = crate::pricing::checked_add_money(result.total, extra_amount, "stay_total")
            .map_err(BookingError::validation)?;
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::{
        build_effective_pricing_rule, calculate_from_loaded_inputs, nights_between,
        StayPricingInputs, StoredPricingRule,
    };
    use crate::domain::booking::BookingError;
    use crate::money::MAX_TRANSPORT_SAFE_MONEY_VND;

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

    #[test]
    fn build_effective_pricing_rule_prefers_stored_rule_values() {
        let mut inputs = sample_inputs();
        inputs.fallback_base_price = Some(500_000);
        inputs.stored_rule = Some(StoredPricingRule {
            room_type: "deluxe".to_string(),
            hourly_rate: 120_000,
            overnight_rate: 500_000,
            daily_rate: 700_000,
            overnight_start: "21:00".to_string(),
            overnight_end: "10:00".to_string(),
            daily_checkin: "13:00".to_string(),
            daily_checkout: "11:00".to_string(),
            early_checkin_surcharge_pct: 15.0,
            late_checkout_surcharge_pct: 20.0,
            weekend_uplift_pct: 12.5,
        });

        let rule = build_effective_pricing_rule(&inputs);

        assert_eq!(rule.room_type, "deluxe");
        assert_eq!(rule.hourly_rate, 120_000);
        assert_eq!(rule.overnight_rate, 500_000);
        assert_eq!(rule.daily_rate, 700_000);
        assert_eq!(rule.overnight_start, "21:00");
        assert_eq!(rule.overnight_end, "10:00");
        assert_eq!(rule.daily_checkin, "13:00");
        assert_eq!(rule.daily_checkout, "11:00");
        assert_eq!(rule.early_checkin_surcharge_pct, 15.0);
        assert_eq!(rule.late_checkout_surcharge_pct, 20.0);
        assert_eq!(rule.weekend_uplift_pct, 12.5);
    }

    #[test]
    fn build_effective_pricing_rule_derives_fallback_rates_from_base_price() {
        let mut inputs = sample_inputs();
        inputs.room_type = "deluxe".to_string();
        inputs.fallback_base_price = Some(500_000);

        let rule = build_effective_pricing_rule(&inputs);

        assert_eq!(rule.room_type, "deluxe");
        assert_eq!(rule.hourly_rate, 100_000);
        assert_eq!(rule.overnight_rate, 375_000);
        assert_eq!(rule.daily_rate, 500_000);
        assert_eq!(rule.overnight_start, "22:00");
        assert_eq!(rule.overnight_end, "11:00");
        assert_eq!(rule.daily_checkin, "14:00");
        assert_eq!(rule.daily_checkout, "12:00");
        assert_eq!(rule.early_checkin_surcharge_pct, 30.0);
        assert_eq!(rule.late_checkout_surcharge_pct, 30.0);
        assert_eq!(rule.weekend_uplift_pct, 20.0);
    }

    #[test]
    fn build_effective_pricing_rule_uses_default_price_and_metadata_when_base_price_missing() {
        let rule = build_effective_pricing_rule(&sample_inputs());

        assert_eq!(rule.room_type, "standard");
        assert_eq!(rule.hourly_rate, 70_000);
        assert_eq!(rule.overnight_rate, 262_500);
        assert_eq!(rule.daily_rate, 350_000);
        assert_eq!(rule.overnight_start, "22:00");
        assert_eq!(rule.overnight_end, "11:00");
        assert_eq!(rule.daily_checkin, "14:00");
        assert_eq!(rule.daily_checkout, "12:00");
        assert_eq!(rule.early_checkin_surcharge_pct, 30.0);
        assert_eq!(rule.late_checkout_surcharge_pct, 30.0);
        assert_eq!(rule.weekend_uplift_pct, 20.0);
    }

    #[test]
    fn calculate_from_loaded_inputs_applies_special_uplift() {
        let mut inputs = sample_inputs();
        inputs.fallback_base_price = Some(500_000);
        inputs.check_in = "2026-04-20T10:00:00+07:00".to_string();
        inputs.check_out = "2026-04-22T10:00:00+07:00".to_string();
        inputs.special_uplift_pct = 10.0;

        let pricing = calculate_from_loaded_inputs(&inputs).unwrap();

        assert_eq!(pricing.pricing_type, "nightly");
        assert_eq!(pricing.base_amount, 1_000_000);
        assert_eq!(pricing.weekend_amount, 0);
        assert_eq!(pricing.surcharge_amount, 100_000);
        assert_eq!(pricing.total, 1_100_000);
        assert_eq!(pricing.breakdown.len(), 2);
        assert_eq!(pricing.breakdown[0].amount, 1_000_000);
        assert_eq!(pricing.breakdown[1].amount, 100_000);
    }

    #[test]
    fn calculate_from_loaded_inputs_maps_invalid_datetime_errors() {
        let mut inputs = sample_inputs();
        inputs.fallback_base_price = Some(500_000);
        inputs.check_in = "not-a-datetime".to_string();

        let error = calculate_from_loaded_inputs(&inputs).unwrap_err();

        assert!(matches!(
            error,
            BookingError::DateTimeParse(message) if message.contains("Invalid check-in datetime")
        ));
    }

    /// Byte 10 của chuỗi rơi giữa ký tự nhiều byte (emoji) — trước đây
    /// `&value[..10]` panic "byte index 10 is not a char boundary"; nay
    /// `get(..10)` trả `None` và hàm phải trả lỗi thay vì sập.
    #[test]
    fn nights_between_rejects_check_in_with_non_char_boundary_byte_ten_without_panicking() {
        let error = nights_between("2026-04-2\u{1F600}x", "2026-04-22").unwrap_err();

        assert!(matches!(error, BookingError::DateTimeParse(_)));
    }

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

    /// Một mốc khách hàng gõ nhầm (999_999_999) nhân với phụ thu 1 triệu, 30
    /// đêm, cho ra khoảng 3e16 — vượt xa `MAX_TRANSPORT_SAFE_MONEY_VND`
    /// (~9.007e15), mốc mà một số JS còn biểu diễn chính xác được. Trước bản
    /// vá này, `extra_guest_charge` dùng `checked_mul`/`checked_add` thô: cả
    /// hai phép tính đều "thành công" theo nghĩa không tràn số nguyên i64,
    /// nên giá trị méo mó vẫn được ghi vào `bookings.total_price` và trả về
    /// UI. Bài test này chứng minh giờ nó trả lỗi thay vì âm thầm ghi số sai.
    #[test]
    fn calculate_from_loaded_inputs_rejects_extra_guest_charge_beyond_transport_safe_ceiling() {
        let mut inputs = sample_inputs();
        inputs.fallback_base_price = Some(500_000);
        inputs.check_in = "2026-04-20".to_string();
        inputs.check_out = "2026-05-20".to_string(); // 30 đêm
        inputs.guests = Some(999_999_999);
        inputs.base_guests = 2;
        inputs.extra_person_fee = 1_000_000;

        let error = calculate_from_loaded_inputs(&inputs).unwrap_err();

        assert!(matches!(error, BookingError::Validation(_)));
    }

    /// Bản thân phụ thu (trước khi cộng vào tổng) cũng phải bị chặn nếu nó
    /// một mình đã vượt trần — không chỉ khi cộng dồn với `base_amount` mới lộ ra.
    #[test]
    fn calculate_from_loaded_inputs_rejects_when_extra_amount_alone_exceeds_the_ceiling() {
        let mut inputs = sample_inputs();
        inputs.fallback_base_price = Some(1);
        inputs.check_in = "2026-04-20".to_string();
        inputs.check_out = "2026-04-21".to_string(); // 1 đêm
        inputs.guests = Some(2);
        inputs.base_guests = 0;
        inputs.extra_person_fee = MAX_TRANSPORT_SAFE_MONEY_VND;

        let error = calculate_from_loaded_inputs(&inputs).unwrap_err();

        assert!(matches!(error, BookingError::Validation(_)));
    }
}
