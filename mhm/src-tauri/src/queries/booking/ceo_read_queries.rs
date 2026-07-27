use serde::Serialize;
use sqlx::{Pool, Row, Sqlite};

use crate::{db::row::get_money_vnd, money::MoneyVnd};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CeoBookingSummary {
    pub booking_id: String,
    pub room_id: String,
    pub guest_name: String,
    pub scheduled_checkin: Option<String>,
    pub scheduled_checkout: Option<String>,
    pub expected_checkout: String,
    pub total_price: MoneyVnd,
    pub paid_amount: MoneyVnd,
    pub balance_due: MoneyVnd,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CeoOperationalRisk {
    pub code: String,
    pub severity: String,
    pub count: i64,
    pub summary: String,
}

pub async fn list_today_arrivals(
    pool: &Pool<Sqlite>,
    business_date: &str,
) -> Result<Vec<CeoBookingSummary>, sqlx::Error> {
    let arrival_date = local_date("COALESCE(NULLIF(b.scheduled_checkin, ''), b.check_in_at)");
    let where_clause = format!(
        "b.status = 'booked'
         AND {arrival_date} = ?"
    );
    load_booking_summaries(
        pool,
        &where_clause,
        business_date,
        "COALESCE(NULLIF(b.scheduled_checkin, ''), b.check_in_at) ASC, b.room_id ASC",
    )
    .await
}

pub async fn list_today_checkouts(
    pool: &Pool<Sqlite>,
    business_date: &str,
) -> Result<Vec<CeoBookingSummary>, sqlx::Error> {
    let checkout_date = local_date("b.expected_checkout");
    let where_clause = format!(
        "b.status = 'active'
         AND {checkout_date} = ?"
    );
    load_booking_summaries(
        pool,
        &where_clause,
        business_date,
        "b.expected_checkout ASC, b.room_id ASC",
    )
    .await
}

pub async fn list_unpaid_balances(
    pool: &Pool<Sqlite>,
) -> Result<Vec<CeoBookingSummary>, sqlx::Error> {
    let expected_checkout_date = local_date("b.expected_checkout");
    let sql = format!(
        "SELECT b.id AS booking_id,
                b.room_id,
                COALESCE(g.full_name, '') AS guest_name,
                b.scheduled_checkin,
                b.scheduled_checkout,
                b.expected_checkout,
                b.total_price,
                COALESCE(b.paid_amount, 0) AS paid_amount,
                b.total_price - COALESCE(b.paid_amount, 0) AS balance_due,
                b.status
         FROM bookings b
         LEFT JOIN guests g ON g.id = b.primary_guest_id
         WHERE b.status IN ('booked', 'active', 'checked_out')
           AND b.total_price - COALESCE(b.paid_amount, 0) > 0
         ORDER BY balance_due DESC, {expected_checkout_date} ASC, b.room_id ASC",
    );
    let rows = sqlx::query(&sql).fetch_all(pool).await?;

    Ok(rows.iter().map(row_to_booking_summary).collect())
}

pub async fn summarize_operational_risks(
    pool: &Pool<Sqlite>,
    business_date: &str,
) -> Result<Vec<CeoOperationalRisk>, sqlx::Error> {
    let unpaid_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
         FROM bookings
         WHERE status IN ('booked', 'active', 'checked_out')
           AND total_price - COALESCE(paid_amount, 0) > 0",
    )
    .fetch_one(pool)
    .await?;

    let checkout_date = local_date("expected_checkout");
    let checkout_count: i64 = sqlx::query_scalar(&format!(
        "SELECT COUNT(*)
         FROM bookings
         WHERE status = 'active'
           AND {checkout_date} = ?",
    ))
    .bind(business_date)
    .fetch_one(pool)
    .await?;

    let mut risks = Vec::new();
    if unpaid_count > 0 {
        risks.push(CeoOperationalRisk {
            code: "unpaid_balances".to_string(),
            severity: "high".to_string(),
            count: unpaid_count,
            summary: format!("{unpaid_count} booking(s) have unpaid balances."),
        });
    }
    if checkout_count > 0 {
        risks.push(CeoOperationalRisk {
            code: "checkouts_due_today".to_string(),
            severity: "medium".to_string(),
            count: checkout_count,
            summary: format!("{checkout_count} active booking(s) are due to checkout today."),
        });
    }

    Ok(risks)
}

async fn load_booking_summaries(
    pool: &Pool<Sqlite>,
    where_clause: &str,
    business_date: &str,
    order_clause: &str,
) -> Result<Vec<CeoBookingSummary>, sqlx::Error> {
    let sql = format!(
        "SELECT b.id AS booking_id,
                b.room_id,
                COALESCE(g.full_name, '') AS guest_name,
                b.scheduled_checkin,
                b.scheduled_checkout,
                b.expected_checkout,
                b.total_price,
                COALESCE(b.paid_amount, 0) AS paid_amount,
                b.total_price - COALESCE(b.paid_amount, 0) AS balance_due,
                b.status
         FROM bookings b
         LEFT JOIN guests g ON g.id = b.primary_guest_id
         WHERE {where_clause}
         ORDER BY {order_clause}",
    );

    let rows = sqlx::query(&sql)
        .bind(business_date)
        .fetch_all(pool)
        .await?;

    Ok(rows.iter().map(row_to_booking_summary).collect())
}

