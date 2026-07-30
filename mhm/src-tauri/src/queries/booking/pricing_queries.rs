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

/// Room types are matched case-insensitively, and **both sides of the
/// comparison must be folded by the same function.**
///
/// Every type lookup here reads `LOWER(x) = LOWER(?)`. It used to bind
/// `str::to_lowercase()` against SQL's `LOWER(x)`, which is two different
/// functions: SQLite's `LOWER` is ASCII-only and leaves `Đ` alone, Rust's lowers
/// it to `đ`. So for any type whose name carries a non-ASCII capital — `Phòng
/// Đôi`, `Phòng Đơn`, most of a Vietnamese hotel's room list — the two strings
/// could never be equal. Not "matched loosely": *never matched*. The configured
/// rule was not found, no room supplied a fallback price, and the stay was quoted
/// and charged at the 350k house default while settings showed something else.
///
/// Folding inside SQL keeps the two sides one function by construction. It is
/// still ASCII-only, so `PHÒNG ĐÔI` will not match `phòng đôi` — but a type now
/// always matches itself, which is the case that actually occurs, since callers
/// pass a name read straight back out of `rooms.type`.
const STORED_PRICING_RULE_SQL: &str = "SELECT room_type, hourly_rate, overnight_rate, daily_rate,
                overnight_start, overnight_end, daily_checkin, daily_checkout,
                early_checkin_surcharge_pct, late_checkout_surcharge_pct,
                weekend_uplift_pct
         FROM pricing_rules WHERE LOWER(room_type) = LOWER(?)";

const ROOM_TYPE_SQL: &str = "SELECT type FROM rooms WHERE id = ? LIMIT 1";

/// The base price a room type falls back to when it has no `pricing_rules` row.
///
/// **Price is a property of the room type, not of the room.** That is the
/// operator's decision, and everything here follows from it: two rooms of one
/// type cost the same night, whichever key the guest is handed.
///
/// So `rooms.base_price` is not a price the system charges. It is per-room data
/// predating the rule table, read here only to derive a *type* price when the
/// type has no rule configured. `ORDER BY id` makes that derivation
/// reproducible: without it SQLite returns whichever row it likes, so a type
/// whose rooms disagree about `base_price` could quote one figure and charge
/// another across app restarts.
///
/// When rooms of one type do disagree, the lowest id wins and the rest of the
/// type bills at its rate. Under a type-keyed price that is not an error to fix
/// in the SQL — it means the operator holds per-room prices the model cannot
/// express, and the answer is to give the type a `pricing_rules` row.
///
/// Both loaders share this constant, so the quote and the charge cannot
/// disagree about which room they derived from.
const FALLBACK_BASE_PRICE_SQL: &str =
    "SELECT base_price FROM rooms WHERE LOWER(type) = LOWER(?) ORDER BY id LIMIT 1";

const ROOM_GUEST_PRICING_SQL: &str = "SELECT COALESCE(max_guests, 2) AS max_guests,
            COALESCE(extra_person_fee, 0) AS extra_person_fee
     FROM rooms WHERE id = ?";

/// Guest limits for a room type, derived the same way the type price is.
///
/// `ORDER BY id` for the reason `FALLBACK_BASE_PRICE_SQL` has it: no room is
/// chosen yet, so a room of the type has to stand in for it, and *which* room
/// must not depend on how SQLite feels about page order. Without it a type whose
/// rooms disagree about `max_guests` quotes one extra-person surcharge today and
/// a different one after a restart — and it is the lowest id that supplies the
/// base price, so anything else here would take the two halves of one quote from
/// two different rooms.
const ROOM_GUEST_PRICING_BY_TYPE_SQL: &str = "SELECT COALESCE(max_guests, 2) AS max_guests,
            COALESCE(extra_person_fee, 0) AS extra_person_fee
     FROM rooms WHERE LOWER(type) = LOWER(?) ORDER BY id LIMIT 1";

const SPECIAL_UPLIFT_SQL: &str =
    "SELECT CAST(uplift_pct AS REAL) FROM special_dates WHERE date = ?";

/// Every room type the house actually has rooms in. A type with a
/// `pricing_rules` row but no rooms prices nothing, so it is not listed.
const ROOM_TYPE_NAMES_SQL: &str = "SELECT DISTINCT type FROM rooms ORDER BY type";

const PRICING_RULE_LISTING_SQL: &str =
    "SELECT id, room_type, hourly_rate, overnight_rate, daily_rate,
                overnight_start, overnight_end, daily_checkin, daily_checkout,
                early_checkin_surcharge_pct, late_checkout_surcharge_pct,
                weekend_uplift_pct
         FROM pricing_rules ORDER BY room_type";

