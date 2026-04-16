use super::{emit_db_update, get_f64, AppState};
use crate::models::*;
use crate::services::booking::reservation_lifecycle;
use sqlx::{Pool, Row, Sqlite};
use tauri::State;

// ═══════════════════════════════════════════════
// Reservation Calendar Block System
// ═══════════════════════════════════════════════

// ─── Check Availability ───

pub async fn do_check_availability(pool: &Pool<Sqlite>, room_id: &str, from_date: &str, to_date: &str) -> Result<AvailabilityResult, String> {
    let rows = sqlx::query(
        "SELECT rc.date, rc.status, rc.booking_id, COALESCE(g.full_name, '') as guest_name
         FROM room_calendar rc
         LEFT JOIN bookings b ON b.id = rc.booking_id
         LEFT JOIN guests g ON g.id = b.primary_guest_id
         WHERE rc.room_id = ? AND rc.date >= ? AND rc.date < ?
         ORDER BY rc.date ASC"
    )
    .bind(room_id).bind(from_date).bind(to_date)
    .fetch_all(pool).await.map_err(|e| e.to_string())?;

    if rows.is_empty() {
        return Ok(AvailabilityResult {
            available: true,
            conflicts: vec![],
            max_nights: None,
        });
    }

    let conflicts: Vec<CalendarConflict> = rows.iter().map(|r| CalendarConflict {
        date: r.get("date"),
        status: r.get("status"),
        guest_name: r.get("guest_name"),
        booking_id: r.get("booking_id"),
    }).collect();

    let first_date = &conflicts[0].date;
    let from_naive = chrono::NaiveDate::parse_from_str(from_date, "%Y-%m-%d")
        .map_err(|e| e.to_string())?;
    let first_naive = chrono::NaiveDate::parse_from_str(first_date, "%Y-%m-%d")
        .map_err(|e| e.to_string())?;
    let max_nights = (first_naive - from_naive).num_days() as i32;

    Ok(AvailabilityResult {
        available: false,
        conflicts,
        max_nights: Some(max_nights),
    })
}

#[tauri::command]
pub async fn check_availability(state: State<'_, AppState>, room_id: String, from_date: String, to_date: String) -> Result<AvailabilityResult, String> {
    do_check_availability(&state.db, &room_id, &from_date, &to_date).await
}

// ─── Create Reservation ───

pub async fn do_create_reservation(
    pool: &Pool<Sqlite>,
    app_handle: Option<&tauri::AppHandle>,
    req: CreateReservationRequest,
) -> Result<Booking, String> {
    let booking = reservation_lifecycle::create_reservation(pool, req)
        .await
        .map_err(|error| error.to_string())?;

    if let Some(app) = app_handle {
        emit_db_update(app, "rooms");
    }

    Ok(booking)
}

#[tauri::command]
pub async fn create_reservation(state: State<'_, AppState>, app: tauri::AppHandle, req: CreateReservationRequest) -> Result<Booking, String> {
    do_create_reservation(&state.db, Some(&app), req).await
}

// ─── Confirm Reservation (Check-in from reservation) ───

#[tauri::command]
pub async fn confirm_reservation(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    booking_id: String,
) -> Result<Booking, String> {
    let booking = reservation_lifecycle::confirm_reservation(&state.db, &booking_id)
        .await
        .map_err(|error| error.to_string())?;
    emit_db_update(&app, "rooms");

    Ok(booking)
}

// ─── Cancel Reservation ───

pub async fn do_cancel_reservation(
    pool: &Pool<Sqlite>,
    app_handle: Option<&tauri::AppHandle>,
    booking_id: &str,
) -> Result<(), String> {
    reservation_lifecycle::cancel_reservation(pool, booking_id)
        .await
        .map_err(|error| error.to_string())?;

    if let Some(app) = app_handle {
        emit_db_update(app, "rooms");
    }

    Ok(())
}

#[tauri::command]
pub async fn cancel_reservation(state: State<'_, AppState>, app: tauri::AppHandle, booking_id: String) -> Result<(), String> {
    do_cancel_reservation(&state.db, Some(&app), &booking_id).await
}

// ─── Modify Reservation ───

pub async fn do_modify_reservation(
    pool: &Pool<Sqlite>,
    app_handle: Option<&tauri::AppHandle>,
    req: ModifyReservationRequest,
) -> Result<Booking, String> {
    let booking = reservation_lifecycle::modify_reservation(pool, req)
        .await
        .map_err(|error| error.to_string())?;
    if let Some(app) = app_handle {
        emit_db_update(app, "rooms");
    }

    Ok(booking)
}

#[tauri::command]
pub async fn modify_reservation(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: ModifyReservationRequest,
) -> Result<Booking, String> {
    do_modify_reservation(&state.db, Some(&app), req).await
}

// ─── Get Room Calendar ───

