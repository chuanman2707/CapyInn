use sqlx::{Pool, Row, Sqlite};

use crate::{
    commands::{get_f64, get_money_vnd},
    models::{AuditLog, BookingExportRow, NightAuditSnapshot},
};

use super::revenue_queries;

pub async fn load_night_audit_snapshot(
    pool: &Pool<Sqlite>,
    audit_date: &str,
) -> Result<NightAuditSnapshot, sqlx::Error> {
    let room_revenue = revenue_queries::load_room_revenue(pool, audit_date, audit_date).await?;
    let folio_revenue = revenue_queries::load_folio_revenue(pool, audit_date, audit_date).await?;
    let cancellation_fee_revenue =
        revenue_queries::load_cancellation_fee_revenue(pool, audit_date, audit_date).await?;

    let expenses = sqlx::query(
        "SELECT COALESCE(SUM(amount), 0) AS value
         FROM expenses
         WHERE expense_date = ?",
    )
    .bind(audit_date)
    .fetch_one(pool)
    .await?;

    let total_rooms: (i32,) = sqlx::query_as("SELECT COUNT(*) FROM rooms")
        .fetch_one(pool)
        .await?;

    let occupancy_checkout = revenue_queries::occupancy_checkout_date_sql("");
    let rooms_sold_query = format!(
        "SELECT COUNT(DISTINCT room_id)
         FROM bookings
         WHERE status IN ('active', 'checked_out')
           AND DATE(check_in_at) < DATE(?1, '+1 day')
           AND DATE({occupancy_checkout}) > DATE(?2)"
    );
    let rooms_sold: (i32,) = sqlx::query_as(&rooms_sold_query)
        .bind(audit_date)
        .bind(audit_date)
        .fetch_one(pool)
        .await?;

    let occupancy_pct = if total_rooms.0 > 0 {
        (rooms_sold.0 as f64 / total_rooms.0 as f64 * 100.0).round()
    } else {
        0.0
    };

    Ok(NightAuditSnapshot {
        audit_date: audit_date.to_string(),
        total_revenue: room_revenue + folio_revenue + cancellation_fee_revenue,
        room_revenue,
        folio_revenue,
        total_expenses: get_money_vnd(&expenses, "value"),
        occupancy_pct,
        rooms_sold: rooms_sold.0,
        total_rooms: total_rooms.0,
    })
}

pub async fn list_audit_logs(pool: &Pool<Sqlite>) -> Result<Vec<AuditLog>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, audit_date, total_revenue, room_revenue, folio_revenue,
                total_expenses, occupancy_pct, rooms_sold, total_rooms, notes, created_at
         FROM night_audit_logs
         ORDER BY audit_date DESC
         LIMIT 30",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| AuditLog {
            id: row.get("id"),
            audit_date: row.get("audit_date"),
            total_revenue: get_money_vnd(row, "total_revenue"),
            room_revenue: get_money_vnd(row, "room_revenue"),
            folio_revenue: get_money_vnd(row, "folio_revenue"),
            total_expenses: get_money_vnd(row, "total_expenses"),
            occupancy_pct: get_f64(row, "occupancy_pct"),
            rooms_sold: row.get("rooms_sold"),
            total_rooms: row.get("total_rooms"),
            notes: row.get("notes"),
            created_at: row.get("created_at"),
        })
        .collect())
}

pub async fn load_booking_export_rows(
    pool: &Pool<Sqlite>,
    from_date: &str,
    to_date: &str,
) -> Result<Vec<BookingExportRow>, sqlx::Error> {
    let reporting_checkout = revenue_queries::recognized_checkout_date_sql("b.");
    let export_checkout = format!(
        "CASE
            WHEN b.status = 'checked_out' THEN {reporting_checkout}
            ELSE b.expected_checkout
        END"
    );
    let rows = sqlx::query(&format!(
        "SELECT b.id, b.room_id,
                COALESCE(g.full_name, '') AS guest_name,
                COALESCE(g.doc_number, '') AS doc_number,
                COALESCE(g.phone, '') AS phone,
                b.check_in_at, {export_checkout} AS expected_checkout, COALESCE(b.actual_checkout, '') AS actual_checkout,
                b.nights, b.total_price,
                COALESCE(charges.charge_total, 0) AS charge_total,
                COALESCE(fees.cancellation_fee_total, 0) AS cancellation_fee_total,
                COALESCE(folio.folio_total, 0) AS folio_total,
                COALESCE(charges.charge_total, 0)
                    + COALESCE(fees.cancellation_fee_total, 0)
                    + COALESCE(folio.folio_total, 0) AS recognized_revenue,
                COALESCE(b.paid_amount, 0) AS paid_amount,
                b.status, COALESCE(b.pricing_type, '') AS pricing_type, COALESCE(b.source, '') AS source
         FROM bookings b
         LEFT JOIN guests g ON b.primary_guest_id = g.id
         LEFT JOIN (
             SELECT booking_id, COALESCE(SUM(amount), 0) AS charge_total
             FROM transactions
             WHERE type = 'charge'
             GROUP BY booking_id
         ) charges ON charges.booking_id = b.id
         LEFT JOIN (
             SELECT booking_id, COALESCE(SUM(amount), 0) AS cancellation_fee_total
             FROM transactions
             WHERE type = 'cancellation_fee'
             GROUP BY booking_id
         ) fees ON fees.booking_id = b.id
         LEFT JOIN (
             SELECT booking_id, COALESCE(SUM(amount), 0) AS folio_total
             FROM folio_lines
             GROUP BY booking_id
         ) folio ON folio.booking_id = b.id
         WHERE (
                b.status = 'checked_out'
                AND DATE({reporting_checkout}) BETWEEN DATE(?1) AND DATE(?2)
            )
            OR (
                b.status != 'checked_out'
                AND DATE(b.check_in_at) BETWEEN DATE(?1) AND DATE(?2)
            )
            OR EXISTS (
                SELECT 1
                FROM transactions tx
                WHERE tx.booking_id = b.id
                  AND tx.type = 'cancellation_fee'
                  AND DATE(tx.created_at) BETWEEN DATE(?1) AND DATE(?2)
            )
         ORDER BY b.check_in_at DESC")
    )
    .bind(from_date)
    .bind(to_date)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| BookingExportRow {
            id: row.get("id"),
            room_id: row.get("room_id"),
            guest_name: row.get("guest_name"),
            doc_number: row.get("doc_number"),
            phone: row.get("phone"),
            check_in_at: row.get("check_in_at"),
            expected_checkout: row.get("expected_checkout"),
            actual_checkout: row.get("actual_checkout"),
            nights: row.get("nights"),
            room_price: get_money_vnd(row, "total_price"),
            charge_total: get_money_vnd(row, "charge_total"),
            cancellation_fee_total: get_money_vnd(row, "cancellation_fee_total"),
            folio_total: get_money_vnd(row, "folio_total"),
            recognized_revenue: get_money_vnd(row, "recognized_revenue"),
            paid_amount: get_money_vnd(row, "paid_amount"),
            status: row.get("status"),
            pricing_type: row.get("pricing_type"),
            source: row.get("source"),
        })
        .collect())
}