const SPECIAL_DATES_SQL: &str =
    "SELECT id, date, label, uplift_pct FROM special_dates ORDER BY date";

/// What it takes to resolve a room type's rule, and nothing about a stay.
///
/// The two loaders below and the room-type rate listing all need exactly this
/// pair, and all three must load it the same way: the fallback is only read when
/// no rule is stored, so a configured type never touches `rooms.base_price`.
pub(crate) struct TypeRuleInputs {
    pub(crate) stored_rule: Option<StoredPricingRule>,
    pub(crate) fallback_base_price: Option<MoneyVnd>,
}

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

/// Mốc tính giá của một phòng. Phòng không tìm thấy thì dùng mặc định của schema
/// (`max_guests` mặc định 2, `extra_person_fee` mặc định 0) — tức không phụ thu.
struct RoomGuestPricing {
    base_guests: i32,
    extra_person_fee: MoneyVnd,
}

impl Default for RoomGuestPricing {
    fn default() -> Self {
        Self {
            base_guests: 2,
            extra_person_fee: 0,
        }
    }
}

fn room_guest_pricing_from_row(row: &sqlx::sqlite::SqliteRow) -> RoomGuestPricing {
    RoomGuestPricing {
        base_guests: row.get("max_guests"),
        extra_person_fee: get_money_vnd(row, "extra_person_fee"),
    }
}

async fn load_room_guest_pricing_tx(
    tx: &mut Transaction<'_, Sqlite>,
    room_id: &str,
) -> BookingResult<RoomGuestPricing> {
    let row = sqlx::query(ROOM_GUEST_PRICING_SQL)
        .bind(room_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_error)?;

    Ok(row
        .as_ref()
        .map(room_guest_pricing_from_row)
        .unwrap_or_default())
}

async fn load_room_guest_pricing_for_type(
    pool: &Pool<Sqlite>,
    room_type: &str,
) -> BookingResult<RoomGuestPricing> {
    let row = sqlx::query(ROOM_GUEST_PRICING_BY_TYPE_SQL)
        .bind(room_type)
        .fetch_optional(pool)
        .await
        .map_err(database_error)?;

    Ok(row
        .as_ref()
        .map(room_guest_pricing_from_row)
        .unwrap_or_default())
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
    guests: Option<i32>,
) -> BookingResult<StayPricingInputs> {
    let room_type = load_room_type_tx(tx, room_id).await?;
    let stored_rule = load_stored_pricing_rule_tx(tx, &room_type).await?;
    let fallback_base_price = if stored_rule.is_none() {
        load_fallback_base_price_tx(tx, &room_type).await?
    } else {
        None
    };
    let special_uplift_pct = load_special_uplift_tx(tx, check_in).await?;
    let guest_pricing = load_room_guest_pricing_tx(tx, room_id).await?;

    Ok(StayPricingInputs {
        room_type,
        stored_rule,
        fallback_base_price,
        special_uplift_pct,
        check_in: check_in.to_string(),
        check_out: check_out.to_string(),
        pricing_type: pricing_type.to_string(),
        guests,
        base_guests: guest_pricing.base_guests,
        extra_person_fee: guest_pricing.extra_person_fee,
    })
}

/// The preview counterpart of `load_stay_pricing_inputs_tx`. Callers of the
/// preview supply a room *type* directly — there is no booking or room yet — so
/// this skips the room lookup and is otherwise identical.
///
/// Every read here propagates, exactly as the transactional twin does. The
/// special-date read used to be lenient — a failure priced the stay at a 0%
/// uplift instead of failing the preview — so on a holiday the quote came out
/// *below* what the lifecycle path would charge, silently and in the guest's
/// favour until the desk collected the difference.
///
/// How often that read can fail is not the point, and the honest answer is
/// rarely: the pool opens WAL with a 5s `busy_timeout` (`db.rs`), so a plain
/// `SELECT` does not queue behind a writer. The point is that guessing 0% is the
/// wrong response to not knowing. A preview that cannot read the prices should
/// say so rather than quote low.
pub(crate) async fn load_stay_pricing_inputs_for_room_type(
    pool: &Pool<Sqlite>,
    room_type: &str,
    check_in: &str,
    check_out: &str,
    pricing_type: &str,
    guests: Option<i32>,
) -> BookingResult<StayPricingInputs> {
    let rule_inputs = load_type_rule_inputs(pool, room_type).await?;
    let special_uplift_pct = load_special_uplift(pool, check_in).await?;
    let guest_pricing = load_room_guest_pricing_for_type(pool, room_type).await?;

    Ok(StayPricingInputs {
        room_type: room_type.to_string(),
        stored_rule: rule_inputs.stored_rule,
        fallback_base_price: rule_inputs.fallback_base_price,
        special_uplift_pct,
        check_in: check_in.to_string(),
        check_out: check_out.to_string(),
        pricing_type: pricing_type.to_string(),
        guests,
        base_guests: guest_pricing.base_guests,
        extra_person_fee: guest_pricing.extra_person_fee,
    })
}

