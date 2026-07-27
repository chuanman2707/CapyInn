//! Reads that feed the stay-pricing rules.
//!
//! Split out of `domain::booking::pricing`, which is now pure.
//!
//! Every loader takes the caller's transaction. Pricing is only ever computed
//! as part of a lifecycle write, which needs to read rows it has inserted but
//! not committed, so a pool-based read would see stale data. The pool variants
//! that used to sit beside these were reachable only from tests.

use sqlx::{Row, Sqlite, Transaction};

use crate::db::row::{get_f64, get_money_vnd};
use crate::domain::booking::pricing::{StayPricingInputs, StoredPricingRule};
use crate::domain::booking::{BookingError, BookingResult};
use crate::money::MoneyVnd;

const STORED_PRICING_RULE_SQL: &str = "SELECT room_type, hourly_rate, overnight_rate, daily_rate,
                overnight_start, overnight_end, daily_checkin, daily_checkout,
                early_checkin_surcharge_pct, late_checkout_surcharge_pct,
                weekend_uplift_pct
         FROM pricing_rules WHERE LOWER(room_type) = ?";

const ROOM_TYPE_SQL: &str = "SELECT type FROM rooms WHERE id = ? LIMIT 1";

const FALLBACK_BASE_PRICE_SQL: &str =
    "SELECT base_price FROM rooms WHERE LOWER(type) = ? LIMIT 1";

const SPECIAL_UPLIFT_SQL: &str =
    "SELECT CAST(uplift_pct AS REAL) FROM special_dates WHERE date = ?";

fn database_error(error: sqlx::Error) -> BookingError {
    BookingError::database(error.to_string())
}

fn room_not_found(room_id: &str) -> BookingError {
    BookingError::not_found(format!("Không tìm thấy phòng {}", room_id))
}

/// `special_dates.date` is a bare `YYYY-MM-DD`, so an RFC3339 check-in has to
/// be truncated before it will match.
fn date_key(date_str: &str) -> &str {
    if date_str.len() >= 10 {
        &date_str[..10]
    } else {
        date_str
    }
}

pub(crate) fn stored_rule_from_row(row: &sqlx::sqlite::SqliteRow) -> StoredPricingRule {
    StoredPricingRule {
        room_type: row.get("room_type"),
        hourly_rate: get_money_vnd(row, "hourly_rate"),
        overnight_rate: get_money_vnd(row, "overnight_rate"),
        daily_rate: get_money_vnd(row, "daily_rate"),
        overnight_start: row.get("overnight_start"),
        overnight_end: row.get("overnight_end"),
        daily_checkin: row.get("daily_checkin"),
        daily_checkout: row.get("daily_checkout"),
        early_checkin_surcharge_pct: get_f64(row, "early_checkin_surcharge_pct"),
        late_checkout_surcharge_pct: get_f64(row, "late_checkout_surcharge_pct"),
        weekend_uplift_pct: get_f64(row, "weekend_uplift_pct"),
    }
}

pub(crate) async fn load_stay_pricing_inputs_tx(
    tx: &mut Transaction<'_, Sqlite>,
    room_id: &str,
    check_in: &str,
    check_out: &str,
    pricing_type: &str,
) -> BookingResult<StayPricingInputs> {
    let room_type = load_room_type_tx(tx, room_id).await?;
    let stored_rule = load_stored_pricing_rule_tx(tx, &room_type).await?;
    let fallback_base_price = if stored_rule.is_none() {
        load_fallback_base_price_tx(tx, &room_type).await?
    } else {
        None
    };
    let special_uplift_pct = load_special_uplift_tx(tx, check_in).await?;

    Ok(StayPricingInputs {
        room_type,
        stored_rule,
        fallback_base_price,
        special_uplift_pct,
        check_in: check_in.to_string(),
        check_out: check_out.to_string(),
        pricing_type: pricing_type.to_string(),
    })
}

async fn load_room_type_tx(
    tx: &mut Transaction<'_, Sqlite>,
    room_id: &str,
) -> BookingResult<String> {
    sqlx::query_scalar::<_, String>(ROOM_TYPE_SQL)
        .bind(room_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_error)?
        .ok_or_else(|| room_not_found(room_id))
}

async fn load_stored_pricing_rule_tx(
    tx: &mut Transaction<'_, Sqlite>,
    room_type: &str,
) -> BookingResult<Option<StoredPricingRule>> {
    let row = sqlx::query(STORED_PRICING_RULE_SQL)
        .bind(room_type.to_lowercase())
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_error)?;

    Ok(row.as_ref().map(stored_rule_from_row))
}

async fn load_fallback_base_price_tx(
    tx: &mut Transaction<'_, Sqlite>,
    room_type: &str,
) -> BookingResult<Option<MoneyVnd>> {
    let fallback_row = sqlx::query(FALLBACK_BASE_PRICE_SQL)
        .bind(room_type.to_lowercase())
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_error)?;

    Ok(fallback_row
        .as_ref()
        .map(|row| get_money_vnd(row, "base_price")))
}

