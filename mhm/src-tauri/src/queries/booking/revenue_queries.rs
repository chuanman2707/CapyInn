use chrono::{Duration, NaiveDate};
use sqlx::{Pool, Row, Sqlite};

use crate::db::local_day::{date_plus_days_sql, date_sql, local_date_sql};
use crate::{
    db::row::get_money_vnd,
    models::{
        AnalyticsData, CategoryExpense, DailyRevenue, DashboardStats, RevenueStats, RoomRevenue,
        SourceRevenue,
    },
    money::MoneyVnd,
};

pub async fn load_dashboard_stats_for_date(
    pool: &Pool<Sqlite>,
    date: &str,
) -> Result<DashboardStats, sqlx::Error> {
    let total: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM rooms")
        .fetch_one(pool)
        .await?;
    let occupied: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM rooms WHERE status = 'occupied'")
        .fetch_one(pool)
        .await?;
    let vacant: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM rooms WHERE status = 'vacant'")
        .fetch_one(pool)
        .await?;
    let cleaning: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM rooms WHERE status = 'cleaning'")
        .fetch_one(pool)
        .await?;

    Ok(DashboardStats {
        total_rooms: total.0 as i32,
        occupied: occupied.0 as i32,
        vacant: vacant.0 as i32,
        cleaning: cleaning.0 as i32,
        revenue_today: load_total_revenue(pool, date, date).await?,
    })
}

pub async fn load_revenue_stats(
    pool: &Pool<Sqlite>,
    from: &str,
    to: &str,
) -> Result<RevenueStats, sqlx::Error> {
    let rooms_sold = load_rooms_sold(pool, from, to).await?;
    let total_rooms: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM rooms")
        .fetch_one(pool)
        .await?;

    Ok(RevenueStats {
        total_revenue: load_total_revenue(pool, from, to).await?,
        rooms_sold,
        occupancy_rate: if total_rooms.0 > 0 {
            (rooms_sold as f64 / total_rooms.0 as f64) * 100.0
        } else {
            0.0
        },
        daily_revenue: load_daily_revenue(pool, from, to).await?,
    })
}

pub async fn load_analytics(
    pool: &Pool<Sqlite>,
    from: &str,
    to: &str,
    period_days: i64,
) -> Result<AnalyticsData, sqlx::Error> {
    let total_revenue = load_total_revenue(pool, from, to).await?;
    let room_revenue = load_room_revenue(pool, from, to).await?;
    let rooms_sold = load_rooms_sold(pool, from, to).await?;
    let room_nights_sold = load_room_nights_sold(pool, from, to).await?;
    let recognized_room_revenue_amount = recognized_room_revenue_amount_sql("b.");
    let recognized_room_revenue_filter = recognized_room_revenue_filter_sql("b.");
    let folio_created_date = local_date_sql("fl.created_at");
    let transaction_created_date = local_date_sql("t.created_at");
    let range_from = local_date_sql("?2");
    let range_to = local_date_sql("?1");

    let total_rooms: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM rooms")
        .fetch_one(pool)
        .await?;

    let occupancy_rate = if total_rooms.0 > 0 {
        (rooms_sold as f64 / total_rooms.0 as f64) * 100.0
    } else {
        0.0
    };
    let adr = if room_nights_sold > 0.0 {
        room_revenue as f64 / room_nights_sold
    } else {
        0.0
    };
    let period_days = period_days.max(1) as f64;
    let revpar = if total_rooms.0 > 0 {
        room_revenue as f64 / (total_rooms.0 as f64 * period_days)
    } else {
        0.0
    };

    let source_rows_sql = format!(
        "SELECT source, COALESCE(SUM(amount), 0) AS value
         FROM (
             SELECT COALESCE(b.source, 'walk-in') AS source,
                    {recognized_room_revenue_amount} AS amount
             FROM bookings b
             WHERE {recognized_room_revenue_filter}
             UNION ALL
             SELECT COALESCE(b.source, 'walk-in') AS source, fl.amount AS amount
             FROM folio_lines fl
             JOIN bookings b ON b.id = fl.booking_id
             WHERE b.status != 'voided'
               AND {folio_created_date} BETWEEN {range_from} AND {range_to}
             UNION ALL
             SELECT COALESCE(b.source, 'walk-in') AS source, t.amount AS amount
             FROM transactions t
             JOIN bookings b ON b.id = t.booking_id
             WHERE t.type = 'cancellation_fee'
               AND b.status != 'voided'
               AND {transaction_created_date} BETWEEN {range_from} AND {range_to}
         ) revenue_items
         GROUP BY source
         ORDER BY value DESC"
    );
    let source_rows = sqlx::query(&source_rows_sql)
        .bind(to)
        .bind(from)
        .fetch_all(pool)
        .await?;

    let revenue_by_source = source_rows
        .iter()
        .map(|row| SourceRevenue {
            name: row.get("source"),
            value: get_money_vnd(row, "value"),
        })
        .collect();

    let expense_rows = sqlx::query(
        "SELECT category, COALESCE(SUM(amount), 0) AS amount
         FROM expenses
         WHERE DATE(expense_date) BETWEEN DATE(?) AND DATE(?)
         GROUP BY category
         ORDER BY amount DESC",
    )
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;

    let expenses_by_category = expense_rows
        .iter()
        .map(|row| CategoryExpense {
            category: row.get("category"),
            amount: get_money_vnd(row, "amount"),
        })
        .collect();

    let room_rows_sql = format!(
        "SELECT room_id, COALESCE(SUM(amount), 0) AS value
         FROM (
             SELECT b.room_id AS room_id,
                    {recognized_room_revenue_amount} AS amount
             FROM bookings b
             WHERE {recognized_room_revenue_filter}
             UNION ALL
             SELECT b.room_id AS room_id, fl.amount AS amount
             FROM folio_lines fl
             JOIN bookings b ON b.id = fl.booking_id
             WHERE b.status != 'voided'
               AND {folio_created_date} BETWEEN {range_from} AND {range_to}
             UNION ALL
             SELECT b.room_id AS room_id, t.amount AS amount
             FROM transactions t
             JOIN bookings b ON b.id = t.booking_id
             WHERE t.type = 'cancellation_fee'
               AND b.status != 'voided'
               AND {transaction_created_date} BETWEEN {range_from} AND {range_to}
         ) revenue_items
         GROUP BY room_id
         ORDER BY value DESC
         LIMIT 5"
    );
    let room_rows = sqlx::query(&room_rows_sql)
        .bind(to)
        .bind(from)
        .fetch_all(pool)
        .await?;

    let top_rooms = room_rows
        .iter()
        .map(|row| RoomRevenue {
            room: row.get("room_id"),
            revenue: get_money_vnd(row, "value"),
        })
        .collect();

    Ok(AnalyticsData {
        total_revenue,
        occupancy_rate,
        adr,
        revpar,
        daily_revenue: load_daily_revenue(pool, from, to).await?,
        revenue_by_source,
        expenses_by_category,
        top_rooms,
    })
}

