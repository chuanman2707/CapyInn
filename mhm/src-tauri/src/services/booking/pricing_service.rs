//! Sequences stay pricing: load the inputs, then apply the rules.
//!
//! The split is deliberate — `queries::booking::pricing_queries` owns the
//! reads, `domain::booking::pricing` owns the rules, and this module is the
//! only place that knows both exist.
//!
//! Both the lifecycle charge and the UI preview come through here, so they
//! cannot drift apart. `commands::pricing` used to carry its own copy of the
//! rule-building — stored rule wins, else derive from the room's base price,
//! else a 350k house default — which meant a preview could in principle
//! disagree with what the guest was actually charged.

use sqlx::{Pool, Sqlite, Transaction};

use crate::app_error::CommandResult;
use crate::domain::booking::pricing::calculate_from_loaded_inputs;
use crate::domain::booking::BookingResult;
use crate::money::{validate_non_negative_money_vnd, MoneyVnd};
use crate::queries::booking::pricing_queries::{
    load_stay_pricing_inputs_for_room, load_stay_pricing_inputs_for_room_type,
    load_stay_pricing_inputs_tx,
};
use crate::repositories::booking::pricing_repository::{self, PricingRuleUpsert};

/// Prices inside the caller's transaction so a lifecycle write can read the
/// rows it just inserted but has not committed.
pub async fn calculate_stay_price_tx(
    tx: &mut Transaction<'_, Sqlite>,
    room_id: &str,
    check_in: &str,
    check_out: &str,
    pricing_type: &str,
    guests: Option<i32>,
) -> BookingResult<crate::pricing::PricingResult> {
    let inputs =
        load_stay_pricing_inputs_tx(tx, room_id, check_in, check_out, pricing_type, guests).await?;
    calculate_from_loaded_inputs(&inputs)
}

/// The quote the UI shows before anything is booked. Keyed by room type, since
/// no room has been picked yet.
pub async fn calculate_price_preview(
    pool: &Pool<Sqlite>,
    room_type: &str,
    check_in: &str,
    check_out: &str,
    pricing_type: &str,
    guests: Option<i32>,
) -> BookingResult<crate::pricing::PricingResult> {
    let inputs = load_stay_pricing_inputs_for_room_type(
        pool,
        room_type,
        check_in,
        check_out,
        pricing_type,
        guests,
    )
    .await?;
    calculate_from_loaded_inputs(&inputs)
}

/// Bản xem trước mà form đặt phòng dùng: keyed theo phòng đã chọn, nên con số
/// nó trả về đúng bằng con số `calculate_stay_price_tx` sẽ tính khi ghi sổ.
pub async fn calculate_room_price_preview(
    pool: &Pool<Sqlite>,
    room_id: &str,
    check_in: &str,
    check_out: &str,
    pricing_type: &str,
    guests: Option<i32>,
) -> BookingResult<crate::pricing::PricingResult> {
    let inputs = load_stay_pricing_inputs_for_room(
        pool,
        room_id,
        check_in,
        check_out,
        pricing_type,
        guests,
    )
    .await?;
    calculate_from_loaded_inputs(&inputs)
}

/// The values a caller may leave unset, and what the house uses instead.
const DEFAULT_OVERNIGHT_START: &str = "22:00";
const DEFAULT_OVERNIGHT_END: &str = "11:00";
const DEFAULT_DAILY_CHECKIN: &str = "14:00";
const DEFAULT_DAILY_CHECKOUT: &str = "12:00";
const DEFAULT_SURCHARGE_PCT: f64 = 30.0;
const DEFAULT_WEEKEND_UPLIFT_PCT: f64 = 0.0;

/// What `save_pricing_rule` was handed, before defaults and validation.
pub struct SavePricingRule {
    pub room_type: String,
    pub hourly_rate: MoneyVnd,
    pub overnight_rate: MoneyVnd,
    pub daily_rate: MoneyVnd,
    pub overnight_start: Option<String>,
    pub overnight_end: Option<String>,
    pub daily_checkin: Option<String>,
    pub daily_checkout: Option<String>,
    pub early_pct: Option<f64>,
    pub late_pct: Option<f64>,
    pub weekend_pct: Option<f64>,
}

