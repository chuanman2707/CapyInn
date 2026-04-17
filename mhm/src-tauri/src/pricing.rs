use chrono::{Datelike, NaiveDateTime, NaiveTime, Weekday};
use serde::{Deserialize, Serialize};

// ═══════════════════════════════════════════════
// VN Hotel Pricing Engine — Pure Rust
// ═══════════════════════════════════════════════
//
// Pricing models for Vietnamese hotels:
// 1. HOURLY (theo giờ): charged per hour, common for short stays
// 2. OVERNIGHT (qua đêm): fixed rate for overnight stays (22:00 - 11:00)
// 3. DAILY (theo ngày): check-in 14:00, check-out 12:00 next day
//
// Capping: if hourly total exceeds overnight/daily, auto-cap to the lower rate
// Surcharges: early check-in or late check-out adds percentage surcharge

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingRule {
    pub room_type: String,
    pub hourly_rate: f64,
    pub overnight_rate: f64,
    pub daily_rate: f64,
    pub overnight_start: String,          // "22:00"
    pub overnight_end: String,            // "11:00"
    pub daily_checkin: String,            // "14:00"
    pub daily_checkout: String,           // "12:00"
    pub early_checkin_surcharge_pct: f64, // % surcharge
    pub late_checkout_surcharge_pct: f64, // % surcharge
    pub weekend_uplift_pct: f64,          // % uplift for weekend
}

