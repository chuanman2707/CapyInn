use sqlx::{Pool, Row, Sqlite, Transaction};

use super::{BookingError, BookingResult};

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct StayPricingInputs {
    room_type: String,
    stored_rule: Option<StoredPricingRule>,
    fallback_base_price: Option<f64>,
    special_uplift_pct: f64,
    check_in: String,
    check_out: String,
    pricing_type: String,
}

#[derive(Debug, Clone)]
struct StoredPricingRule {
    room_type: String,
    hourly_rate: f64,
    overnight_rate: f64,
    daily_rate: f64,
    overnight_start: String,
    overnight_end: String,
    daily_checkin: String,
    daily_checkout: String,
    early_checkin_surcharge_pct: f64,
    late_checkout_surcharge_pct: f64,
    weekend_uplift_pct: f64,
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

fn build_effective_pricing_rule(inputs: &StayPricingInputs) -> crate::pricing::PricingRule {
    if let Some(stored_rule) = &inputs.stored_rule {
        return stored_rule.to_pricing_rule();
    }

    let fallback_price = inputs.fallback_base_price.unwrap_or(350_000.0);

    crate::pricing::PricingRule {
        room_type: inputs.room_type.clone(),
        hourly_rate: fallback_price / 5.0,
        overnight_rate: fallback_price * 0.75,
        daily_rate: fallback_price,
        ..Default::default()
    }
}

#[allow(dead_code)]
fn calculate_from_loaded_inputs(
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

fn stored_rule_from_row(row: &sqlx::sqlite::SqliteRow) -> StoredPricingRule {
    StoredPricingRule {
        room_type: row.get("room_type"),
        hourly_rate: read_f64(row, "hourly_rate"),
        overnight_rate: read_f64(row, "overnight_rate"),
        daily_rate: read_f64(row, "daily_rate"),
        overnight_start: row.get("overnight_start"),
        overnight_end: row.get("overnight_end"),
        daily_checkin: row.get("daily_checkin"),
        daily_checkout: row.get("daily_checkout"),
        early_checkin_surcharge_pct: read_f64(row, "early_checkin_surcharge_pct"),
        late_checkout_surcharge_pct: read_f64(row, "late_checkout_surcharge_pct"),
        weekend_uplift_pct: read_f64(row, "weekend_uplift_pct"),
    }
}

#[allow(dead_code)]
pub async fn calculate_stay_price(
    pool: &Pool<Sqlite>,
    room_id: &str,
    check_in: &str,
    check_out: &str,
    pricing_type: &str,
) -> BookingResult<crate::pricing::PricingResult> {
    let room_type = load_room_type(pool, room_id).await?;
    let rule = load_pricing_rule(pool, &room_type).await?;
    let special_uplift = load_special_uplift(pool, check_in).await?;

    crate::pricing::calculate_price(&rule, check_in, check_out, pricing_type, special_uplift)
        .map_err(BookingError::datetime_parse)
}
pub async fn calculate_stay_price_tx(
    tx: &mut Transaction<'_, Sqlite>,
    room_id: &str,
    check_in: &str,
    check_out: &str,
    pricing_type: &str,
) -> BookingResult<crate::pricing::PricingResult> {
    let room_type = load_room_type_tx(tx, room_id).await?;
    let rule = load_pricing_rule_tx(tx, &room_type).await?;
    let special_uplift = load_special_uplift_tx(tx, check_in).await?;

    crate::pricing::calculate_price(&rule, check_in, check_out, pricing_type, special_uplift)
        .map_err(BookingError::datetime_parse)
}

#[allow(dead_code)]
async fn load_room_type(pool: &Pool<Sqlite>, room_id: &str) -> BookingResult<String> {
    sqlx::query_scalar::<_, String>("SELECT type FROM rooms WHERE id = ? LIMIT 1")
        .bind(room_id)
        .fetch_optional(pool)
        .await
        .map_err(|error| BookingError::database(error.to_string()))?
        .ok_or_else(|| BookingError::not_found(format!("Không tìm thấy phòng {}", room_id)))
}

async fn load_room_type_tx(
    tx: &mut Transaction<'_, Sqlite>,
    room_id: &str,
) -> BookingResult<String> {
    sqlx::query_scalar::<_, String>("SELECT type FROM rooms WHERE id = ? LIMIT 1")
        .bind(room_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|error| BookingError::database(error.to_string()))?
        .ok_or_else(|| BookingError::not_found(format!("Không tìm thấy phòng {}", room_id)))
}

#[allow(dead_code)]
async fn load_pricing_rule(
    pool: &Pool<Sqlite>,
    room_type: &str,
) -> BookingResult<crate::pricing::PricingRule> {
    let room_type_lower = room_type.to_lowercase();
    let row = sqlx::query(
        "SELECT room_type, hourly_rate, overnight_rate, daily_rate,
                overnight_start, overnight_end, daily_checkin, daily_checkout,
                early_checkin_surcharge_pct, late_checkout_surcharge_pct,
                weekend_uplift_pct
         FROM pricing_rules WHERE LOWER(room_type) = ?",
    )
    .bind(&room_type_lower)
    .fetch_optional(pool)
    .await
    .map_err(|error| BookingError::database(error.to_string()))?;

    let fallback_row = sqlx::query("SELECT base_price FROM rooms WHERE LOWER(type) = ? LIMIT 1")
        .bind(&room_type_lower)
        .fetch_optional(pool)
        .await
        .map_err(|error| BookingError::database(error.to_string()))?;

    let inputs = StayPricingInputs {
        room_type: room_type.to_string(),
        stored_rule: row.as_ref().map(stored_rule_from_row),
        fallback_base_price: fallback_row.as_ref().map(|row| read_f64(row, "base_price")),
        special_uplift_pct: 0.0,
        check_in: String::new(),
        check_out: String::new(),
        pricing_type: String::new(),
    };

    Ok(build_effective_pricing_rule(&inputs))
}

async fn load_pricing_rule_tx(
    tx: &mut Transaction<'_, Sqlite>,
    room_type: &str,
) -> BookingResult<crate::pricing::PricingRule> {
    let room_type_lower = room_type.to_lowercase();
    let row = sqlx::query(
        "SELECT room_type, hourly_rate, overnight_rate, daily_rate,
                overnight_start, overnight_end, daily_checkin, daily_checkout,
                early_checkin_surcharge_pct, late_checkout_surcharge_pct,
                weekend_uplift_pct
         FROM pricing_rules WHERE LOWER(room_type) = ?",
    )
    .bind(&room_type_lower)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|error| BookingError::database(error.to_string()))?;

    let fallback_row = sqlx::query("SELECT base_price FROM rooms WHERE LOWER(type) = ? LIMIT 1")
        .bind(&room_type_lower)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|error| BookingError::database(error.to_string()))?;

