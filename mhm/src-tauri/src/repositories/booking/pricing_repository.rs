//! Pricing-configuration writes.
//!
//! Both are upserts keyed on the natural business key — one rule per room type,
//! one uplift per date — so saving twice replaces rather than duplicating.

use sqlx::{Pool, Sqlite, Transaction};

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

/// Giá trị đã được service kiểm và đã đủ, writer không quyết định gì.
pub struct SpecialDateUpsert<'a> {
    pub id: &'a str,
    pub date: &'a str,
    pub label: &'a str,
    pub uplift_pct: f64,
    pub now: &'a str,
}

/// `created_at` chỉ được ghi lúc thêm mới; lần cập nhật giữ nguyên mốc cũ, nên
/// nó không nằm trong danh sách `DO UPDATE SET`. `id` cũng vậy: ngày đã tồn tại
/// thì giữ mã cũ, để mọi tham chiếu bên ngoài còn dùng được.
///
/// Nhận `tx` chứ không nhận `pool`: khai một khoảng là nhiều dòng, và mất nửa
/// khoảng còn tệ hơn báo lỗi.
pub async fn upsert_special_date_tx(
    tx: &mut Transaction<'_, Sqlite>,
    upsert: &SpecialDateUpsert<'_>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO special_dates (id, date, label, uplift_pct, created_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(date) DO UPDATE SET
            label = excluded.label,
            uplift_pct = excluded.uplift_pct",
    )
    .bind(upsert.id)
    .bind(upsert.date)
    .bind(upsert.label)
    .bind(upsert.uplift_pct)
    .bind(upsert.now)
    .execute(&mut **tx)
    .await?;

    Ok(())
}

/// Xoá từng ngày một thay vì dựng `IN (…)`: sqlx không bind được mảng cho
/// SQLite, và ghép chuỗi SQL động là thứ không đáng đổi lấy một vòng lặp tối đa
/// 366 bước.
pub async fn delete_special_dates_tx(
    tx: &mut Transaction<'_, Sqlite>,
    dates: &[String],
) -> Result<(), sqlx::Error> {
    for date in dates {
        sqlx::query("DELETE FROM special_dates WHERE date = ?")
            .bind(date)
            .execute(&mut **tx)
            .await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{delete_special_dates_tx, upsert_special_date_tx, SpecialDateUpsert};
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::{Executor, Pool, Sqlite};

    async fn test_pool() -> Pool<Sqlite> {
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

    #[tokio::test]
    async fn upsert_keeps_the_original_created_at_when_the_date_already_exists() {
        let pool = test_pool().await;
        let mut tx = pool.begin().await.unwrap();
        upsert_special_date_tx(
            &mut tx,
            &SpecialDateUpsert {
                id: "id-1",
                date: "2026-02-14",
                label: "Tết",
                uplift_pct: 40.0,
                now: "2026-01-01T00:00:00+07:00",
            },
        )
        .await
        .unwrap();
        upsert_special_date_tx(
            &mut tx,
            &SpecialDateUpsert {
                id: "id-2",
                date: "2026-02-14",
                label: "Tết Nguyên đán",
                uplift_pct: 45.0,
                now: "2026-06-01T00:00:00+07:00",
            },
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let row: (String, String, f64, String) = sqlx::query_as(
            "SELECT id, label, uplift_pct, created_at FROM special_dates WHERE date = ?",
        )
        .bind("2026-02-14")
        .fetch_one(&pool)
        .await
        .unwrap();

        assert_eq!(row.0, "id-1", "id cũ phải được giữ");
        assert_eq!(row.1, "Tết Nguyên đán");
        assert_eq!(row.2, 45.0);
        assert_eq!(
            row.3, "2026-01-01T00:00:00+07:00",
            "created_at phải được giữ"
        );
    }

    #[tokio::test]
    async fn delete_removes_exactly_the_listed_dates() {
        let pool = test_pool().await;
        let mut tx = pool.begin().await.unwrap();
        for (index, date) in ["2026-02-14", "2026-02-15", "2026-02-16"]
            .iter()
            .enumerate()
        {
            upsert_special_date_tx(
                &mut tx,
                &SpecialDateUpsert {
                    id: &format!("id-{index}"),
                    date,
                    label: "Tết",
                    uplift_pct: 40.0,
                    now: "2026-01-01T00:00:00+07:00",
                },
            )
            .await
            .unwrap();
        }
        delete_special_dates_tx(&mut tx, &["2026-02-15".to_string()])
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let remaining: Vec<(String,)> =
            sqlx::query_as("SELECT date FROM special_dates ORDER BY date")
                .fetch_all(&pool)
                .await
                .unwrap();

        assert_eq!(
            remaining
                .iter()
                .map(|row| row.0.as_str())
                .collect::<Vec<_>>(),
            vec!["2026-02-14", "2026-02-16"]
        );
    }
}
