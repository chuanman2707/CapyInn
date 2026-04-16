use super::{get_f64, AppState};
use crate::models::*;
use sqlx::{Pool, Row, Sqlite};
use tauri::State;

// ─── A1: Get All Bookings (Reservations) ───

pub async fn do_get_all_bookings(
    pool: &Pool<Sqlite>,
    filter: Option<BookingFilter>,
) -> Result<Vec<BookingWithGuest>, String> {
    let mut sql = String::from(
        "SELECT b.id, b.room_id, r.name as room_name, g.full_name as guest_name,
                b.check_in_at, b.expected_checkout, b.actual_checkout,
                b.nights, b.total_price, b.paid_amount, b.status, b.source,
                b.booking_type, b.deposit_amount, b.scheduled_checkin, b.scheduled_checkout, b.guest_phone
         FROM bookings b
         JOIN rooms r ON r.id = b.room_id
         JOIN guests g ON g.id = b.primary_guest_id
         WHERE 1=1"
    );

    let mut binds: Vec<String> = vec![];

    if let Some(ref f) = filter {
        if let Some(ref status) = f.status {
            match status.as_str() {
                "active" => sql.push_str(" AND b.status = 'active'"),
                "completed" => sql.push_str(" AND b.status = 'checked_out'"),
                "booked" => sql.push_str(" AND b.status = 'booked'"),
                _ => {}
            }
        }
        if let Some(ref from) = f.from {
            sql.push_str(" AND b.check_in_at >= ?");
            binds.push(from.clone());
        }
        if let Some(ref to) = f.to {
            sql.push_str(" AND b.expected_checkout <= ?");
            binds.push(to.clone());
        }
    }

    sql.push_str(" ORDER BY b.check_in_at DESC");

    let mut query = sqlx::query(&sql);
    for b in &binds {
        query = query.bind(b);
    }

    let rows = query.fetch_all(pool).await.map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| BookingWithGuest {
            id: r.get("id"),
            room_id: r.get("room_id"),
            room_name: r.get("room_name"),
            guest_name: r.get("guest_name"),
            check_in_at: r.get("check_in_at"),
            expected_checkout: r.get("expected_checkout"),
            actual_checkout: r.get("actual_checkout"),
            nights: r.get("nights"),
            total_price: get_f64(r, "total_price"),
            paid_amount: get_f64(r, "paid_amount"),
            status: r.get("status"),
            source: r.get("source"),
            booking_type: r.get("booking_type"),
            deposit_amount: r.try_get::<f64, _>("deposit_amount").ok(),
            scheduled_checkin: r.get("scheduled_checkin"),
            scheduled_checkout: r.get("scheduled_checkout"),
            guest_phone: r.get("guest_phone"),
        })
        .collect())
}

#[tauri::command]
pub async fn get_all_bookings(
    state: State<'_, AppState>,
    filter: Option<BookingFilter>,
) -> Result<Vec<BookingWithGuest>, String> {
    do_get_all_bookings(&state.db, filter).await
}