pub async fn save_pricing_rule(
    pool: &Pool<Sqlite>,
    request: SavePricingRule,
    id: String,
    now: String,
) -> CommandResult<()> {
    let upsert = PricingRuleUpsert {
        id,
        room_type: request.room_type,
        hourly_rate: validate_non_negative_money_vnd(request.hourly_rate, "hourly_rate")?,
        overnight_rate: validate_non_negative_money_vnd(request.overnight_rate, "overnight_rate")?,
        daily_rate: validate_non_negative_money_vnd(request.daily_rate, "daily_rate")?,
        overnight_start: request
            .overnight_start
            .unwrap_or_else(|| DEFAULT_OVERNIGHT_START.to_string()),
        overnight_end: request
            .overnight_end
            .unwrap_or_else(|| DEFAULT_OVERNIGHT_END.to_string()),
        daily_checkin: request
            .daily_checkin
            .unwrap_or_else(|| DEFAULT_DAILY_CHECKIN.to_string()),
        daily_checkout: request
            .daily_checkout
            .unwrap_or_else(|| DEFAULT_DAILY_CHECKOUT.to_string()),
        early_checkin_surcharge_pct: request.early_pct.unwrap_or(DEFAULT_SURCHARGE_PCT),
        late_checkout_surcharge_pct: request.late_pct.unwrap_or(DEFAULT_SURCHARGE_PCT),
        weekend_uplift_pct: request.weekend_pct.unwrap_or(DEFAULT_WEEKEND_UPLIFT_PCT),
        now,
    };

    pricing_repository::upsert_pricing_rule(pool, &upsert)
        .await
        .map_err(|error| {
            crate::app_error::log_system_error(
                "save_pricing_rule",
                error.to_string(),
                serde_json::json!({ "step": "upsert_pricing_rule", "room_type": &upsert.room_type }),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::{
        calculate_price_preview, calculate_stay_price_tx, save_pricing_rule, SavePricingRule,
    };
    use crate::app_error::codes;
    use sqlx::{sqlite::SqlitePoolOptions, Pool, Row, Sqlite};

    const CHECK_IN: &str = "2026-04-20T14:00:00+07:00";
    const CHECK_OUT: &str = "2026-04-22T12:00:00+07:00";

    fn request(hourly: i64, overnight: i64, daily: i64) -> SavePricingRule {
        SavePricingRule {
            room_type: "standard".to_string(),
            hourly_rate: hourly,
            overnight_rate: overnight,
            daily_rate: daily,
            overnight_start: None,
            overnight_end: None,
            daily_checkin: None,
            daily_checkout: None,
            early_pct: None,
            late_pct: None,
            weekend_pct: None,
        }
    }

    async fn migrated_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("connect in-memory sqlite");
        crate::db::run_migrations(&pool)
            .await
            .expect("run migrations");
        pool
    }

    async fn seed_room(pool: &Pool<Sqlite>, room_id: &str, room_type: &str, base_price: i64) {
        sqlx::query(
            "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status)
             VALUES (?, ?, ?, 1, 0, ?, 2, 0, 'vacant')",
        )
        .bind(room_id)
        .bind(format!("Room {room_id}"))
        .bind(room_type)
        .bind(base_price)
        .execute(pool)
        .await
        .expect("seed room");
    }

    /// The point of routing both paths through this module: the quote the UI
    /// shows and the amount charged at check-in must agree.
    ///
    /// Covers the two rule sources both paths can reach — a configured rule, and
    /// a rule derived from the room's base price. The 350k house default cannot
    /// be compared: it only applies when no room of the type exists, and the
    /// lifecycle path starts from a room id, so that room's own type always
    /// supplies a base price. `the_house_default_only_applies_to_types_with_no_rooms`
    /// covers that branch on the preview side.
    #[tokio::test]
    async fn the_preview_and_the_lifecycle_charge_agree_on_every_reachable_rule_source() {
        let pool = migrated_pool().await;

        // 1. configured rule
        seed_room(&pool, "R-CONF", "configured", 400_000).await;
        save_pricing_rule(
            &pool,
            SavePricingRule {
                room_type: "configured".to_string(),
                hourly_rate: 90_000,
                overnight_rate: 320_000,
                daily_rate: 480_000,
                overnight_start: None,
                overnight_end: None,
                daily_checkin: None,
                daily_checkout: None,
                early_pct: Some(10.0),
                late_pct: Some(15.0),
                weekend_pct: Some(5.0),
            },
            "rule-configured".to_string(),
            "2026-04-22T00:00:00+07:00".to_string(),
        )
        .await
        .expect("save rule");

        // 2. no rule, but the room carries a base price
        seed_room(&pool, "R-BASE", "derived", 620_000).await;

        for (room_id, room_type) in [("R-CONF", "configured"), ("R-BASE", "derived")] {
            let preview =
                calculate_price_preview(&pool, room_type, CHECK_IN, CHECK_OUT, "nightly", None)
                    .await
                    .unwrap_or_else(|error| panic!("preview for {room_type}: {error}"));

            let mut tx = pool.begin().await.expect("begin");
            let charged =
                calculate_stay_price_tx(&mut tx, room_id, CHECK_IN, CHECK_OUT, "nightly", None)
                    .await
                    .unwrap_or_else(|error| panic!("charge for {room_type}: {error}"));
            tx.rollback().await.expect("rollback");

            assert_eq!(preview.total, charged.total, "total for {room_type}");
            assert_eq!(
                preview.base_amount, charged.base_amount,
                "base for {room_type}"
            );
            assert_eq!(
                preview.weekend_amount, charged.weekend_amount,
                "weekend for {room_type}"
            );
            assert_eq!(
                preview.surcharge_amount, charged.surcharge_amount,
                "surcharge for {room_type}"
            );
            assert_eq!(
                preview.breakdown.len(),
                charged.breakdown.len(),
                "breakdown for {room_type}"
            );
        }
    }

    /// The 350k house default is preview-only in practice: it needs a room type
    /// with no rooms behind it, which a check-in can never be.
    #[tokio::test]
    async fn the_house_default_only_applies_to_types_with_no_rooms() {
        let pool = migrated_pool().await;
        seed_room(&pool, "R-BASE", "derived", 350_000).await;

        let house =
            calculate_price_preview(&pool, "no-such-type", CHECK_IN, CHECK_OUT, "nightly", None)
                .await
                .expect("house preview");
        let derived =
            calculate_price_preview(&pool, "derived", CHECK_IN, CHECK_OUT, "nightly", None)
                .await
                .expect("derived preview");

        // The house default *is* 350k, so a room priced at 350k must quote the same.
        assert_eq!(house.total, derived.total);
        assert!(house.total > 0);
    }

    /// A configured rule must actually change the quote, otherwise the test above
    /// could pass on two identically-wrong numbers.
    #[tokio::test]
    async fn the_three_rule_sources_produce_different_quotes() {
        let pool = migrated_pool().await;
        seed_room(&pool, "R-CONF", "configured", 400_000).await;
        save_pricing_rule(
            &pool,
            SavePricingRule {
                room_type: "configured".to_string(),
                hourly_rate: 90_000,
                overnight_rate: 320_000,
                daily_rate: 480_000,
                overnight_start: None,
                overnight_end: None,
                daily_checkin: None,
                daily_checkout: None,
                early_pct: None,
                late_pct: None,
                weekend_pct: None,
            },
            "rule-configured".to_string(),
            "2026-04-22T00:00:00+07:00".to_string(),
        )
        .await
        .expect("save rule");
        seed_room(&pool, "R-BASE", "derived", 620_000).await;

        let configured =
            calculate_price_preview(&pool, "configured", CHECK_IN, CHECK_OUT, "nightly", None)
                .await
                .expect("configured preview");
        let derived =
            calculate_price_preview(&pool, "derived", CHECK_IN, CHECK_OUT, "nightly", None)
                .await
                .expect("derived preview");
        let house =
            calculate_price_preview(&pool, "no-such-type", CHECK_IN, CHECK_OUT, "nightly", None)
                .await
                .expect("house preview");

        assert_ne!(configured.total, derived.total);
        assert_ne!(derived.total, house.total);
        assert_ne!(configured.total, house.total);
    }

    #[tokio::test]
    async fn a_special_date_uplifts_the_preview() {
        let pool = migrated_pool().await;
        seed_room(&pool, "R-SPECIAL", "derived", 500_000).await;

        let plain = calculate_price_preview(&pool, "derived", CHECK_IN, CHECK_OUT, "nightly", None)
            .await
            .expect("plain preview");

        sqlx::query("INSERT INTO special_dates (id, date, label, uplift_pct, created_at) VALUES (?, ?, ?, ?, ?)")
            .bind("sd-1")
            .bind("2026-04-20")
            .bind("Lễ")
            .bind(10.0)
            .bind("2026-04-01T00:00:00+07:00")
            .execute(&pool)
            .await
            .expect("seed special date");

        let uplifted =
            calculate_price_preview(&pool, "derived", CHECK_IN, CHECK_OUT, "nightly", None)
                .await
                .expect("uplifted preview");

        assert!(uplifted.surcharge_amount > plain.surcharge_amount);
        assert!(uplifted.total > plain.total);
    }

    #[tokio::test]
    async fn save_pricing_rule_rejects_negative_money_rates() {
        let pool = migrated_pool().await;

        for (hourly, overnight, daily, field) in [
            (-1, 300_000, 400_000, "hourly_rate"),
            (80_000, -1, 400_000, "overnight_rate"),
            (80_000, 300_000, -1, "daily_rate"),
        ] {
            let error = save_pricing_rule(
                &pool,
                request(hourly, overnight, daily),
                "rule-1".to_string(),
                "2026-04-22T00:00:00+07:00".to_string(),
            )
            .await
            .expect_err("negative pricing money must fail");

            assert_eq!(error.code, codes::VALIDATION_INVALID_INPUT);
            assert!(error.message.contains(field));
        }

        let saved: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pricing_rules")
            .fetch_one(&pool)
            .await
            .expect("count rules");
        assert_eq!(saved, 0, "a rejected rule must not be written");
    }

    #[tokio::test]
    async fn save_pricing_rule_fills_in_the_house_defaults() {
        let pool = migrated_pool().await;

        save_pricing_rule(
            &pool,
            request(80_000, 300_000, 400_000),
            "rule-defaults".to_string(),
            "2026-04-22T00:00:00+07:00".to_string(),
        )
        .await
        .expect("save rule");

        let row = sqlx::query(
            "SELECT overnight_start, overnight_end, daily_checkin, daily_checkout,
                    early_checkin_surcharge_pct, late_checkout_surcharge_pct, weekend_uplift_pct
             FROM pricing_rules WHERE room_type = ?",
        )
        .bind("standard")
        .fetch_one(&pool)
        .await
        .expect("saved rule");

        assert_eq!(row.get::<String, _>("overnight_start"), "22:00");
        assert_eq!(row.get::<String, _>("overnight_end"), "11:00");
        assert_eq!(row.get::<String, _>("daily_checkin"), "14:00");
        assert_eq!(row.get::<String, _>("daily_checkout"), "12:00");
        assert_eq!(row.get::<f64, _>("early_checkin_surcharge_pct"), 30.0);
        assert_eq!(row.get::<f64, _>("late_checkout_surcharge_pct"), 30.0);
        assert_eq!(row.get::<f64, _>("weekend_uplift_pct"), 0.0);
    }

    #[tokio::test]
    async fn save_pricing_rule_replaces_the_rule_for_a_room_type() {
        let pool = migrated_pool().await;

        for (id, daily) in [("rule-a", 400_000), ("rule-b", 550_000)] {
            save_pricing_rule(
                &pool,
                request(80_000, 300_000, daily),
                id.to_string(),
                "2026-04-22T00:00:00+07:00".to_string(),
            )
            .await
            .expect("save rule");
        }

        let rows: Vec<(String, i64)> =
            sqlx::query_as("SELECT id, daily_rate FROM pricing_rules WHERE room_type = ?")
                .bind("standard")
                .fetch_all(&pool)
                .await
                .expect("saved rules");

        assert_eq!(rows.len(), 1, "one rule per room type");
        assert_eq!(rows[0].1, 550_000, "the later save wins");
        assert_eq!(rows[0].0, "rule-a", "the original row id is kept");
    }
}
