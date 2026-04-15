use sqlx::{Pool, Row, Sqlite, Transaction};

use super::{BookingError, BookingResult};

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

    Ok(crate::pricing::calculate_price(
        &rule,
        check_in,
        check_out,
        pricing_type,
        special_uplift,
    ))
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

    Ok(crate::pricing::calculate_price(
        &rule,
        check_in,
        check_out,
        pricing_type,
        special_uplift,
    ))
}

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

async fn load_pricing_rule(pool: &Pool<Sqlite>, room_type: &str) -> BookingResult<crate::pricing::PricingRule> {
    let room_type_lower = room_type.to_lowercase();
    let row = sqlx::query(
        "SELECT room_type, hourly_rate, overnight_rate, daily_rate,
                overnight_start, overnight_end, daily_checkin, daily_checkout,
                early_checkin_surcharge_pct, late_checkout_surcharge_pct,
                weekend_uplift_pct
         FROM pricing_rules WHERE LOWER(room_type) = ?"
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

    let fallback_row = sqlx::query(
        "SELECT base_price FROM rooms WHERE LOWER(type) = ? LIMIT 1"
    )
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

async fn load_special_uplift(pool: &Pool<Sqlite>, date_str: &str) -> BookingResult<f64> {
    let date = if date_str.len() >= 10 { &date_str[..10] } else { date_str };
    let row: Option<(f64,)> = sqlx::query_as(
        "SELECT CAST(uplift_pct AS REAL) FROM special_dates WHERE date = ?"
    )
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
    let date = if date_str.len() >= 10 { &date_str[..10] } else { date_str };
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
