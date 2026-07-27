//! Pure stay-pricing rules.
//!
//! This module owns the decision of *what a stay costs given its inputs*, and
//! nothing else. Loading those inputs is `queries::booking::pricing_queries`;
//! sequencing load-then-calculate is `services::booking::pricing_service`.
//! Keeping the rules free of SQL is what makes them testable without a
//! database, and it is enforced by `architecture_guard`.

use super::{BookingError, BookingResult};
use crate::money::MoneyVnd;

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

pub(crate) fn calculate_from_loaded_inputs(
    inputs: &StayPricingInputs,
) -> BookingResult<crate::pricing::PricingResult> {
    let rule = build_effective_pricing_rule(inputs);

    crate::pricing::calculate_price(
        &rule,
        &inputs.check_in,
        &inputs.check_out,
        &inputs.pricing_type,
        inputs.special_uplift_pct,
    )
    .map_err(BookingError::datetime_parse)
}

#[cfg(test)]
mod tests {
    use super::{
        build_effective_pricing_rule, calculate_from_loaded_inputs, StayPricingInputs,
        StoredPricingRule,
    };
    use crate::domain::booking::BookingError;

    fn sample_inputs() -> StayPricingInputs {
        StayPricingInputs {
            room_type: "standard".to_string(),
            stored_rule: None,
            fallback_base_price: None,
            special_uplift_pct: 0.0,
            check_in: "2026-04-20".to_string(),
            check_out: "2026-04-22".to_string(),
            pricing_type: "nightly".to_string(),
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
}