#[tauri::command]
pub async fn get_room_calendar(state: State<'_, AppState>, room_id: String, from: String, to: String) -> Result<Vec<CalendarEntry>, String> {
    let rows = sqlx::query(
        "SELECT room_id, date, booking_id, status FROM room_calendar
         WHERE room_id = ? AND date >= ? AND date <= ?
         ORDER BY date ASC"
    )
    .bind(&room_id).bind(&from).bind(&to)
    .fetch_all(&state.db).await.map_err(|e| e.to_string())?;

    Ok(rows.iter().map(|r| CalendarEntry {
        room_id: r.get("room_id"),
        date: r.get("date"),
        booking_id: r.get("booking_id"),
        status: r.get("status"),
    }).collect())
}

// ─── Get Rooms Availability (Dashboard) ───

pub async fn do_get_rooms_availability(pool: &Pool<Sqlite>) -> Result<Vec<RoomWithAvailability>, String> {
    let room_rows = sqlx::query("SELECT id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status FROM rooms ORDER BY id")
        .fetch_all(pool).await.map_err(|e| e.to_string())?;

    if room_rows.is_empty() {
        return Ok(Vec::new());
    }

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut results = Vec::new();

    let room_ids: Vec<String> = room_rows.iter().map(|rr| rr.get("id")).collect();

    let mut current_bookings_map: std::collections::HashMap<String, Booking> = std::collections::HashMap::new();
    let mut upcoming_map: std::collections::HashMap<String, Vec<UpcomingReservation>> = std::collections::HashMap::new();

    let mut qb_current = sqlx::QueryBuilder::new("SELECT * FROM bookings WHERE status = 'active' AND room_id IN (");
    let mut separated = qb_current.separated(", ");
    for id in &room_ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");

    let current_rows = qb_current.build().fetch_all(pool).await.map_err(|e| e.to_string())?;
    for r in current_rows {
        let room_id: String = r.get("room_id");
        current_bookings_map.insert(room_id.clone(), Booking {
            id: r.get("id"),
            room_id,
            primary_guest_id: r.get("primary_guest_id"),
            check_in_at: r.get("check_in_at"),
            expected_checkout: r.get("expected_checkout"),
            actual_checkout: r.get("actual_checkout"),
            nights: r.get("nights"),
            total_price: get_f64(&r, "total_price"),
            paid_amount: get_f64(&r, "paid_amount"),
            status: r.get("status"),
            source: r.get("source"),
            notes: r.get("notes"),
            created_at: r.get("created_at"),
        });
    }

    let mut qb_upcoming = sqlx::QueryBuilder::new(
        "SELECT b.room_id, b.id, g.full_name, b.scheduled_checkin, b.scheduled_checkout, b.deposit_amount, b.status
         FROM bookings b
         JOIN guests g ON g.id = b.primary_guest_id
         WHERE b.status = 'booked' AND b.scheduled_checkin >= "
    );
    qb_upcoming.push_bind(&today);
    qb_upcoming.push(" AND b.room_id IN (");
    let mut separated = qb_upcoming.separated(", ");
    for id in &room_ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(") ORDER BY b.scheduled_checkin ASC");

    let upcoming_rows = qb_upcoming.build().fetch_all(pool).await.map_err(|e| e.to_string())?;
    for r in upcoming_rows {
        let room_id: String = r.get("room_id");
        let res = UpcomingReservation {
            booking_id: r.get("id"),
            guest_name: r.get("full_name"),
            scheduled_checkin: r.get::<Option<String>, _>("scheduled_checkin").unwrap_or_default(),
            scheduled_checkout: r.get::<Option<String>, _>("scheduled_checkout").unwrap_or_default(),
            deposit_amount: r.try_get::<f64, _>("deposit_amount").unwrap_or(0.0),
            status: r.get("status"),
        };
        upcoming_map.entry(room_id).or_default().push(res);
    }

    for rr in &room_rows {
        let room = Room {
            id: rr.get("id"),
            name: rr.get("name"),
            room_type: rr.get("type"),
            floor: rr.get("floor"),
            has_balcony: rr.get::<i32, _>("has_balcony") == 1,
            base_price: get_f64(rr, "base_price"),
            max_guests: rr.try_get::<i32, _>("max_guests").unwrap_or(2),
            extra_person_fee: rr.try_get::<f64, _>("extra_person_fee").unwrap_or(0.0),
            status: rr.get("status"),
        };

        let current_booking = current_bookings_map.remove(&room.id);
        let upcoming = upcoming_map.remove(&room.id).unwrap_or_default();
        let next_until = upcoming.first().map(|u| u.scheduled_checkin.clone());

        results.push(RoomWithAvailability {
            room,
            current_booking,
            upcoming_reservations: upcoming,
            next_available_until: next_until,
        });
    }

    Ok(results)
}

#[tauri::command]
pub async fn get_rooms_availability(state: State<'_, AppState>) -> Result<Vec<RoomWithAvailability>, String> {
    do_get_rooms_availability(&state.db).await
}