    let inputs = StayPricingInputs {
        room_type: room_type.to_string(),
        stored_rule: row.as_ref().map(stored_rule_from_row),
        fallback_base_price: fallback_row.as_ref().map(|row| read_f64(row, "base_price")),
        special_uplift_pct: 0.0,
        check_in: String::new(),
        check_out: String::new(),
        pricing_type: String::new(),
    };

    Ok(build_effective_pricing_rule(&inputs))
}

#[allow(dead_code)]
async fn load_special_uplift(pool: &Pool<Sqlite>, date_str: &str) -> BookingResult<f64> {
    let date = if date_str.len() >= 10 {
        &date_str[..10]
    } else {
        date_str
    };
    let row: Option<(f64,)> =
        sqlx::query_as("SELECT CAST(uplift_pct AS REAL) FROM special_dates WHERE date = ?")
            .bind(date)
            .fetch_optional(pool)
            .await
            .map_err(|error| BookingError::database(error.to_string()))?;

    Ok(row.map(|value| value.0).unwrap_or(0.0))
}

async fn load_special_uplift_tx(
    tx: &mut Transaction<'_, Sqlite>,
    date_str: &str,
) -> BookingResult<f64> {
    let date = if date_str.len() >= 10 {
        &date_str[..10]
    } else {
        date_str
    };
    let row: Option<(f64,)> =
        sqlx::query_as("SELECT CAST(uplift_pct AS REAL) FROM special_dates WHERE date = ?")
            .bind(date)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|error| BookingError::database(error.to_string()))?;

    Ok(row.map(|value| value.0).unwrap_or(0.0))
}

