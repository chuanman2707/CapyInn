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
    if !pct.is_finite() {
        return Err(CommandError::user(
            codes::VALIDATION_INVALID_INPUT,
            format!("{field} percentage must be finite"),
        ));
    }

    let scaled_pct = (pct * 1_000_000.0).round() as i128;
    let numerator = base as i128 * scaled_pct;
    let denominator = 100_i128 * 1_000_000_i128;
    let rounded = round_ratio_half_away_from_zero(numerator, denominator);
    if rounded < MoneyVnd::MIN as i128 || rounded > MoneyVnd::MAX as i128 {
        return Err(CommandError::user(
            codes::VALIDATION_INVALID_INPUT,
            format!("{field} percentage calculation overflowed"),
        ));
    }
    validate_transport_money_vnd(rounded as MoneyVnd, field)
}

fn round_ratio_half_away_from_zero(numerator: i128, denominator: i128) -> i128 {
    debug_assert!(denominator > 0);
    let sign = if numerator < 0 { -1 } else { 1 };
    let abs = numerator.abs();
    let quotient = abs / denominator;
    let remainder = abs % denominator;
    let rounded = if remainder * 2 >= denominator {
        quotient + 1
    } else {
        quotient
    };
    rounded * sign
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
}
