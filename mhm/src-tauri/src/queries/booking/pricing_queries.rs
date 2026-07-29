//! Reads that feed the stay-pricing rules.
//!
//! Split out of `domain::booking::pricing`, which is now pure.
//!
//! The `_tx` loaders take the caller's transaction: a lifecycle write needs to
//! read rows it has inserted but not committed, so a pool-based read would see
//! stale data. The pool loaders below serve the *preview* path, which has no
//! transaction and keys off a room type rather than a room id.

use sqlx::{Pool, Row, Sqlite, Transaction};

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

/// The base price a room type falls back to when it has no `pricing_rules` row.
///
/// `ORDER BY id` is load-bearing, if only for reproducibility. Without it SQLite
/// returns whichever row it likes, so two rooms of one type with different
/// `base_price` could quote one figure and charge another across app restarts.
///
/// It does not make the answer *right*, and this is the honest limit of the
/// type-keyed design: with mixed prices inside a type, a type-level rule cannot
/// represent both rooms, so an 800k room is billed at its 600k sibling's rate.
/// Pricing the booked room instead would need the preview to know which room —
/// which the group sheet, quoting per type, does not. That is a product
/// decision, not a bug fix. Both loaders share this constant so the quote and
/// the charge cannot disagree about which room they picked.
const FALLBACK_BASE_PRICE_SQL: &str =
    "SELECT base_price FROM rooms WHERE LOWER(type) = ? ORDER BY id LIMIT 1";

const SPECIAL_UPLIFT_SQL: &str =
    "SELECT CAST(uplift_pct AS REAL) FROM special_dates WHERE date = ?";

const PRICING_RULE_LISTING_SQL: &str =
    "SELECT id, room_type, hourly_rate, overnight_rate, daily_rate,
                overnight_start, overnight_end, daily_checkin, daily_checkout,
                early_checkin_surcharge_pct, late_checkout_surcharge_pct,
                weekend_uplift_pct
         FROM pricing_rules ORDER BY room_type";

const SPECIAL_DATES_SQL: &str =
    "SELECT id, date, label, uplift_pct FROM special_dates ORDER BY date";

/// A stored rule plus its row id, which the settings screen needs and the
/// pricing rules themselves do not.
pub(crate) struct PricingRuleListing {
    pub(crate) id: String,
    pub(crate) rule: StoredPricingRule,
}

pub struct SpecialDate {
    pub id: String,
    pub date: String,
    pub label: String,
    pub uplift_pct: f64,
}

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

/// The preview counterpart of `load_stay_pricing_inputs_tx`. Callers of the
/// preview supply a room *type* directly — there is no booking or room yet — so
/// this skips the room lookup and is otherwise identical.
///
/// Every read here propagates, exactly as the transactional twin does. The
/// special-date read used to be lenient — a failure priced the stay at a 0%
/// uplift instead of failing the preview — so on a holiday the quote could come
/// out below what the lifecycle path would charge, silently.
///
/// How often that read can actually fail is not the point, and the honest answer
/// is *rarely*: the pool opens WAL with a 5s `busy_timeout` (`db.rs`), so a plain
/// `SELECT` does not block behind a writer. The point is that guessing 0% is the
/// wrong response to not knowing. A preview that cannot read the prices should
/// say so rather than quote low.
///
/// Note this path has no equivalent of the twin's room lookup: an unknown room
/// type previews at the house default where a check-in would fail. See
/// `the_house_default_only_applies_to_types_with_no_rooms`.
pub(crate) async fn load_stay_pricing_inputs_for_room_type(
    pool: &Pool<Sqlite>,
    room_type: &str,
    check_in: &str,
    check_out: &str,
    pricing_type: &str,
) -> BookingResult<StayPricingInputs> {
    let stored_rule = load_stored_pricing_rule(pool, room_type).await?;
    let fallback_base_price = if stored_rule.is_none() {
        load_fallback_base_price(pool, room_type).await?
    } else {
        None
    };
    let special_uplift_pct = load_special_uplift(pool, check_in).await?;

    Ok(StayPricingInputs {
        room_type: room_type.to_string(),
        stored_rule,
        fallback_base_price,
        special_uplift_pct,
        check_in: check_in.to_string(),
        check_out: check_out.to_string(),
        pricing_type: pricing_type.to_string(),
    })
}

async fn load_stored_pricing_rule(
    pool: &Pool<Sqlite>,
    room_type: &str,
) -> BookingResult<Option<StoredPricingRule>> {
    let row = sqlx::query(STORED_PRICING_RULE_SQL)
        .bind(room_type.to_lowercase())
        .fetch_optional(pool)
        .await
        .map_err(database_error)?;

    Ok(row.as_ref().map(stored_rule_from_row))
}