impl Default for PricingRule {
    fn default() -> Self {
        Self {
            room_type: "standard".to_string(),
            hourly_rate: 80_000.0,
            overnight_rate: 300_000.0,
            daily_rate: 400_000.0,
            overnight_start: "22:00".to_string(),
            overnight_end: "11:00".to_string(),
            daily_checkin: "14:00".to_string(),
            daily_checkout: "12:00".to_string(),
            early_checkin_surcharge_pct: 30.0,
            late_checkout_surcharge_pct: 30.0,
            weekend_uplift_pct: 20.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingResult {
    pub pricing_type: String,  // "hourly" | "overnight" | "daily" | "nightly"
    pub base_amount: f64,      // before surcharges
    pub surcharge_amount: f64, // early/late surcharges
    pub weekend_amount: f64,   // weekend uplift
    pub total: f64,            // final price
    pub breakdown: Vec<PricingLine>, // itemized breakdown
    pub capped: bool,          // was hourly capped to overnight/daily?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingLine {
    pub label: String,
    pub amount: f64,
}

// ─── Public API ───

/// Calculate the price for a stay
/// `check_in` and `check_out` are ISO 8601 datetime strings
pub fn calculate_price(
    rule: &PricingRule,
    check_in: &str,
    check_out: &str,
    pricing_type: &str,
    special_dates_uplift: f64,
) -> PricingResult {
    let zero = PricingResult {
        pricing_type: pricing_type.to_string(),
        base_amount: 0.0,
        surcharge_amount: 0.0,
        weekend_amount: 0.0,
        total: 0.0,
        breakdown: vec![],
        capped: false,
    };

    let ci_has_time = has_explicit_time(check_in);
    let co_has_time = has_explicit_time(check_out);
    let Some(ci) = parse_datetime(check_in) else { return zero; };
    let Some(co) = parse_datetime(check_out) else { return zero; };

    if co <= ci {
        return zero;
    }

    match pricing_type {
        "hourly" => calculate_hourly(rule, ci, co, special_dates_uplift),
        "overnight" => {
            calculate_overnight(rule, ci, co, special_dates_uplift, ci_has_time, co_has_time)
        }
        "daily" => calculate_daily(rule, ci, co, special_dates_uplift, ci_has_time, co_has_time),
        _ => calculate_nightly(rule, ci, co, special_dates_uplift),
    }
}

/// Legacy nightly calculation: base_price × nights (backward compatible)
fn calculate_nightly(
    rule: &PricingRule,
    ci: NaiveDateTime,
    co: NaiveDateTime,
    special_dates_uplift: f64,
) -> PricingResult {
    let nights = (co.date() - ci.date()).num_days().max(1) as f64;
    let base = rule.daily_rate * nights;
    let weekend = calculate_weekend_uplift(rule, ci, co);
    let special = base * special_dates_uplift / 100.0;

    let total = base + weekend + special;

    PricingResult {
        pricing_type: "nightly".to_string(),
        base_amount: base,
        surcharge_amount: special,
        weekend_amount: weekend,
        total,
        breakdown: vec![
            PricingLine {
                label: format!("{} night(s) x {}", nights, fmt_vnd(rule.daily_rate)),
                amount: base,
            },
            if weekend > 0.0 {
                PricingLine {
                    label: "Weekend surcharge".into(),
                    amount: weekend,
                }
            } else {
                PricingLine {
                    label: String::new(),
                    amount: 0.0,
                }
            },
            if special > 0.0 {
                PricingLine {
                    label: "Holiday surcharge".into(),
                    amount: special,
                }
            } else {
                PricingLine {
                    label: String::new(),
                    amount: 0.0,
                }
            },
        ]
        .into_iter()
        .filter(|l| l.amount > 0.0)
        .collect(),
        capped: false,
    }
}

/// Hourly pricing with auto-capping
fn calculate_hourly(
    rule: &PricingRule,
    ci: NaiveDateTime,
    co: NaiveDateTime,
    special_dates_uplift: f64,
) -> PricingResult {
    let duration_hours = (co - ci).num_minutes() as f64 / 60.0;
    let hours = duration_hours.ceil().max(1.0);
    let raw_hourly = rule.hourly_rate * hours;

    // Capping: if hourly exceeds overnight, cap at overnight
    let (base, capped, cap_type) = if raw_hourly > rule.overnight_rate && duration_hours <= 13.0 {
        (rule.overnight_rate, true, "overnight")
    } else if raw_hourly > rule.daily_rate {
        // If exceeds daily rate, calculate as daily
        let days = (duration_hours / 24.0).ceil().max(1.0);
        let daily_total = rule.daily_rate * days;
        if raw_hourly > daily_total {
            (daily_total, true, "daily")
        } else {
            (raw_hourly, false, "hourly")
        }
    } else {
        (raw_hourly, false, "hourly")
    };

    let weekend = calculate_weekend_uplift(rule, ci, co);
    let special = base * special_dates_uplift / 100.0;
    let total = base + weekend + special;

    let mut breakdown = vec![];
    if capped {
        breakdown.push(PricingLine {
            label: format!(
                "{}h x {} = {} -> Capped to {} rate",
                hours,
                fmt_vnd(rule.hourly_rate),
                fmt_vnd(raw_hourly),
                cap_type
            ),
            amount: base,
        });
    } else {
        breakdown.push(PricingLine {
            label: format!("{}h x {}", hours, fmt_vnd(rule.hourly_rate)),
            amount: base,
        });
    }
    if weekend > 0.0 {
        breakdown.push(PricingLine {
            label: "Weekend surcharge".into(),
            amount: weekend,
        });
    }
    if special > 0.0 {
        breakdown.push(PricingLine {
            label: "Holiday surcharge".into(),
            amount: special,
        });
    }

    PricingResult {
        pricing_type: if capped {
            cap_type.to_string()
        } else {
            "hourly".to_string()
        },
        base_amount: base,
        surcharge_amount: special,
        weekend_amount: weekend,
        total,
        breakdown,
        capped,
    }
}

/// Overnight pricing: fixed rate + early/late surcharges
/// `ci_has_time`/`co_has_time`: whether the original input had an explicit time.
/// When no time was provided, surcharges are skipped (unknown arrival time).
fn calculate_overnight(
    rule: &PricingRule,
    ci: NaiveDateTime,
    co: NaiveDateTime,
    special_dates_uplift: f64,
    ci_has_time: bool,
    co_has_time: bool,
) -> PricingResult {
    let ci_time = ci.time();
    let co_time = co.time();
    let overnight_start = parse_time(&rule.overnight_start);
    let overnight_end = parse_time(&rule.overnight_end);

    let nights = {
        let days = (co.date() - ci.date()).num_days();
        if days == 0 {
            1
        } else {
            days
        }
    } as f64;

    let base = rule.overnight_rate * nights;
    let mut surcharge = 0.0;
    let mut breakdown = vec![PricingLine {
        label: format!("{} night(s) x {}", nights, fmt_vnd(rule.overnight_rate)),
        amount: base,
    }];

    // Early check-in surcharge: only when explicit time is known
    if ci_has_time && ci_time < overnight_start {
        let early_amount = base * rule.early_checkin_surcharge_pct / 100.0;
        surcharge += early_amount;
        breakdown.push(PricingLine {
            label: format!(
                "Early check-in surcharge ({}%)",
                rule.early_checkin_surcharge_pct
            ),
            amount: early_amount,
        });
    }

    // Late check-out surcharge: only when explicit time is known
    if co_has_time && co_time > overnight_end {
        let late_amount = base * rule.late_checkout_surcharge_pct / 100.0;
        surcharge += late_amount;
        breakdown.push(PricingLine {
            label: format!(
                "Late check-out surcharge ({}%)",
                rule.late_checkout_surcharge_pct
            ),
            amount: late_amount,
        });
    }

    let weekend = calculate_weekend_uplift(rule, ci, co);
    let special = base * special_dates_uplift / 100.0;

    if weekend > 0.0 {
        breakdown.push(PricingLine {
            label: "Weekend surcharge".into(),
            amount: weekend,
        });
    }
    if special > 0.0 {
        breakdown.push(PricingLine {
            label: "Holiday surcharge".into(),
            amount: special,
        });
    }

    let total = base + surcharge + weekend + special;

    PricingResult {
        pricing_type: "overnight".to_string(),
        base_amount: base,
        surcharge_amount: surcharge + special,
        weekend_amount: weekend,
        total,
        breakdown,
        capped: false,
    }
}

/// Daily pricing: per-day rate
/// `ci_has_time`/`co_has_time`: whether the original input had an explicit time.
/// When no time was provided, surcharges are skipped (unknown arrival time).
fn calculate_daily(
    rule: &PricingRule,
    ci: NaiveDateTime,
    co: NaiveDateTime,
    special_dates_uplift: f64,
    ci_has_time: bool,
    co_has_time: bool,
) -> PricingResult {
    // Count days by calendar date difference (not hours)
    let days = (co.date() - ci.date()).num_days().max(1) as f64;
    let base = rule.daily_rate * days;

    let ci_time = ci.time();
    let co_time = co.time();
    let daily_checkin = parse_time(&rule.daily_checkin);
    let daily_checkout = parse_time(&rule.daily_checkout);

    let mut surcharge = 0.0;
    let mut breakdown = vec![PricingLine {
        label: format!("{} day(s) x {}", days, fmt_vnd(rule.daily_rate)),
        amount: base,
    }];

    // Early check-in surcharge: only when explicit time is known
    if ci_has_time && ci_time < daily_checkin {
        let early_amount = rule.daily_rate * rule.early_checkin_surcharge_pct / 100.0;
        surcharge += early_amount;
        breakdown.push(PricingLine {
            label: format!(
                "Early check-in surcharge ({}%)",
                rule.early_checkin_surcharge_pct
            ),
            amount: early_amount,
        });
    }

    // Late check-out surcharge: only when explicit time is known
    if co_has_time && co_time > daily_checkout {
        let late_amount = rule.daily_rate * rule.late_checkout_surcharge_pct / 100.0;
        surcharge += late_amount;
        breakdown.push(PricingLine {
            label: format!(
                "Late check-out surcharge ({}%)",
                rule.late_checkout_surcharge_pct
            ),
            amount: late_amount,
        });
    }

    let weekend = calculate_weekend_uplift(rule, ci, co);
    let special = base * special_dates_uplift / 100.0;

    if weekend > 0.0 {
        breakdown.push(PricingLine {
            label: "Weekend surcharge".into(),
            amount: weekend,
        });
    }
    if special > 0.0 {
        breakdown.push(PricingLine {
            label: "Holiday surcharge".into(),
            amount: special,
        });
    }

    let total = base + surcharge + weekend + special;

    PricingResult {
        pricing_type: "daily".to_string(),
        base_amount: base,
        surcharge_amount: surcharge + special,
        weekend_amount: weekend,
        total,
        breakdown,
        capped: false,
    }
}

// ─── Helpers ───

fn calculate_weekend_uplift(rule: &PricingRule, ci: NaiveDateTime, co: NaiveDateTime) -> f64 {
    if rule.weekend_uplift_pct <= 0.0 {
        return 0.0;
    }

    let mut date = ci.date();
    let end_date = co.date();
    let mut weekend_days = 0;
    let mut total_days = 0;

    while date < end_date {
        total_days += 1;
        match date.weekday() {
            Weekday::Sat | Weekday::Sun => weekend_days += 1,
            _ => {}
        }
        date = date.succ_opt().unwrap_or(date);
    }

    if total_days == 0 {
        return 0.0;
    }
    let weekend_ratio = weekend_days as f64 / total_days as f64;
    let base_per_day = rule.daily_rate;
    base_per_day * weekend_ratio * rule.weekend_uplift_pct / 100.0 * total_days as f64
}

/// Check if a datetime string contains an explicit time component.
/// Date-only strings like "2026-04-01" return false.
/// Strings with time like "2026-04-01T14:00:00" return true.
fn has_explicit_time(s: &str) -> bool {
    // If it contains 'T' or a space followed by digits:digits, it has a time
    s.contains('T') || s.len() > 10
}

fn parse_datetime(s: &str) -> Option<NaiveDateTime> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.naive_local());
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Some(dt);
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(dt);
    }
    if let Ok(dt) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M") {
        return Some(dt);
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        return Some(d.and_hms_opt(0, 0, 0).unwrap());
    }
    None
}