fn read_f64(row: &sqlx::sqlite::SqliteRow, column: &str) -> f64 {
    row.try_get::<f64, _>(column)
        .unwrap_or_else(|_| row.get::<i64, _>(column) as f64)
}

#[cfg(test)]
mod tests {
    use super::{build_effective_pricing_rule, StayPricingInputs, StoredPricingRule};

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
        inputs.fallback_base_price = Some(500_000.0);
        inputs.stored_rule = Some(StoredPricingRule {
            room_type: "deluxe".to_string(),
            hourly_rate: 120_000.0,
            overnight_rate: 420_000.0,
            daily_rate: 560_000.0,
            overnight_start: "21:00".to_string(),
            overnight_end: "10:00".to_string(),
            daily_checkin: "13:00".to_string(),
            daily_checkout: "11:00".to_string(),
            early_checkin_surcharge_pct: 15.0,
            late_checkout_surcharge_pct: 25.0,
            weekend_uplift_pct: 18.0,
        });

        let rule = build_effective_pricing_rule(&inputs);

        assert_eq!(rule.room_type, "deluxe");
        assert_eq!(rule.hourly_rate, 120_000.0);
        assert_eq!(rule.overnight_rate, 420_000.0);
        assert_eq!(rule.daily_rate, 560_000.0);
        assert_eq!(rule.overnight_start, "21:00");
        assert_eq!(rule.overnight_end, "10:00");
        assert_eq!(rule.daily_checkin, "13:00");
        assert_eq!(rule.daily_checkout, "11:00");
        assert_eq!(rule.early_checkin_surcharge_pct, 15.0);
        assert_eq!(rule.late_checkout_surcharge_pct, 25.0);
        assert_eq!(rule.weekend_uplift_pct, 18.0);
    }

    #[test]
    fn build_effective_pricing_rule_derives_fallback_rates_from_base_price() {
        let mut inputs = sample_inputs();
        inputs.fallback_base_price = Some(500_000.0);

        let rule = build_effective_pricing_rule(&inputs);
        let defaults = crate::pricing::PricingRule::default();

        assert_eq!(rule.room_type, "standard");
        assert_eq!(rule.hourly_rate, 100_000.0);
        assert_eq!(rule.overnight_rate, 375_000.0);
        assert_eq!(rule.daily_rate, 500_000.0);
        assert_eq!(rule.overnight_start, defaults.overnight_start);
        assert_eq!(rule.overnight_end, defaults.overnight_end);
        assert_eq!(rule.daily_checkin, defaults.daily_checkin);
        assert_eq!(rule.daily_checkout, defaults.daily_checkout);
        assert_eq!(
            rule.early_checkin_surcharge_pct,
            defaults.early_checkin_surcharge_pct
        );
        assert_eq!(
            rule.late_checkout_surcharge_pct,
            defaults.late_checkout_surcharge_pct
        );
        assert_eq!(rule.weekend_uplift_pct, defaults.weekend_uplift_pct);
    }

    #[test]
    fn build_effective_pricing_rule_uses_default_price_and_metadata_when_base_price_missing() {
        let rule = build_effective_pricing_rule(&sample_inputs());
        let defaults = crate::pricing::PricingRule::default();

        assert_eq!(rule.room_type, "standard");
        assert_eq!(rule.hourly_rate, 70_000.0);
        assert_eq!(rule.overnight_rate, 262_500.0);
        assert_eq!(rule.daily_rate, 350_000.0);
        assert_eq!(rule.overnight_start, defaults.overnight_start);
        assert_eq!(rule.overnight_end, defaults.overnight_end);
        assert_eq!(rule.daily_checkin, defaults.daily_checkin);
        assert_eq!(rule.daily_checkout, defaults.daily_checkout);
        assert_eq!(
            rule.early_checkin_surcharge_pct,
            defaults.early_checkin_surcharge_pct
        );
        assert_eq!(
            rule.late_checkout_surcharge_pct,
            defaults.late_checkout_surcharge_pct
        );
        assert_eq!(rule.weekend_uplift_pct, defaults.weekend_uplift_pct);
    }
}