pub async fn load_room_revenue(
    pool: &Pool<Sqlite>,
    from: &str,
    to: &str,
) -> Result<MoneyVnd, sqlx::Error> {
    let room_revenue_sql = format!(
        "SELECT COALESCE(SUM(
            {}
         ), 0) AS value
         FROM bookings
         WHERE {}",
        recognized_room_revenue_amount_sql(""),
        recognized_room_revenue_filter_sql(""),
    );
    let row = sqlx::query(&room_revenue_sql)
        .bind(to)
        .bind(from)
        .fetch_one(pool)
        .await?;

    Ok(get_money_vnd(&row, "value"))
}

pub async fn load_folio_revenue(
    pool: &Pool<Sqlite>,
    from: &str,
    to: &str,
) -> Result<MoneyVnd, sqlx::Error> {
    let created_date = local_date_sql("fl.created_at");
    let range_from = local_date_sql("?");
    let range_to = local_date_sql("?");
    // JOIN bookings là bắt buộc, không phải trang trí: không có nó thì dịch vụ
    // của một lượt đã bị xoá vì nhập sai vẫn cộng vào doanh thu.
    let row = sqlx::query(&format!(
        "SELECT COALESCE(SUM(fl.amount), 0) AS value
         FROM folio_lines fl
         JOIN bookings b ON b.id = fl.booking_id
         WHERE b.status != 'voided'
           AND {created_date} BETWEEN {range_from} AND {range_to}"
    ))
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await?;

    Ok(get_money_vnd(&row, "value"))
}

pub async fn load_cancellation_fee_revenue(
    pool: &Pool<Sqlite>,
    from: &str,
    to: &str,
) -> Result<MoneyVnd, sqlx::Error> {
    let created_date = local_date_sql("t.created_at");
    let range_from = local_date_sql("?");
    let range_to = local_date_sql("?");
    let row = sqlx::query(&format!(
        "SELECT COALESCE(SUM(t.amount), 0) AS value
         FROM transactions t
         JOIN bookings b ON b.id = t.booking_id
         WHERE t.type = 'cancellation_fee'
           AND b.status != 'voided'
           AND {created_date} BETWEEN {range_from} AND {range_to}"
    ))
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await?;

    Ok(get_money_vnd(&row, "value"))
}

pub async fn load_total_revenue(
    pool: &Pool<Sqlite>,
    from: &str,
    to: &str,
) -> Result<MoneyVnd, sqlx::Error> {
    Ok(load_room_revenue(pool, from, to).await?
        + load_folio_revenue(pool, from, to).await?
        + load_cancellation_fee_revenue(pool, from, to).await?)
}