/// Bản xem trước theo **mã phòng**. Khác `..._for_room_type` ở chỗ phụ thu thêm
/// người là thuộc tính của từng phòng, nên phòng đã chọn rồi thì phải đọc đúng
/// phòng đó — hai phòng cùng loại có thể đặt phụ thu khác nhau.
///
/// Ngày đặc biệt đọc y như bản theo loại phòng: lỗi thì **báo lỗi**, không coi
/// như 0%. Chính vì đây là xem trước nên mới phải chặt: con số này được đọc cho
/// khách nghe trước khi thu tiền, nên đoán 0% đúng vào ngày lễ chỉ tạo ra một
/// báo giá thấp hơn số thực thu. Không đọc được giá thì phải nói là không biết.
pub(crate) async fn load_stay_pricing_inputs_for_room(
    pool: &Pool<Sqlite>,
    room_id: &str,
    check_in: &str,
    check_out: &str,
    pricing_type: &str,
    guests: Option<i32>,
) -> BookingResult<StayPricingInputs> {
    let room_type = load_room_type(pool, room_id).await?;
    let rule_inputs = load_type_rule_inputs(pool, &room_type).await?;
    let special_uplift_pct = load_special_uplift(pool, check_in).await?;
    let guest_pricing = load_room_guest_pricing(pool, room_id).await?;

    Ok(StayPricingInputs {
        room_type,
        stored_rule: rule_inputs.stored_rule,
        fallback_base_price: rule_inputs.fallback_base_price,
        special_uplift_pct,
        check_in: check_in.to_string(),
        check_out: check_out.to_string(),
        pricing_type: pricing_type.to_string(),
        guests,
        base_guests: guest_pricing.base_guests,
        extra_person_fee: guest_pricing.extra_person_fee,
    })
}

async fn load_room_type(pool: &Pool<Sqlite>, room_id: &str) -> BookingResult<String> {
    sqlx::query_scalar::<_, String>(ROOM_TYPE_SQL)
        .bind(room_id)
        .fetch_optional(pool)
        .await
        .map_err(database_error)?
        .ok_or_else(|| room_not_found(room_id))
}

async fn load_room_guest_pricing(
    pool: &Pool<Sqlite>,
    room_id: &str,
) -> BookingResult<RoomGuestPricing> {
    let row = sqlx::query(ROOM_GUEST_PRICING_SQL)
        .bind(room_id)
        .fetch_optional(pool)
        .await
        .map_err(database_error)?;

    Ok(row
        .as_ref()
        .map(room_guest_pricing_from_row)
        .unwrap_or_default())
}

pub(crate) async fn load_room_type_names(pool: &Pool<Sqlite>) -> BookingResult<Vec<String>> {
    sqlx::query_scalar::<_, String>(ROOM_TYPE_NAMES_SQL)
        .fetch_all(pool)
        .await
        .map_err(database_error)
}

pub(crate) async fn load_type_rule_inputs(
    pool: &Pool<Sqlite>,
    room_type: &str,
) -> BookingResult<TypeRuleInputs> {
    let stored_rule = load_stored_pricing_rule(pool, room_type).await?;
    let fallback_base_price = if stored_rule.is_none() {
        load_fallback_base_price(pool, room_type).await?
    } else {
        None
    };

    Ok(TypeRuleInputs {
        stored_rule,
        fallback_base_price,
    })
}

async fn load_stored_pricing_rule(
    pool: &Pool<Sqlite>,
    room_type: &str,
) -> BookingResult<Option<StoredPricingRule>> {
    let row = sqlx::query(STORED_PRICING_RULE_SQL)
        .bind(room_type)
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
        .bind(room_type)
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
        .bind(room_type)
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
        .bind(room_type)
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
            "CREATE TABLE rooms (
                id TEXT PRIMARY KEY,
                type TEXT NOT NULL,
                base_price INTEGER,
                max_guests INTEGER NOT NULL DEFAULT 2,
                extra_person_fee INTEGER NOT NULL DEFAULT 0
            )",
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
        pool.execute(
            "CREATE TABLE rooms (
                id TEXT PRIMARY KEY,
                type TEXT NOT NULL,
                max_guests INTEGER NOT NULL DEFAULT 2,
                extra_person_fee INTEGER NOT NULL DEFAULT 0
            )",
        )
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
            None,
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
            None,
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