fn row_to_booking_summary(row: &sqlx::sqlite::SqliteRow) -> CeoBookingSummary {
    CeoBookingSummary {
        booking_id: row.get("booking_id"),
        room_id: row.get("room_id"),
        guest_name: row.get("guest_name"),
        scheduled_checkin: row.get("scheduled_checkin"),
        scheduled_checkout: row.get("scheduled_checkout"),
        expected_checkout: row.get("expected_checkout"),
        total_price: get_money_vnd(row, "total_price"),
        paid_amount: get_money_vnd(row, "paid_amount"),
        balance_due: get_money_vnd(row, "balance_due"),
        status: row.get("status"),
    }
}

fn local_date(expression: &str) -> String {
    format!("substr(NULLIF({expression}, ''), 1, 10)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::money::MoneyVnd;
    use sqlx::{sqlite::SqlitePoolOptions, Pool, Sqlite};

    async fn test_pool() -> Pool<Sqlite> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("failed to open sqlite test pool");
        crate::db::run_migrations(&pool)
            .await
            .expect("run migrations");
        pool
    }

    async fn seed_room_and_guest(pool: &Pool<Sqlite>) {
        sqlx::query(
            "INSERT INTO rooms (
                id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status
            ) VALUES ('101', 'Room 101', 'standard', 1, 0, 900000, 2, 0, 'vacant')",
        )
        .execute(pool)
        .await
        .expect("seed room");

        sqlx::query(
            "INSERT INTO guests (
                id, guest_type, full_name, doc_number, created_at
            ) VALUES ('guest-1', 'domestic', 'CEO Visible Guest', 'DOC-1', '2026-05-06T08:00:00+07:00')",
        )
        .execute(pool)
        .await
        .expect("seed guest");
    }

    async fn seed_booking(
        pool: &Pool<Sqlite>,
        booking_id: &str,
        status: &str,
        check_in_at: &str,
        expected_checkout: &str,
        total_price: MoneyVnd,
        paid_amount: MoneyVnd,
    ) {
        seed_booking_for_room(
            pool,
            booking_id,
            "101",
            status,
            check_in_at,
            expected_checkout,
            total_price,
            paid_amount,
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    async fn seed_booking_for_room(
        pool: &Pool<Sqlite>,
        booking_id: &str,
        room_id: &str,
        status: &str,
        check_in_at: &str,
        expected_checkout: &str,
        total_price: MoneyVnd,
        paid_amount: MoneyVnd,
    ) {
        sqlx::query(
            "INSERT INTO bookings (
                id, room_id, primary_guest_id, check_in_at, expected_checkout,
                actual_checkout, nights, total_price, paid_amount, status,
                source, notes, scheduled_checkin, scheduled_checkout, created_at
            ) VALUES (?, ?, 'guest-1', ?, ?, NULL, 1, ?, ?, ?, 'walk-in', 'private note', ?, ?, '2026-05-06T08:00:00+07:00')",
        )
        .bind(booking_id)
        .bind(room_id)
        .bind(check_in_at)
        .bind(expected_checkout)
        .bind(total_price)
        .bind(paid_amount)
        .bind(status)
        .bind(check_in_at)
        .bind(expected_checkout)
        .execute(pool)
        .await
        .expect("seed booking");
    }

    #[tokio::test]
    async fn list_today_arrivals_reads_booked_arrivals_for_business_date() {
        let pool = test_pool().await;
        seed_room_and_guest(&pool).await;
        seed_booking(
            &pool,
            "booking-arrival",
            "booked",
            "2026-05-06",
            "2026-05-07",
            900_000,
            100_000,
        )
        .await;

        let arrivals = list_today_arrivals(&pool, "2026-05-06")
            .await
            .expect("arrivals");

        assert_eq!(arrivals.len(), 1);
        assert_eq!(arrivals[0].booking_id, "booking-arrival");
        assert_eq!(arrivals[0].balance_due, 800_000);
    }

    #[tokio::test]
    async fn list_today_arrivals_keeps_local_rfc3339_business_date() {
        let pool = test_pool().await;
        seed_room_and_guest(&pool).await;
        seed_booking(
            &pool,
            "booking-arrival-rfc3339",
            "booked",
            "2026-05-06T00:30:00+07:00",
            "2026-05-07",
            900_000,
            100_000,
        )
        .await;

        let arrivals = list_today_arrivals(&pool, "2026-05-06")
            .await
            .expect("arrivals");

        assert_eq!(arrivals.len(), 1);
        assert_eq!(arrivals[0].booking_id, "booking-arrival-rfc3339");
    }

    #[tokio::test]
    async fn list_today_checkouts_reads_active_bookings_due_for_departure() {
        let pool = test_pool().await;
        seed_room_and_guest(&pool).await;
        seed_booking(
            &pool,
            "booking-checkout",
            "active",
            "2026-05-05",
            "2026-05-06",
            900_000,
            900_000,
        )
        .await;

        let checkouts = list_today_checkouts(&pool, "2026-05-06")
            .await
            .expect("checkouts");

        assert_eq!(checkouts.len(), 1);
        assert_eq!(checkouts[0].booking_id, "booking-checkout");
        assert_eq!(checkouts[0].expected_checkout, "2026-05-06");
    }

    #[tokio::test]
    async fn list_today_checkouts_keeps_local_rfc3339_business_date() {
        let pool = test_pool().await;
        seed_room_and_guest(&pool).await;
        seed_booking(
            &pool,
            "booking-checkout-rfc3339",
            "active",
            "2026-05-05",
            "2026-05-06T00:30:00+07:00",
            900_000,
            900_000,
        )
        .await;

        let checkouts = list_today_checkouts(&pool, "2026-05-06")
            .await
            .expect("checkouts");

        assert_eq!(checkouts.len(), 1);
        assert_eq!(checkouts[0].booking_id, "booking-checkout-rfc3339");
    }

    #[tokio::test]
    async fn list_unpaid_balances_reads_open_rows_with_positive_balance() {
        let pool = test_pool().await;
        seed_room_and_guest(&pool).await;
        seed_booking(
            &pool,
            "booking-unpaid",
            "checked_out",
            "2026-05-05",
            "2026-05-06",
            900_000,
            100_000,
        )
        .await;
        seed_booking(
            &pool,
            "booking-paid",
            "active",
            "2026-05-05",
            "2026-05-06",
            900_000,
            900_000,
        )
        .await;

        let balances = list_unpaid_balances(&pool).await.expect("unpaid balances");

        assert_eq!(balances.len(), 1);
        assert_eq!(balances[0].booking_id, "booking-unpaid");
        assert_eq!(balances[0].balance_due, 800_000);
    }

    #[tokio::test]
    async fn list_unpaid_balances_orders_equal_balances_by_local_expected_checkout_date() {
        let pool = test_pool().await;
        seed_room_and_guest(&pool).await;
        sqlx::query(
            "INSERT INTO rooms (
                id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status
            ) VALUES ('102', 'Room 102', 'standard', 1, 0, 900000, 2, 0, 'vacant')",
        )
        .execute(&pool)
        .await
        .expect("seed second room");
        seed_booking_for_room(
            &pool,
            "booking-local-may6",
            "101",
            "active",
            "2026-05-05",
            "2026-05-06T00:30:00+07:00",
            900_000,
            100_000,
        )
        .await;
        seed_booking_for_room(
            &pool,
            "booking-local-may5",
            "102",
            "active",
            "2026-05-05",
            "2026-05-05T23:30:00+07:00",
            900_000,
            100_000,
        )
        .await;

        let balances = list_unpaid_balances(&pool).await.expect("unpaid balances");

        assert_eq!(balances.len(), 2);
        assert_eq!(balances[0].booking_id, "booking-local-may5");
        assert_eq!(balances[1].booking_id, "booking-local-may6");
    }

    #[tokio::test]
    async fn summarize_operational_risks_reports_unpaid_balances_and_checkouts_due_today() {
        let pool = test_pool().await;
        seed_room_and_guest(&pool).await;
        seed_booking(
            &pool,
            "booking-risk",
            "active",
            "2026-05-05",
            "2026-05-06",
            900_000,
            100_000,
        )
        .await;

        let risks = summarize_operational_risks(&pool, "2026-05-06")
            .await
            .expect("risks");

        assert!(risks
            .iter()
            .any(|risk| risk.code == "unpaid_balances" && risk.count == 1));
        assert!(risks
            .iter()
            .any(|risk| risk.code == "checkouts_due_today" && risk.count == 1));
    }

    #[tokio::test]
    async fn summarize_operational_risks_counts_local_rfc3339_checkout_date() {
        let pool = test_pool().await;
        seed_room_and_guest(&pool).await;
        seed_booking(
            &pool,
            "booking-risk-rfc3339",
            "active",
            "2026-05-05",
            "2026-05-06T00:30:00+07:00",
            900_000,
            900_000,
        )
        .await;

        let risks = summarize_operational_risks(&pool, "2026-05-06")
            .await
            .expect("risks");

        assert!(risks
            .iter()
            .any(|risk| risk.code == "checkouts_due_today" && risk.count == 1));
    }
}