pub async fn load_daily_revenue(
    pool: &Pool<Sqlite>,
    from: &str,
    to: &str,
) -> Result<Vec<DailyRevenue>, sqlx::Error> {
    let start = normalize_date(from);
    let end = normalize_date(to);
    let mut current = start;
    let mut daily_revenue = Vec::new();

    while current <= end {
        let day = current.format("%Y-%m-%d").to_string();
        let revenue = load_total_revenue(pool, &day, &day).await?;
        if revenue > 0 {
            daily_revenue.push(DailyRevenue { date: day, revenue });
        }
        current += Duration::days(1);
    }

    Ok(daily_revenue)
}

async fn load_rooms_sold(pool: &Pool<Sqlite>, from: &str, to: &str) -> Result<i32, sqlx::Error> {
    let occupancy_checkout = occupancy_checkout_date_sql("");
    let check_in_date = date_sql("check_in_at");
    let range_start = date_sql("?2");
    let range_end_exclusive = date_plus_days_sql("?1", 1);
    let query = format!(
        "SELECT COUNT(DISTINCT room_id)
         FROM bookings
         WHERE status IN ('active', 'checked_out')
           AND {check_in_date} < {range_end_exclusive}
           AND DATE({occupancy_checkout}) > {range_start}"
    );
    let row: (i64,) = sqlx::query_as(&query)
        .bind(to)
        .bind(from)
        .fetch_one(pool)
        .await?;

    Ok(row.0 as i32)
}

async fn load_room_nights_sold(
    pool: &Pool<Sqlite>,
    from: &str,
    to: &str,
) -> Result<f64, sqlx::Error> {
    let recognized_checkout = recognized_checkout_date_sql("");
    let check_in_date = date_sql("check_in_at");
    let range_start = date_sql("?2");
    let range_end_exclusive = date_plus_days_sql("?1", 1);
    let row: (f64,) = sqlx::query_as(&format!(
        "SELECT CAST(COALESCE(SUM(
            MAX(
                0,
                JULIANDAY(MIN(DATE({recognized_checkout}), {range_end_exclusive})) -
                JULIANDAY(MAX({check_in_date}, {range_start}))
            )
         ), 0) AS REAL)
         FROM bookings
         WHERE {}",
        recognized_room_revenue_filter_sql(""),
    ))
    .bind(to)
    .bind(from)
    .fetch_one(pool)
    .await?;

    Ok(row.0)
}

fn normalize_date(value: &str) -> NaiveDate {
    let normalized = value.get(..10).unwrap_or(value);
    NaiveDate::parse_from_str(normalized, "%Y-%m-%d")
        .expect("revenue query dates should always be normalized")
}

pub(crate) fn recognized_checkout_date_sql(column_prefix: &str) -> String {
    let reporting_checkout = date_sql(&format!(
        "json_extract({column_prefix}pricing_snapshot, '$.checkout_settlement.reporting_checkout')"
    ));
    let fallback_checkout = date_sql(&format!(
        "COALESCE({column_prefix}actual_checkout, {column_prefix}expected_checkout)"
    ));
    format!(
        "COALESCE(
            {reporting_checkout},
            {fallback_checkout}
        )"
    )
}

pub(crate) fn occupancy_checkout_date_sql(column_prefix: &str) -> String {
    let stay_checkout = date_sql(&format!(
        "COALESCE({column_prefix}actual_checkout, {column_prefix}expected_checkout)"
    ));
    let check_in = date_sql(&format!("{column_prefix}check_in_at"));
    let check_in_plus_one = date_plus_days_sql(&format!("{column_prefix}check_in_at"), 1);
    format!(
        "CASE
            WHEN json_extract({column_prefix}pricing_snapshot, '$.checkout_settlement.mode') IN ('actual_nights', 'hourly')
                 AND {stay_checkout} <= {check_in}
            THEN {check_in_plus_one}
            ELSE {stay_checkout}
        END"
    )
}

fn recognized_room_revenue_amount_sql(column_prefix: &str) -> String {
    let recognized_checkout = recognized_checkout_date_sql(column_prefix);
    let check_in_date = date_sql(&format!("{column_prefix}check_in_at"));
    let range_start = date_sql("?2");
    let range_end_exclusive = date_plus_days_sql("?1", 1);
    format!(
        "CASE
            WHEN {column_prefix}nights > 0 THEN {column_prefix}total_price * (
                MAX(
                    0,
                    JULIANDAY(MIN(DATE({recognized_checkout}), {range_end_exclusive})) -
                    JULIANDAY(MAX({check_in_date}, {range_start}))
                )
            ) / {column_prefix}nights
            ELSE 0
        END"
    )
}

fn recognized_room_revenue_filter_sql(column_prefix: &str) -> String {
    let recognized_checkout = recognized_checkout_date_sql(column_prefix);
    let check_in_date = date_sql(&format!("{column_prefix}check_in_at"));
    let range_start = date_sql("?2");
    let range_end_exclusive = date_plus_days_sql("?1", 1);
    format!(
        "{column_prefix}status IN ('active', 'checked_out')
         AND {check_in_date} < {range_end_exclusive}
         AND DATE({recognized_checkout}) > {range_start}"
    )
}
