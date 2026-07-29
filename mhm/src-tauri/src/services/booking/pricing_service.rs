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

use chrono::NaiveDate;
use sqlx::{Pool, Sqlite, Transaction};

use crate::app_error::{codes, CommandError, CommandResult};
use crate::domain::booking::pricing::{build_effective_pricing_rule, calculate_from_loaded_inputs};
use crate::domain::booking::BookingResult;
use crate::money::{validate_non_negative_money_vnd, MoneyVnd};
use crate::queries::booking::pricing_queries::{
    load_room_type_names, load_stay_pricing_inputs_for_room,
    load_stay_pricing_inputs_for_room_type, load_stay_pricing_inputs_tx, load_type_rule_inputs,
};
use crate::repositories::booking::pricing_repository::{
    self, delete_special_dates_tx, upsert_special_date_tx, PricingRuleUpsert, SpecialDateUpsert,
};

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
    let inputs =
        load_stay_pricing_inputs_for_room(pool, room_id, check_in, check_out, pricing_type, guests)
            .await?;
    calculate_from_loaded_inputs(&inputs)
}

/// The nightly rate a room type lists, before any date-dependent uplift.
pub struct RoomTypeRate {
    pub room_type: String,
    pub nightly_rate: MoneyVnd,
    /// `false` means no `pricing_rules` row: the rate below was *derived* from a
    /// room's `base_price` (or the house default). The UI says so, because a
    /// derived rate is a number nobody chose and the fix is to configure one.
    pub configured: bool,
}

/// Every room type's listed nightly rate, for screens that show a rate without
/// having a stay to price — room cards, the room drawer, the detail panel.
///
/// Goes through `build_effective_pricing_rule`, the same function the charge
/// resolves its rule with, so the figure on a card is the figure a nightly stay
/// starts from. Those screens used to print `rooms.base_price`, which the engine
/// ignores outright once a type has a rule: a card could read 300k beside a desk
/// collecting 480k.
///
/// It is the *list* rate, not a quote — weekend uplift, `special_dates` and the
/// extra-person fee all depend on dates and guests this call does not have. A
/// number attached to actual dates has to come from a preview command.
pub async fn list_room_type_rates(pool: &Pool<Sqlite>) -> BookingResult<Vec<RoomTypeRate>> {
    let mut rates = Vec::new();

    for room_type in load_room_type_names(pool).await? {
        let inputs = load_type_rule_inputs(pool, &room_type).await?;
        let configured = inputs.stored_rule.is_some();
        let rule = build_effective_pricing_rule(
            &room_type,
            inputs.stored_rule.as_ref(),
            inputs.fallback_base_price,
        );

        rates.push(RoomTypeRate {
            room_type,
            nightly_rate: rule.daily_rate,
            configured,
        });
    }

    Ok(rates)
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

const MAX_SPECIAL_RANGE_DAYS: i64 = 366;
const MAX_SPECIAL_UPLIFT_PCT: f64 = 500.0;

pub struct SaveSpecialDateRange {
    /// Những ngày phải xoá **cùng** transaction với lần ghi này. Rỗng là khai
    /// mới; có giá trị là đang sửa một cụm cho ngắn lại. Đây là lý do lệnh xoá
    /// và lệnh ghi không tách rời: xoá xong mà ghi hỏng là mất hẳn mấy ngày đã
    /// khai, và không ai biết.
    pub remove: Vec<String>,
    pub from: String,
    pub to: String,
    pub label: String,
    pub uplift_pct: f64,
}

fn parse_date_only(value: &str, field: &str) -> CommandResult<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| {
        CommandError::user(
            codes::VALIDATION_INVALID_INPUT,
            format!("{field} phải có dạng YYYY-MM-DD"),
        )
    })
}

fn invalid_input(message: impl Into<String>) -> CommandError {
    CommandError::user(codes::VALIDATION_INVALID_INPUT, message)
}

