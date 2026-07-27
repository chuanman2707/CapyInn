//! Pricing-configuration writes.
//!
//! Both are upserts keyed on the natural business key — one rule per room type,
//! one uplift per date — so saving twice replaces rather than duplicating.

use sqlx::{Pool, Sqlite};

use crate::money::MoneyVnd;

/// Already-validated and already-defaulted values. Deciding what a missing
/// `overnight_start` should be is the service's job, not the writer's.
pub struct PricingRuleUpsert {
    pub id: String,
    pub room_type: String,
    pub hourly_rate: MoneyVnd,
    pub overnight_rate: MoneyVnd,
    pub daily_rate: MoneyVnd,
    pub overnight_start: String,
    pub overnight_end: String,
    pub daily_checkin: String,
    pub daily_checkout: String,
    pub early_checkin_surcharge_pct: f64,
    pub late_checkout_surcharge_pct: f64,
    pub weekend_uplift_pct: f64,
    pub now: String,
}

pub async fn upsert_pricing_rule(
    pool: &Pool<Sqlite>,
    rule: &PricingRuleUpsert,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO pricing_rules
         (id, room_type, hourly_rate, overnight_rate, daily_rate,
          overnight_start, overnight_end, daily_checkin, daily_checkout,
          early_checkin_surcharge_pct, late_checkout_surcharge_pct,
          weekend_uplift_pct, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(room_type) DO UPDATE SET
            hourly_rate = excluded.hourly_rate,
            overnight_rate = excluded.overnight_rate,
            daily_rate = excluded.daily_rate,
            overnight_start = excluded.overnight_start,
            overnight_end = excluded.overnight_end,
            daily_checkin = excluded.daily_checkin,
            daily_checkout = excluded.daily_checkout,
            early_checkin_surcharge_pct = excluded.early_checkin_surcharge_pct,
            late_checkout_surcharge_pct = excluded.late_checkout_surcharge_pct,
            weekend_uplift_pct = excluded.weekend_uplift_pct,
            updated_at = excluded.updated_at",
    )
    .bind(&rule.id)
    .bind(&rule.room_type)
    .bind(rule.hourly_rate)
    .bind(rule.overnight_rate)
    .bind(rule.daily_rate)
    .bind(&rule.overnight_start)
    .bind(&rule.overnight_end)
    .bind(&rule.daily_checkin)
    .bind(&rule.daily_checkout)
    .bind(rule.early_checkin_surcharge_pct)
    .bind(rule.late_checkout_surcharge_pct)
    .bind(rule.weekend_uplift_pct)
    .bind(&rule.now)
    .bind(&rule.now)
    .execute(pool)
    .await?;

    Ok(())
}

/// `created_at` is only written on insert; an update leaves the original date in
/// place, which is why it is not in the `DO UPDATE SET` list.
pub async fn upsert_special_date(
    pool: &Pool<Sqlite>,
    id: &str,
    date: &str,
    label: &str,
    uplift_pct: f64,
    now: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO special_dates (id, date, label, uplift_pct, created_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(date) DO UPDATE SET
            label = excluded.label,
            uplift_pct = excluded.uplift_pct",
    )
    .bind(id)
    .bind(date)
    .bind(label)
    .bind(uplift_pct)
    .bind(now)
    .execute(pool)
    .await?;

    Ok(())
}
