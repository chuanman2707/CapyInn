use crate::app_error::{codes, CommandError, CommandResult};

pub type MoneyVnd = i64;

pub const MIN_TRANSPORT_SAFE_MONEY_VND: MoneyVnd = -9_007_199_254_740_991;
pub const MAX_TRANSPORT_SAFE_MONEY_VND: MoneyVnd = 9_007_199_254_740_991;

pub fn validate_transport_money_vnd(value: MoneyVnd, field: &str) -> CommandResult<MoneyVnd> {
    if !(MIN_TRANSPORT_SAFE_MONEY_VND..=MAX_TRANSPORT_SAFE_MONEY_VND).contains(&value) {
        return Err(CommandError::user(
            codes::VALIDATION_INVALID_INPUT,
            format!("{field} must be a safe integer VND value"),
        ));
    }
    Ok(value)
}

pub fn validate_non_negative_money_vnd(value: MoneyVnd, field: &str) -> CommandResult<MoneyVnd> {
    let value = validate_transport_money_vnd(value, field)?;
    if value < 0 {
        return Err(CommandError::user(
            codes::VALIDATION_INVALID_INPUT,
            format!("{field} must be greater than or equal to 0"),
        ));
    }
    Ok(value)
}

pub fn percentage_money_line(base: MoneyVnd, pct: f64, field: &str) -> CommandResult<MoneyVnd> {
    let base = validate_transport_money_vnd(base, field)?;

    if !pct.is_finite() {
        return Err(CommandError::user(
            codes::VALIDATION_INVALID_INPUT,
            format!("{field} percentage must be finite"),
        ));
    }

    let scaled_pct_float = (pct * 1_000_000.0).round();
    if !scaled_pct_float.is_finite()
        || scaled_pct_float <= i128::MIN as f64
        || scaled_pct_float >= i128::MAX as f64
    {
        return Err(CommandError::user(
            codes::VALIDATION_INVALID_INPUT,
            format!("{field} percentage calculation overflowed"),
        ));
    }

    let scaled_pct = scaled_pct_float as i128;
    let numerator = (base as i128).checked_mul(scaled_pct).ok_or_else(|| {
        CommandError::user(
            codes::VALIDATION_INVALID_INPUT,
            format!("{field} percentage calculation overflowed"),
        )
    })?;
    let denominator = 100_i128 * 1_000_000_i128;
    let rounded = round_ratio_half_away_from_zero(numerator, denominator).ok_or_else(|| {
        CommandError::user(
            codes::VALIDATION_INVALID_INPUT,
            format!("{field} percentage calculation overflowed"),
        )
    })?;
    if rounded < MoneyVnd::MIN as i128 || rounded > MoneyVnd::MAX as i128 {
        return Err(CommandError::user(
            codes::VALIDATION_INVALID_INPUT,
            format!("{field} percentage calculation overflowed"),
        ));
    }
    validate_transport_money_vnd(rounded as MoneyVnd, field)
}

fn round_ratio_half_away_from_zero(numerator: i128, denominator: i128) -> Option<i128> {
    debug_assert!(denominator > 0);
    let sign = if numerator < 0 { -1 } else { 1 };
    let abs = numerator.checked_abs()?;
    let quotient = abs / denominator;
    let remainder = abs % denominator;
    let rounded = if remainder * 2 >= denominator {
        quotient + 1
    } else {
        quotient
    };
    Some(rounded * sign)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_transport_money_accepts_safe_integer() {
        assert_eq!(
            validate_transport_money_vnd(500_000, "amount").unwrap(),
            500_000
        );
    }

    #[test]
    fn validate_transport_money_rejects_unsafe_integer() {
        let error = validate_transport_money_vnd(MAX_TRANSPORT_SAFE_MONEY_VND + 1, "amount")
            .expect_err("unsafe integer must fail");
        assert_eq!(error.code, codes::VALIDATION_INVALID_INPUT);
        assert_eq!(error.kind, crate::app_error::AppErrorKind::User);
        assert!(error.message.contains("amount"));
    }

    #[test]
    fn validate_non_negative_money_rejects_negative_value() {
        let error = validate_non_negative_money_vnd(-1, "deposit_amount")
            .expect_err("negative deposit must fail");
        assert!(error.message.contains("deposit_amount"));
    }

    #[test]
    fn percentage_money_rounds_half_away_from_zero() {
        assert_eq!(
            percentage_money_line(333_333, 10.0, "surcharge").unwrap(),
            33_333
        );
        assert_eq!(percentage_money_line(5, 10.0, "surcharge").unwrap(), 1);
        assert_eq!(percentage_money_line(-5, 10.0, "adjustment").unwrap(), -1);
    }

    #[test]
    fn percentage_money_rejects_too_large_percentage_without_panic() {
        let error = percentage_money_line(2, f64::MAX, "surcharge")
            .expect_err("too-large percentage must fail");
        assert!(error.message.contains("surcharge"));
    }

    #[test]
    fn percentage_money_rejects_non_finite_percentage() {
        let nan_error = percentage_money_line(100, f64::NAN, "surcharge")
            .expect_err("NaN percentage must fail");
        assert!(nan_error.message.contains("surcharge"));

        let infinity_error = percentage_money_line(100, f64::INFINITY, "surcharge")
            .expect_err("infinite percentage must fail");
        assert!(infinity_error.message.contains("surcharge"));
    }

    #[test]
    fn percentage_money_rejects_result_outside_transport_safe_range() {
        let error = percentage_money_line(MAX_TRANSPORT_SAFE_MONEY_VND, 200.0, "surcharge")
            .expect_err("out-of-range result must fail");
        assert!(error.message.contains("surcharge"));
    }

    #[test]
    fn percentage_money_rejects_unsafe_base_before_arithmetic() {
        let pct = (u64::MAX as f64 + 1.0) / 1_000_000.0;
        let error = percentage_money_line(MoneyVnd::MIN, pct, "base_amount")
            .expect_err("unsafe base must fail");
        assert!(error.message.contains("base_amount"));
    }

    #[test]
    fn percentage_money_rejects_min_i128_rounding_overflow_without_panic() {
        let pct = ((1_i128 << 126) as f64) / 1_000_000.0;
        let error =
            percentage_money_line(-2, pct, "adjustment").expect_err("rounding overflow must fail");
        assert!(error.message.contains("adjustment"));
    }
}