/// Khai một khoảng ngày là mùa cao điểm, một dòng cho một ngày.
///
/// `id_base` cấp mã cho từng ngày mới theo dạng `{id_base}-{chỉ số}`. Lệnh
/// truyền vào một uuid nên trong thực tế không bao giờ đụng; test thì truyền
/// một gốc đã có trong bảng để ép lỗi khoá chính giữa transaction — đó là chỗ
/// hỏng-được-theo-ý duy nhất, vì `ON CONFLICT(date)` đã nuốt mất ràng buộc trên
/// `date`.
pub async fn save_special_date_range(
    pool: &Pool<Sqlite>,
    request: SaveSpecialDateRange,
    id_base: String,
    now: String,
) -> CommandResult<()> {
    let from = parse_date_only(&request.from, "Ngày bắt đầu")?;
    let to = parse_date_only(&request.to, "Ngày kết thúc")?;
    if to < from {
        return Err(invalid_input("Ngày kết thúc không được trước ngày bắt đầu"));
    }

    let day_count = (to - from).num_days() + 1;
    if day_count > MAX_SPECIAL_RANGE_DAYS {
        return Err(invalid_input(format!(
            "Khoảng ngày tối đa {MAX_SPECIAL_RANGE_DAYS} ngày"
        )));
    }

    if !(0.0..=MAX_SPECIAL_UPLIFT_PCT).contains(&request.uplift_pct) {
        return Err(invalid_input(format!(
            "Mức phụ thu phải trong khoảng 0–{MAX_SPECIAL_UPLIFT_PCT:.0}%"
        )));
    }

    let label = request.label.trim();
    if label.is_empty() {
        return Err(invalid_input("Tên đợt cao điểm không được để trống"));
    }

    if request.remove.len() as i64 > MAX_SPECIAL_RANGE_DAYS {
        return Err(invalid_input(format!(
            "Chỉ xoá được tối đa {MAX_SPECIAL_RANGE_DAYS} ngày một lần"
        )));
    }
    for date in &request.remove {
        let removed = parse_date_only(date, "Ngày cần xoá")?;
        if removed >= from && removed <= to {
            return Err(invalid_input(
                "Ngày cần xoá không được nằm trong chính khoảng đang khai",
            ));
        }
    }

    let mut tx = pool.begin().await.map_err(|error| {
        crate::app_error::log_system_error(
            "save_special_date_range",
            error.to_string(),
            serde_json::json!({ "step": "begin" }),
        )
    })?;

    // Xoá trước, ghi sau. Ngược lại thì một ngày vừa nằm trong `remove` vừa
    // nằm trong khoảng sẽ bị xoá mất ngay bên trong transaction dựng lên để
    // đừng mất nó. Đã chặn chồng lấn ở trên, nhưng thứ tự vẫn phải đúng.
    delete_special_dates_tx(&mut tx, &request.remove)
        .await
        .map_err(|error| {
            crate::app_error::log_system_error(
                "save_special_date_range",
                error.to_string(),
                serde_json::json!({ "step": "delete_special_dates_tx" }),
            )
        })?;

    let mut date = from;
    for index in 0..day_count {
        let id = format!("{id_base}-{index}");
        let date_key = date.format("%Y-%m-%d").to_string();
        upsert_special_date_tx(
            &mut tx,
            &SpecialDateUpsert {
                id: &id,
                date: &date_key,
                label,
                uplift_pct: request.uplift_pct,
                now: &now,
            },
        )
        .await
        .map_err(|error| {
            crate::app_error::log_system_error(
                "save_special_date_range",
                error.to_string(),
                serde_json::json!({ "step": "upsert_special_date_tx", "date": &date_key }),
            )
        })?;
        date = date.succ_opt().unwrap_or(date);
    }

    tx.commit().await.map_err(|error| {
        crate::app_error::log_system_error(
            "save_special_date_range",
            error.to_string(),
            serde_json::json!({ "step": "commit" }),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::{
        calculate_price_preview, calculate_room_price_preview, calculate_stay_price_tx,
        save_pricing_rule, save_special_date_range, SavePricingRule, SaveSpecialDateRange,
    };
    use crate::app_error::codes;
    use crate::domain::booking::BookingError;
    use sqlx::{sqlite::SqlitePoolOptions, Executor, Pool, Row, Sqlite};

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

    /// Dựng riêng cho phần khai mùa cao điểm — chỉ bảng `special_dates`, không
    /// chạy migrations đầy đủ như `migrated_pool`, để không đụng vào các test
    /// giá đang dựa vào đúng bộ bảng của hàm đó.
    async fn special_dates_pool() -> Pool<Sqlite> {
        use sqlx::sqlite::SqlitePoolOptions;
        use sqlx::Executor;

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        pool.execute(
            "CREATE TABLE special_dates (
                id TEXT PRIMARY KEY,
                date TEXT NOT NULL,
                label TEXT NOT NULL DEFAULT '',
                uplift_pct REAL NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                UNIQUE(date)
            )",
        )
        .await
        .unwrap();
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

    /// `seed_room` with the guest columns opened up: the type-keyed preview has
    /// to stand a room in for the type, so tests need rooms that disagree.
    async fn seed_room_with_guests(
        pool: &Pool<Sqlite>,
        room_id: &str,
        room_type: &str,
        base_price: i64,
        max_guests: i32,
        extra_person_fee: i64,
    ) {
        sqlx::query(
            "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status)
             VALUES (?, ?, ?, 1, 0, ?, ?, ?, 'vacant')",
        )
        .bind(room_id)
        .bind(format!("Room {room_id}"))
        .bind(room_type)
        .bind(base_price)
        .bind(max_guests)
        .bind(extra_person_fee)
        .execute(pool)
        .await
        .expect("seed room with guests");
    }

    async fn seed_special_date(pool: &Pool<Sqlite>, date: &str, uplift_pct: f64) {
        sqlx::query(
            "INSERT INTO special_dates (id, date, label, uplift_pct, created_at)
             VALUES (?, ?, 'Lễ', ?, '2026-04-01T00:00:00+07:00')",
        )
        .bind(format!("sd-{date}"))
        .bind(date)
        .bind(uplift_pct)
        .execute(pool)
        .await
        .expect("seed special date");
    }

    /// Price belongs to the room type, so the room a guest is given must not
    /// change what they pay. This is the test that says so.
    ///
    /// The case that can break it: no `pricing_rules` row, so the type price is
    /// derived from *one* room's `base_price` via `FALLBACK_BASE_PRICE_SQL`.
    /// Three properties, all of which the decision requires:
    ///
    /// 1. Every room of the type is charged the same — the 800k room and the
    ///    600k room cost the same night.
    /// 2. The quote and the charge derive from the same room. They do only
    ///    because both loaders share one SQL constant.
    /// 3. The pick does not drift between runs. `ORDER BY id` is what makes that
    ///    true; without it SQLite is free to return either row.
    #[tokio::test]
    async fn the_room_a_guest_is_given_does_not_change_what_the_type_costs() {
        let pool = migrated_pool().await;
        // Inserted expensive-first so insertion order and id order disagree.
        seed_room(&pool, "R-902", "mixed", 800_000).await;
        seed_room(&pool, "R-901", "mixed", 600_000).await;

        let preview = calculate_price_preview(&pool, "mixed", CHECK_IN, CHECK_OUT, "nightly", None)
            .await
            .expect("preview");

        let mut charged_by_room = Vec::new();
        for room_id in ["R-901", "R-902"] {
            let mut tx = pool.begin().await.expect("begin");
            let charged =
                calculate_stay_price_tx(&mut tx, room_id, CHECK_IN, CHECK_OUT, "nightly", None)
                    .await
                    .unwrap_or_else(|error| panic!("charge for {room_id}: {error}"));
            tx.rollback().await.expect("rollback");

            assert_eq!(
                preview.total, charged.total,
                "the quote and the charge disagreed for {room_id}"
            );
            charged_by_room.push(charged.total);
        }

        // The decision itself, stated without going through the preview: two
        // rooms of one type, 200k apart in `base_price`, cost the same night.
        assert_eq!(
            charged_by_room[0], charged_by_room[1],
            "the cheap room and the expensive room of one type were charged differently"
        );

        // Which room won: the lowest id, not the lowest price, not the insertion
        // order. Compared against types holding exactly one room at that price,
        // so the expectation does not restate the derivation arithmetic.
        seed_room(&pool, "R-801", "lone-600", 600_000).await;
        seed_room(&pool, "R-802", "lone-800", 800_000).await;
        let lone_600 =
            calculate_price_preview(&pool, "lone-600", CHECK_IN, CHECK_OUT, "nightly", None)
                .await
                .expect("lone 600k preview");
        let lone_800 =
            calculate_price_preview(&pool, "lone-800", CHECK_IN, CHECK_OUT, "nightly", None)
                .await
                .expect("lone 800k preview");

        assert_eq!(
            preview.total, lone_600.total,
            "R-901 has the lower id, so its 600k sets the mixed-type price"
        );
        assert_ne!(
            preview.total, lone_800.total,
            "a type cannot hold two prices: the 800k room bills at its sibling's rate"
        );
    }

    /// A room type named in Vietnamese must be able to hold a price.
    ///
    /// The lookups fold case on both sides — `LOWER(type) = ?` in SQL against a
    /// `str::to_lowercase()` bind — and the two folds are not the same function.
    /// SQLite's `LOWER` is ASCII-only, so it leaves `Đ` alone; Rust's lowers it to
    /// `đ`. For any type whose name carries a non-ASCII capital, the two strings
    /// could never be equal, so *every* lookup missed: no stored rule was found,
    /// no room supplied a fallback, and the quote came out at the 350k house
    /// default. An operator could set 900k in settings and watch the desk read
    /// 350k to the guest, with nothing anywhere reporting a problem.
    ///
    /// `Đ` is not an exotic character here. It opens `Đôi` and `Đơn` — double and
    /// single — which is what half the room types in a Vietnamese hotel are called.
    #[tokio::test]
    async fn a_room_type_named_in_vietnamese_keeps_the_price_configured_for_it() {
        let pool = migrated_pool().await;
        seed_room(&pool, "R-201", "Phòng Đôi", 640_000).await;
        save_pricing_rule(
            &pool,
            SavePricingRule {
                room_type: "Phòng Đôi".to_string(),
                hourly_rate: 100_000,
                overnight_rate: 500_000,
                daily_rate: 900_000,
                ..request(0, 0, 0)
            },
            "rule-vn".to_string(),
            "2026-04-01T00:00:00+07:00".to_string(),
        )
        .await
        .expect("save rule");

        let quoted =
            calculate_room_price_preview(&pool, "R-201", CHECK_IN, CHECK_OUT, "nightly", None)
                .await
                .expect("preview");

        // Two nights at the configured 900k. Stated against the arithmetic
        // rather than a magic number, because what is being asserted is *which
        // rate was found*, and the two candidates are far apart: 900k configured,
        // 640k derived from the room, 350k the house default.
        assert_eq!(
            quoted.total,
            900_000 * 2,
            "the rule configured for Phòng Đôi was not the rule applied"
        );

        // And the type-keyed preview agrees, so the miss is not hiding in one of
        // the two lookup paths.
        let by_type =
            calculate_price_preview(&pool, "Phòng Đôi", CHECK_IN, CHECK_OUT, "nightly", None)
                .await
                .expect("type preview");
        assert_eq!(
            by_type.total, quoted.total,
            "the type-keyed and room-keyed previews found different rules"
        );
    }

    /// A type-keyed quote must take *all* of its per-room stand-ins from one
    /// room. `FALLBACK_BASE_PRICE_SQL` picks the lowest id; the guest limits used
    /// to be picked by `LIMIT 1` with no `ORDER BY`, so SQLite was free to supply
    /// them from a different room than the base price came from — a total
    /// assembled from two rooms, and one that could change on restart.
    #[tokio::test]
    async fn a_type_keyed_quote_takes_its_base_price_and_its_guest_limits_from_one_room() {
        let pool = migrated_pool().await;
        // Inserted high-id first, and the two rooms disagree about every column
        // the derivation reads, so a mismatched pick shows up as a wrong total
        // rather than cancelling out.
        // Named in Vietnamese on purpose: this test also holds the case-fold, so a
        // lookup that stops matching `Đ` fails here as well as in the rule test.
        seed_room_with_guests(&pool, "R-912", "Phòng Đôi", 800_000, 4, 0).await;
        seed_room_with_guests(&pool, "R-911", "Phòng Đôi", 600_000, 2, 150_000).await;

        let quoted =
            calculate_price_preview(&pool, "Phòng Đôi", CHECK_IN, CHECK_OUT, "nightly", Some(3))
                .await
                .expect("type preview for 3 guests");

        // R-911 holds the lowest id, so it is the room the type stands on: its
        // 600k base *and* its 150k surcharge, not one of each.
        let lowest_id =
            calculate_room_price_preview(&pool, "R-911", CHECK_IN, CHECK_OUT, "nightly", Some(3))
                .await
                .expect("R-911 preview");
        let other_room =
            calculate_room_price_preview(&pool, "R-912", CHECK_IN, CHECK_OUT, "nightly", Some(3))
                .await
                .expect("R-912 preview");

        assert_eq!(
            quoted.total, lowest_id.total,
            "the type quote should match the room its price is derived from"
        );
        assert_ne!(
            quoted.total, other_room.total,
            "R-912 seats 4, so it charges no surcharge for a third guest — if the \
             type quote matches it, the guest limits came from the wrong room"
        );

        // And the surcharge is genuinely in there: without it the two rooms would
        // agree, and the assertion above would pass on an empty difference.
        let no_guest_count =
            calculate_price_preview(&pool, "Phòng Đôi", CHECK_IN, CHECK_OUT, "nightly", None)
                .await
                .expect("type preview with no guest count");
        assert!(
            quoted.total > no_guest_count.total,
            "third guest cost nothing: {} vs {}",
            quoted.total,
            no_guest_count.total
        );
    }

    /// Both previews used to swallow a failed `special_dates` read and quote a
    /// 0% uplift. On a holiday that is a number below what check-in charges,
    /// read aloud to the guest before anyone takes their money.
    #[tokio::test]
    async fn a_failed_special_date_read_fails_both_previews_instead_of_quoting_no_uplift() {
        let pool = migrated_pool().await;
        seed_room(&pool, "R-901", "derived", 500_000).await;
        seed_special_date(&pool, "2026-04-20", 10.0).await;

        // Precondition: the holiday really is reaching both previews, so the
        // failure below is about the read and not about an uplift that was never
        // applied to begin with.
        let by_type =
            calculate_price_preview(&pool, "derived", CHECK_IN, CHECK_OUT, "nightly", None)
                .await
                .expect("type preview");
        let by_room =
            calculate_room_price_preview(&pool, "R-901", CHECK_IN, CHECK_OUT, "nightly", None)
                .await
                .expect("room preview");
        assert!(
            by_type.surcharge_amount > 0,
            "precondition: the holiday uplift is not reaching the type preview"
        );
        assert_eq!(
            by_type.total, by_room.total,
            "the two previews disagreed before anything was broken"
        );

        pool.execute("DROP TABLE special_dates")
            .await
            .expect("drop special_dates");

        // Pinned on the error *variant*, not on SQLite's wording. Only this
        // test's own DROP produces "no such table", so asserting that text would
        // prove the test rather than the code.
        let type_error =
            calculate_price_preview(&pool, "derived", CHECK_IN, CHECK_OUT, "nightly", None)
                .await
                .expect_err("the type preview quoted a price it could not read");
        assert!(
            matches!(type_error, BookingError::Database(_)),
            "type preview: {type_error:?}"
        );

        let room_error =
            calculate_room_price_preview(&pool, "R-901", CHECK_IN, CHECK_OUT, "nightly", None)
                .await
                .expect_err("the room preview quoted a price it could not read");
        assert!(
            matches!(room_error, BookingError::Database(_)),
            "room preview: {room_error:?}"
        );
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

    #[tokio::test]
    async fn save_special_date_range_writes_one_row_per_day() {
        let pool = special_dates_pool().await;

        save_special_date_range(
            &pool,
            SaveSpecialDateRange {
                remove: Vec::new(),
                from: "2026-02-14".to_string(),
                to: "2026-02-22".to_string(),
                label: "Tết Nguyên đán".to_string(),
                uplift_pct: 40.0,
            },
            "base".to_string(),
            "2026-01-01T00:00:00+07:00".to_string(),
        )
        .await
        .unwrap();

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM special_dates")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 9);
    }

    #[tokio::test]
    async fn save_special_date_range_accepts_a_single_day() {
        let pool = special_dates_pool().await;

        save_special_date_range(
            &pool,
            SaveSpecialDateRange {
                remove: Vec::new(),
                from: "2026-04-30".to_string(),
                to: "2026-04-30".to_string(),
                label: "Lễ 30/4".to_string(),
                uplift_pct: 30.0,
            },
            "base".to_string(),
            "2026-01-01T00:00:00+07:00".to_string(),
        )
        .await
        .unwrap();

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM special_dates")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 1);
    }

    #[tokio::test]
    async fn a_failed_write_leaves_every_removed_day_in_place() {
        // Test quan trọng nhất của task này: `remove` tồn tại để việc xoá và
        // việc ghi cùng chung một transaction.
        //
        // Ép hỏng giữa chừng bằng cách cho `id_base` sinh ra một mã đã tồn tại
        // trên một ngày KHÁC, để bước upsert đụng khoá chính. `ON CONFLICT` chỉ
        // đỡ cho `date`, không đỡ cho `id` — đây là chỗ hỏng-được-theo-ý duy
        // nhất, và nó chỉ dùng được vì `id` do bên ngoài truyền vào.
        let pool = special_dates_pool().await;
        sqlx::query(
            "INSERT INTO special_dates (id, date, label, uplift_pct, created_at)
             VALUES ('base-1', '2026-12-25', 'Noel', 10.0, '2026-01-01T00:00:00+07:00'),
                    ('keep-1', '2026-02-25', 'Tết', 40.0, '2026-01-01T00:00:00+07:00')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let error = save_special_date_range(
            &pool,
            SaveSpecialDateRange {
                remove: vec!["2026-02-25".to_string()],
                from: "2026-02-14".to_string(),
                to: "2026-02-16".to_string(),
                label: "Tết".to_string(),
                uplift_pct: 40.0,
            },
            "base".to_string(),
            "2026-06-01T00:00:00+07:00".to_string(),
        )
        .await
        .expect_err("mã trùng phải làm cả transaction hỏng");
        let _ = error;

        let kept: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM special_dates WHERE date = '2026-02-25'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            kept.0, 1,
            "ngày trong `remove` phải còn nguyên khi ghi hỏng"
        );

        let written: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM special_dates WHERE date = '2026-02-14'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(written.0, 0, "không được để lại nửa khoảng");
    }

    #[tokio::test]
    async fn remove_is_deleted_before_the_range_is_written() {
        let pool = special_dates_pool().await;
        sqlx::query(
            "INSERT INTO special_dates (id, date, label, uplift_pct, created_at)
             VALUES ('old-1', '2026-02-20', 'Tết', 40.0, '2026-01-01T00:00:00+07:00')",
        )
        .execute(&pool)
        .await
        .unwrap();

        save_special_date_range(
            &pool,
            SaveSpecialDateRange {
                remove: vec!["2026-02-20".to_string()],
                from: "2026-02-14".to_string(),
                to: "2026-02-16".to_string(),
                label: "Tết".to_string(),
                uplift_pct: 40.0,
            },
            "base".to_string(),
            "2026-06-01T00:00:00+07:00".to_string(),
        )
        .await
        .unwrap();

        let dates: Vec<(String,)> = sqlx::query_as("SELECT date FROM special_dates ORDER BY date")
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(
            dates.iter().map(|row| row.0.as_str()).collect::<Vec<_>>(),
            vec!["2026-02-14", "2026-02-15", "2026-02-16"]
        );
    }

    #[tokio::test]
    async fn save_special_date_range_rejects_bad_input() {
        let pool = special_dates_pool().await;

        let base = || SaveSpecialDateRange {
            remove: Vec::new(),
            from: "2026-02-14".to_string(),
            to: "2026-02-16".to_string(),
            label: "Tết".to_string(),
            uplift_pct: 40.0,
        };
        let run = |request| {
            let pool = pool.clone();
            async move {
                save_special_date_range(
                    &pool,
                    request,
                    "base".to_string(),
                    "2026-01-01T00:00:00+07:00".to_string(),
                )
                .await
            }
        };

        // ngày sai định dạng
        let mut request = base();
        request.from = "14/02/2026".to_string();
        run(request).await.expect_err("ngày sai định dạng");

        // ngày kết thúc trước ngày bắt đầu
        let mut request = base();
        request.to = "2026-02-10".to_string();
        run(request).await.expect_err("to < from");

        // quá 366 ngày
        let mut request = base();
        request.to = "2027-02-16".to_string();
        run(request).await.expect_err("367 ngày");

        // phần trăm âm
        let mut request = base();
        request.uplift_pct = -10.0;
        run(request).await.expect_err("phần trăm âm");

        // phần trăm quá trần
        let mut request = base();
        request.uplift_pct = 600.0;
        run(request).await.expect_err("phần trăm quá 500");

        // nhãn toàn khoảng trắng
        let mut request = base();
        request.label = "   ".to_string();
        run(request).await.expect_err("nhãn rỗng");

        // phần tử của remove sai định dạng
        let mut request = base();
        request.remove = vec!["hôm-qua".to_string()];
        run(request).await.expect_err("remove sai định dạng");

        // remove chồng lên khoảng đang khai
        let mut request = base();
        request.remove = vec!["2026-02-15".to_string()];
        run(request).await.expect_err("remove chồng lấn");

        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM special_dates")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 0, "không lần nào được ghi gì");
    }
}