async fn load_fallback_base_price(
    pool: &Pool<Sqlite>,
    room_type: &str,
) -> BookingResult<Option<MoneyVnd>> {
    let row = sqlx::query(FALLBACK_BASE_PRICE_SQL)
        .bind(room_type.to_lowercase())
        .fetch_optional(pool)
        .await
        .map_err(database_error)?;

    Ok(row.as_ref().map(|row| get_money_vnd(row, "base_price")))
}

async fn load_special_uplift(pool: &Pool<Sqlite>, date_str: &str) -> BookingResult<f64> {
    let row: Option<(f64,)> = sqlx::query_as(SPECIAL_UPLIFT_SQL)
        .bind(date_key(date_str))
        .fetch_optional(pool)
        .await
        .map_err(database_error)?;

    Ok(row.map(|value| value.0).unwrap_or(0.0))
}

pub(crate) async fn load_pricing_rule_listings(
    pool: &Pool<Sqlite>,
) -> Result<Vec<PricingRuleListing>, sqlx::Error> {
    let rows = sqlx::query(PRICING_RULE_LISTING_SQL)
        .fetch_all(pool)
        .await?;

    Ok(rows
        .iter()
        .map(|row| PricingRuleListing {
            id: row.get("id"),
            rule: stored_rule_from_row(row),
        })
        .collect())
}

pub async fn load_special_dates(pool: &Pool<Sqlite>) -> Result<Vec<SpecialDate>, sqlx::Error> {
    let rows = sqlx::query(SPECIAL_DATES_SQL).fetch_all(pool).await?;

    Ok(rows
        .iter()
        .map(|row| SpecialDate {
            id: row.get("id"),
            date: row.get("date"),
            label: row.get("label"),
            uplift_pct: get_f64(row, "uplift_pct"),
        })
        .collect())
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
    use super::{
        load_stay_pricing_inputs_for_room_type, load_stay_pricing_inputs_tx, stored_rule_from_row,
        BookingError,
    };
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

    const HOLIDAY_CHECK_IN: &str = "2026-04-20T14:00:00+07:00";
    const HOLIDAY_CHECK_OUT: &str = "2026-04-21T12:00:00+07:00";

    async fn pool_with_a_holiday() -> sqlx::SqlitePool {
        let pool = setup_loader_pool().await;
        sqlx::query("INSERT INTO rooms (id, type, base_price) VALUES (?, ?, ?)")
            .bind("room-3")
            .bind("deluxe")
            .bind(600000)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO special_dates (date, uplift_pct) VALUES (?, ?)")
            .bind("2026-04-20")
            .bind(10.0)
            .execute(&pool)
            .await
            .unwrap();
        pool
    }

    async fn preview_uplift(pool: &sqlx::SqlitePool) -> super::BookingResult<f64> {
        load_stay_pricing_inputs_for_room_type(
            pool,
            "deluxe",
            HOLIDAY_CHECK_IN,
            HOLIDAY_CHECK_OUT,
            "nightly",
        )
        .await
        .map(|inputs| inputs.special_uplift_pct)
    }

    /// The preview used to end its special-date read with `.unwrap_or(0.0)`, so a
    /// failed read quoted the stay with no holiday surcharge while the lifecycle
    /// path still charged one. Dropping the table is the cheapest way to make that
    /// read fail; the realistic cause is a transient lock, not a missing table.
    ///
    /// Paired with the happy path on purpose: a preview that errors on *everything*
    /// would also pass the first assertion alone.
    ///
    /// Asserts the error *variant*, not its text. The message here is SQLite's
    /// `no such table`, which only this artificial `DROP` produces; a real lock
    /// says `database is locked` and names no table. Matching on the string would
    /// pin an accident of the test setup.
    #[tokio::test]
    async fn a_failed_special_date_read_fails_the_preview_instead_of_quoting_no_uplift() {
        let pool = pool_with_a_holiday().await;
        assert_eq!(
            preview_uplift(&pool).await.unwrap(),
            10.0,
            "precondition: the preview reads the uplift when the read works"
        );

        pool.execute("DROP TABLE special_dates").await.unwrap();

        let error = preview_uplift(&pool)
            .await
            .expect_err("an unreadable special_dates must not be quoted as 0% uplift");
        assert!(
            matches!(error, BookingError::Database(_)),
            "a failed read is a database error, not a price: {error:?}"
        );
    }
}