fn parse_time(s: &str) -> NaiveTime {
    NaiveTime::parse_from_str(s, "%H:%M").unwrap_or(NaiveTime::from_hms_opt(12, 0, 0).unwrap())
}

fn fmt_vnd(amount: f64) -> String {
    let n = amount as i64;
    if n >= 1_000_000 {
        format!("{}M", n / 1_000_000)
    } else if n >= 1_000 {
        format!("{}k", n / 1_000)
    } else {
        format!("{}d", n)
    }
}

// ═══════════════════════════════════════════════
// Unit Tests
// ═══════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    // No weekend uplift for predictable assertions
    fn std_rule() -> PricingRule {
        PricingRule {
            room_type: "standard".to_string(),
            hourly_rate: 80_000.0,
            overnight_rate: 300_000.0,
            daily_rate: 400_000.0,
            overnight_start: "22:00".to_string(),
            overnight_end: "11:00".to_string(),
            daily_checkin: "14:00".to_string(),
            daily_checkout: "12:00".to_string(),
            early_checkin_surcharge_pct: 30.0,
            late_checkout_surcharge_pct: 30.0,
            weekend_uplift_pct: 0.0, // isolated from weekend logic
        }
    }

    // 2026-03-17 = Tuesday, 2026-03-18 = Wednesday (weekdays)

    #[test]
    fn test_hourly_1h() {
        let r = calculate_price(
            &std_rule(),
            "2026-03-17T10:00:00",
            "2026-03-17T11:00:00",
            "hourly",
            0.0,
        );
        assert_eq!(r.total, 80_000.0);
        assert!(!r.capped);
    }

    #[test]
    fn test_hourly_2h() {
        let r = calculate_price(
            &std_rule(),
            "2026-03-17T10:00:00",
            "2026-03-17T12:00:00",
            "hourly",
            0.0,
        );
        assert_eq!(r.total, 160_000.0);
        assert!(!r.capped);
    }

    #[test]
    fn test_hourly_3h() {
        let r = calculate_price(
            &std_rule(),
            "2026-03-17T09:00:00",
            "2026-03-17T12:00:00",
            "hourly",
            0.0,
        );
        assert_eq!(r.total, 240_000.0);
        assert!(!r.capped);
    }

    #[test]
    fn test_hourly_partial_hour_rounds_up() {
        let r = calculate_price(
            &std_rule(),
            "2026-03-17T10:00:00",
            "2026-03-17T11:30:00",
            "hourly",
            0.0,
        );
        assert_eq!(r.total, 160_000.0); // ceil(1.5) = 2h × 80k
    }

    #[test]
    fn test_hourly_capping_to_overnight() {
        // 5h × 80k = 400k > overnight 300k → capped
        let r = calculate_price(
            &std_rule(),
            "2026-03-17T18:00:00",
            "2026-03-17T23:00:00",
            "hourly",
            0.0,
        );
        assert_eq!(r.total, 300_000.0);
        assert!(r.capped);
    }

    #[test]
    fn test_hourly_capping_to_daily() {
        // 20h × 80k = 1600k > daily 400k → capped
        let r = calculate_price(
            &std_rule(),
            "2026-03-17T08:00:00",
            "2026-03-18T04:00:00",
            "hourly",
            0.0,
        );
        assert!(r.capped);
        assert!(r.total <= 400_000.0 * 2.0);
    }

    #[test]
    fn test_overnight_basic() {
        let r = calculate_price(
            &std_rule(),
            "2026-03-17T22:00:00",
            "2026-03-18T11:00:00",
            "overnight",
            0.0,
        );
        assert_eq!(r.base_amount, 300_000.0);
        assert_eq!(r.surcharge_amount, 0.0);
    }

    #[test]
    fn test_overnight_early_checkin() {
        let r = calculate_price(
            &std_rule(),
            "2026-03-17T18:00:00",
            "2026-03-18T11:00:00",
            "overnight",
            0.0,
        );
        assert_eq!(r.base_amount, 300_000.0);
        assert_eq!(r.surcharge_amount, 90_000.0); // 30% of 300k
    }

    #[test]
    fn test_overnight_late_checkout() {
        let r = calculate_price(
            &std_rule(),
            "2026-03-17T22:00:00",
            "2026-03-18T14:00:00",
            "overnight",
            0.0,
        );
        assert_eq!(r.base_amount, 300_000.0);
        assert_eq!(r.surcharge_amount, 90_000.0); // 30% of 300k
    }

    // ── Date-only inputs should NOT trigger surcharges ──

    #[test]
    fn test_daily_date_only_no_surcharge() {
        // Date-only: no explicit time → no surcharge
        let r = calculate_price(&std_rule(), "2026-03-17", "2026-03-18", "daily", 0.0);
        assert_eq!(r.base_amount, 400_000.0);
        assert_eq!(r.surcharge_amount, 0.0); // NO surcharge!
    }

    #[test]
    fn test_overnight_date_only_no_surcharge() {
        let r = calculate_price(&std_rule(), "2026-03-17", "2026-03-18", "overnight", 0.0);
        assert_eq!(r.base_amount, 300_000.0);
        assert_eq!(r.surcharge_amount, 0.0); // NO surcharge!
    }

    #[test]
    fn test_daily_explicit_early_time_has_surcharge() {
        // Explicit early time → surcharge should apply
        let r = calculate_price(
            &std_rule(),
            "2026-03-17T10:00:00",
            "2026-03-18T12:00:00",
            "daily",
            0.0,
        );
        assert_eq!(r.surcharge_amount, 120_000.0); // 30% of 400k
    }

    #[test]
    fn test_daily_1day() {
        let r = calculate_price(
            &std_rule(),
            "2026-03-17T14:00:00",
            "2026-03-18T12:00:00",
            "daily",
            0.0,
        );
        assert_eq!(r.base_amount, 400_000.0);
        assert_eq!(r.surcharge_amount, 0.0);
    }

    #[test]
    fn test_daily_2days() {
        let r = calculate_price(
            &std_rule(),
            "2026-03-17T14:00:00",
            "2026-03-19T12:00:00",
            "daily",
            0.0,
        );
        assert_eq!(r.base_amount, 800_000.0);
    }

    #[test]
    fn test_daily_early_checkin_surcharge() {
        let r = calculate_price(
            &std_rule(),
            "2026-03-17T10:00:00",
            "2026-03-18T12:00:00",
            "daily",
            0.0,
        );
        assert_eq!(r.base_amount, 400_000.0);
        assert_eq!(r.surcharge_amount, 120_000.0); // 30% of 400k
    }

    #[test]
    fn test_daily_late_checkout_surcharge() {
        let r = calculate_price(
            &std_rule(),
            "2026-03-17T14:00:00",
            "2026-03-18T15:00:00",
            "daily",
            0.0,
        );
        assert_eq!(r.base_amount, 400_000.0);
        assert_eq!(r.surcharge_amount, 120_000.0); // 30% of 400k
    }

    #[test]
    fn test_special_date_uplift() {
        let r = calculate_price(
            &std_rule(),
            "2026-03-17T14:00:00",
            "2026-03-18T12:00:00",
            "daily",
            50.0,
        );
        assert_eq!(r.base_amount, 400_000.0);
        assert_eq!(r.surcharge_amount, 200_000.0); // 50% uplift
        assert_eq!(r.total, 600_000.0);
    }

    #[test]
    fn test_nightly_legacy() {
        let r = calculate_price(
            &std_rule(),
            "2026-03-17T14:00:00",
            "2026-03-19T12:00:00",
            "nightly",
            0.0,
        );
        assert_eq!(r.base_amount, 800_000.0); // 2 nights × 400k
    }

    #[test]
    fn test_zero_duration() {
        let r = calculate_price(
            &std_rule(),
            "2026-03-17T14:00:00",
            "2026-03-17T14:00:00",
            "hourly",
            0.0,
        );
        assert_eq!(r.total, 0.0);
    }

    #[test]
    fn test_weekend_uplift() {
        // Sat-Sun: 2026-03-21 = Saturday, 2026-03-22 = Sunday
        let rule = PricingRule {
            weekend_uplift_pct: 20.0,
            ..std_rule()
        };
        let r = calculate_price(
            &rule,
            "2026-03-21T14:00:00",
            "2026-03-22T12:00:00",
            "daily",
            0.0,
        );
        assert_eq!(r.base_amount, 400_000.0);
        assert!(r.weekend_amount > 0.0);
    }
}
