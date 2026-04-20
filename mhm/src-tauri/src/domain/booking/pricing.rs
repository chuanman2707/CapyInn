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

#[allow(dead_code)]
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

    if let Some(row) = row {
        return Ok(crate::pricing::PricingRule {
            room_type: row.get("room_type"),
            hourly_rate: read_f64(&row, "hourly_rate"),
            overnight_rate: read_f64(&row, "overnight_rate"),
            daily_rate: read_f64(&row, "daily_rate"),
            overnight_start: row.get("overnight_start"),
            overnight_end: row.get("overnight_end"),
            daily_checkin: row.get("daily_checkin"),
            daily_checkout: row.get("daily_checkout"),
            early_checkin_surcharge_pct: read_f64(&row, "early_checkin_surcharge_pct"),
            late_checkout_surcharge_pct: read_f64(&row, "late_checkout_surcharge_pct"),
            weekend_uplift_pct: read_f64(&row, "weekend_uplift_pct"),
        });
    }

    let fallback_row = sqlx::query("SELECT base_price FROM rooms WHERE LOWER(type) = ? LIMIT 1")
        .bind(&room_type_lower)
        .fetch_optional(pool)
        .await
        .map_err(|error| BookingError::database(error.to_string()))?;

    let fallback_price = fallback_row
        .as_ref()
        .map(|row| read_f64(row, "base_price"))
        .unwrap_or(350_000.0);

    Ok(crate::pricing::PricingRule {
        room_type: room_type.to_string(),
        hourly_rate: fallback_price / 5.0,
        overnight_rate: fallback_price * 0.75,
        daily_rate: fallback_price,
        ..Default::default()
    })
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

    if let Some(row) = row {
        return Ok(crate::pricing::PricingRule {
            room_type: row.get("room_type"),
            hourly_rate: read_f64(&row, "hourly_rate"),
            overnight_rate: read_f64(&row, "overnight_rate"),
            daily_rate: read_f64(&row, "daily_rate"),
            overnight_start: row.get("overnight_start"),
            overnight_end: row.get("overnight_end"),
            daily_checkin: row.get("daily_checkin"),
            daily_checkout: row.get("daily_checkout"),
            early_checkin_surcharge_pct: read_f64(&row, "early_checkin_surcharge_pct"),
            late_checkout_surcharge_pct: read_f64(&row, "late_checkout_surcharge_pct"),
            weekend_uplift_pct: read_f64(&row, "weekend_uplift_pct"),
        });
    }

    let fallback_row = sqlx::query("SELECT base_price FROM rooms WHERE LOWER(type) = ? LIMIT 1")
        .bind(&room_type_lower)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|error| BookingError::database(error.to_string()))?;

    let fallback_price = fallback_row
        .as_ref()
        .map(|row| read_f64(row, "base_price"))
        .unwrap_or(350_000.0);

    Ok(crate::pricing::PricingRule {
        room_type: room_type.to_string(),
        hourly_rate: fallback_price / 5.0,
        overnight_rate: fallback_price * 0.75,
        daily_rate: fallback_price,
        ..Default::default()
    })
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
    use super::{
        build_effective_pricing_rule, calculate_from_loaded_inputs, stored_rule_from_row,
        StayPricingInputs, StoredPricingRule,
    };
    use crate::domain::booking::BookingError;
    use sqlx::{Connection, SqliteConnection};

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
            overnight_rate: 500_000.0,
            daily_rate: 700_000.0,
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
        assert_eq!(rule.hourly_rate, 120_000.0);
        assert_eq!(rule.overnight_rate, 500_000.0);
        assert_eq!(rule.daily_rate, 700_000.0);
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
        inputs.fallback_base_price = Some(500_000.0);

        let rule = build_effective_pricing_rule(&inputs);

        assert_eq!(rule.room_type, "deluxe");
        assert_eq!(rule.hourly_rate, 100_000.0);
        assert_eq!(rule.overnight_rate, 375_000.0);
        assert_eq!(rule.daily_rate, 500_000.0);
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
        assert_eq!(rule.hourly_rate, 70_000.0);
        assert_eq!(rule.overnight_rate, 262_500.0);
        assert_eq!(rule.daily_rate, 350_000.0);
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
        inputs.fallback_base_price = Some(500_000.0);
        inputs.check_in = "2026-04-20T10:00:00+07:00".to_string();
        inputs.check_out = "2026-04-22T10:00:00+07:00".to_string();
        inputs.special_uplift_pct = 10.0;

        let pricing = calculate_from_loaded_inputs(&inputs).unwrap();

        assert_eq!(pricing.pricing_type, "nightly");
        assert_eq!(pricing.base_amount, 1_000_000.0);
        assert_eq!(pricing.weekend_amount, 0.0);
        assert_eq!(pricing.surcharge_amount, 100_000.0);
        assert_eq!(pricing.total, 1_100_000.0);
        assert_eq!(pricing.breakdown.len(), 2);
        assert_eq!(pricing.breakdown[0].amount, 1_000_000.0);
        assert_eq!(pricing.breakdown[1].amount, 100_000.0);
    }

    #[test]
    fn calculate_from_loaded_inputs_maps_invalid_datetime_errors() {
        let mut inputs = sample_inputs();
        inputs.fallback_base_price = Some(500_000.0);
        inputs.check_in = "not-a-datetime".to_string();

        let error = calculate_from_loaded_inputs(&inputs).unwrap_err();

        assert!(matches!(
            error,
            BookingError::DateTimeParse(message) if message.contains("Invalid check-in datetime")
        ));
    }

    #[tokio::test]
    async fn stored_rule_from_row_maps_all_columns() {
        let mut connection = SqliteConnection::connect(":memory:").await.unwrap();
        let row = sqlx::query(
            "SELECT
                'deluxe' AS room_type,
                120000.0 AS hourly_rate,
                500000.0 AS overnight_rate,
                700000.0 AS daily_rate,
                '21:00' AS overnight_start,
                '10:00' AS overnight_end,
                '13:00' AS daily_checkin,
                '11:00' AS daily_checkout,
                15.0 AS early_checkin_surcharge_pct,
                20.0 AS late_checkout_surcharge_pct,
                12.5 AS weekend_uplift_pct",
        )
        .fetch_one(&mut connection)
        .await
        .unwrap();

        let rule = stored_rule_from_row(&row);

        assert_eq!(rule.room_type, "deluxe");
        assert_eq!(rule.hourly_rate, 120_000.0);
        assert_eq!(rule.overnight_rate, 500_000.0);
        assert_eq!(rule.daily_rate, 700_000.0);
        assert_eq!(rule.overnight_start, "21:00");
        assert_eq!(rule.overnight_end, "10:00");
        assert_eq!(rule.daily_checkin, "13:00");
        assert_eq!(rule.daily_checkout, "11:00");
        assert_eq!(rule.early_checkin_surcharge_pct, 15.0);
        assert_eq!(rule.late_checkout_surcharge_pct, 20.0);
        assert_eq!(rule.weekend_uplift_pct, 12.5);
    }
}