async fn load_special_uplift_tx(
    tx: &mut Transaction<'_, Sqlite>,
    date_str: &str,
) -> BookingResult<f64> {
    let row: Option<(f64,)> = sqlx::query_as(SPECIAL_UPLIFT_SQL)
        .bind(date_key(date_str))
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_error)?;

    Ok(row.map(|value| value.0).unwrap_or(0.0))
}

#[cfg(test)]
mod tests {
    use super::{load_stay_pricing_inputs_tx, stored_rule_from_row};
    use sqlx::{sqlite::SqlitePoolOptions, Connection, Executor, SqliteConnection};

    #[tokio::test]
    async fn stored_rule_from_row_maps_all_columns() {
        let mut connection = SqliteConnection::connect(":memory:").await.unwrap();
        let row = sqlx::query(
            "SELECT
                'deluxe' AS room_type,
                120000 AS hourly_rate,
                500000 AS overnight_rate,
                700000 AS daily_rate,
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

    async fn setup_loader_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect(":memory:")
            .await
            .unwrap();

        pool.execute(
            "CREATE TABLE rooms (id TEXT PRIMARY KEY, type TEXT NOT NULL, base_price INTEGER)",
        )
        .await
        .unwrap();
        pool.execute(
            "CREATE TABLE pricing_rules (
                room_type TEXT,
                hourly_rate INTEGER,
                overnight_rate INTEGER,
                daily_rate INTEGER,
                overnight_start TEXT,
                overnight_end TEXT,
                daily_checkin TEXT,
                daily_checkout TEXT,
                early_checkin_surcharge_pct REAL,
                late_checkout_surcharge_pct REAL,
                weekend_uplift_pct REAL
            )",
        )
        .await
        .unwrap();
        pool.execute("CREATE TABLE special_dates (date TEXT PRIMARY KEY, uplift_pct REAL)")
            .await
            .unwrap();

        pool
    }

    #[tokio::test]
    async fn load_stay_pricing_inputs_prefers_stored_rule_without_fallback_query() {
        let pool = setup_loader_pool().await;

        pool.execute("DROP TABLE rooms").await.unwrap();
        pool.execute("CREATE TABLE rooms (id TEXT PRIMARY KEY, type TEXT NOT NULL)")
            .await
            .unwrap();
        sqlx::query("INSERT INTO rooms (id, type) VALUES (?, ?)")
            .bind("room-1")
            .bind("deluxe")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO pricing_rules (
                room_type, hourly_rate, overnight_rate, daily_rate, overnight_start,
                overnight_end, daily_checkin, daily_checkout,
                early_checkin_surcharge_pct, late_checkout_surcharge_pct, weekend_uplift_pct
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("deluxe")
        .bind(120000)
        .bind(500000)
        .bind(700000)
        .bind("21:00")
        .bind("10:00")
        .bind("13:00")
        .bind("11:00")
        .bind(15.0)
        .bind(20.0)
        .bind(12.5)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO special_dates (date, uplift_pct) VALUES (?, ?)")
            .bind("2026-04-20")
            .bind(10.0)
            .execute(&pool)
            .await
            .unwrap();

        let mut tx = pool.begin().await.unwrap();
        let inputs = load_stay_pricing_inputs_tx(
            &mut tx,
            "room-1",
            "2026-04-20T14:00:00+07:00",
            "2026-04-21T12:00:00+07:00",
            "nightly",
        )
        .await
        .unwrap();

        assert_eq!(inputs.room_type, "deluxe");
        assert!(inputs.stored_rule.is_some());
        assert_eq!(inputs.fallback_base_price, None);
        assert_eq!(inputs.special_uplift_pct, 10.0);

        tx.rollback().await.unwrap();
    }

    #[tokio::test]
    async fn load_stay_pricing_inputs_loads_fallback_base_price_when_rule_missing() {
        let pool = setup_loader_pool().await;

        sqlx::query("INSERT INTO rooms (id, type, base_price) VALUES (?, ?, ?)")
            .bind("room-2")
            .bind("standard")
            .bind(480000)
            .execute(&pool)
            .await
            .unwrap();

        let mut tx = pool.begin().await.unwrap();
        let inputs = load_stay_pricing_inputs_tx(
            &mut tx,
            "room-2",
            "2026-04-21T14:00:00+07:00",
            "2026-04-22T12:00:00+07:00",
            "nightly",
        )
        .await
        .unwrap();

        assert_eq!(inputs.room_type, "standard");
        assert!(inputs.stored_rule.is_none());
        assert_eq!(inputs.fallback_base_price, Some(480000));
        assert_eq!(inputs.special_uplift_pct, 0.0);

        tx.rollback().await.unwrap();
    }
}
