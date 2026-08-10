//! Reads behind the admin CSV export.
//!
//! The rows are shaped for the export and nothing else, so they get their own
//! structs rather than reusing `models::Booking` — the export joins the guest
//! name in and drops most booking fields.

use sqlx::{Pool, Row, Sqlite};

use crate::db::row::get_money_vnd;
use crate::money::MoneyVnd;

pub struct BookingExportRow {
    pub id: String,
    pub room_id: String,
    pub guest_name: String,
    pub check_in_at: String,
    pub expected_checkout: String,
    pub nights: i32,
    pub total_price: MoneyVnd,
    pub paid_amount: MoneyVnd,
    pub status: String,
    pub source: String,
}

pub struct GuestExportRow {
    pub id: String,
    pub full_name: String,
    pub doc_number: String,
    pub nationality: String,
    pub created_at: String,
}

pub async fn load_booking_export_rows(
    pool: &Pool<Sqlite>,
) -> Result<Vec<BookingExportRow>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT b.id, b.room_id, g.full_name, b.check_in_at, b.expected_checkout, b.nights,
                b.total_price, b.paid_amount, b.status, b.source
         FROM bookings b
         JOIN guests g ON g.id = b.primary_guest_id
         WHERE b.status != 'voided'
         ORDER BY b.check_in_at DESC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| BookingExportRow {
            id: row.get("id"),
            room_id: row.get("room_id"),
            guest_name: row.get("full_name"),
            check_in_at: row.get("check_in_at"),
            expected_checkout: row.get("expected_checkout"),
            nights: row.get("nights"),
            total_price: get_money_vnd(row, "total_price"),
            paid_amount: get_money_vnd(row, "paid_amount"),
            status: row.get("status"),
            source: row.get::<Option<String>, _>("source").unwrap_or_default(),
        })
        .collect())
}

pub async fn load_guest_export_rows(
    pool: &Pool<Sqlite>,
) -> Result<Vec<GuestExportRow>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT id, full_name, doc_number, nationality, created_at
         FROM guests
         ORDER BY created_at DESC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| GuestExportRow {
            id: row.get("id"),
            full_name: row.get("full_name"),
            doc_number: row.get("doc_number"),
            nationality: row
                .get::<Option<String>, _>("nationality")
                .unwrap_or_default(),
            created_at: row.get("created_at"),
        })
        .collect())
}
