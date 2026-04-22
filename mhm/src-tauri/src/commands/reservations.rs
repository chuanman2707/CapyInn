use super::{emit_db_update, get_f64, get_user, AppState};
use crate::{
    app_error::{
        codes, correlation_context, log_system_error, normalize_correlation_id,
        record_command_failure, CommandError, CommandResult, EffectiveCorrelationId,
    },
    domain::booking::BookingError,
    models::*,
};
use crate::services::booking::reservation_lifecycle;
use serde_json::{json, Value};
use sqlx::{Pool, Row, Sqlite};
use tauri::State;

// ═══════════════════════════════════════════════
// Reservation Calendar Block System
// ═══════════════════════════════════════════════

// ─── Check Availability ───

pub async fn do_check_availability(
    pool: &Pool<Sqlite>,
    room_id: &str,
    from_date: &str,
    to_date: &str,
) -> Result<AvailabilityResult, String> {
    let rows = sqlx::query(
        "SELECT rc.date, rc.status, rc.booking_id, COALESCE(g.full_name, '') as guest_name
         FROM room_calendar rc
         LEFT JOIN bookings b ON b.id = rc.booking_id
         LEFT JOIN guests g ON g.id = b.primary_guest_id
         WHERE rc.room_id = ? AND rc.date >= ? AND rc.date < ?
         ORDER BY rc.date ASC",
    )
    .bind(room_id)
    .bind(from_date)
    .bind(to_date)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    if rows.is_empty() {
        return Ok(AvailabilityResult {
            available: true,
            conflicts: vec![],
            max_nights: None,
        });
    }

    let conflicts: Vec<CalendarConflict> = rows
        .iter()
        .map(|r| CalendarConflict {
            date: r.get("date"),
            status: r.get("status"),
            guest_name: r.get("guest_name"),
            booking_id: r.get("booking_id"),
        })
        .collect();

    let first_date = &conflicts[0].date;
    let from_naive =
        chrono::NaiveDate::parse_from_str(from_date, "%Y-%m-%d").map_err(|e| e.to_string())?;
    let first_naive =
        chrono::NaiveDate::parse_from_str(first_date, "%Y-%m-%d").map_err(|e| e.to_string())?;
    let max_nights = (first_naive - from_naive).num_days() as i32;

    Ok(AvailabilityResult {
        available: false,
        conflicts,
        max_nights: Some(max_nights),
    })
}

#[tauri::command]
pub async fn check_availability(
    state: State<'_, AppState>,
    room_id: String,
    from_date: String,
    to_date: String,
) -> Result<AvailabilityResult, String> {
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

fn log_user_reservation_error(
    command_name: &str,
    effective_correlation_id: &EffectiveCorrelationId,
    message: &str,
    context: &Value,
) {
    let context_json = serde_json::to_string(&correlation_context(
        &effective_correlation_id.value,
        context.clone(),
    ))
    .unwrap_or_else(|_| "{}".to_string());

    log::warn!(
        "user error {} correlation_id={} source={:?}: {} | context={}",
        command_name,
        effective_correlation_id.value,
        effective_correlation_id.source,
        message,
        context_json
    );
}

fn map_create_reservation_user_error(
    code: &'static str,
    command_name: &str,
    effective_correlation_id: &EffectiveCorrelationId,
    message: impl Into<String>,
    context: &Value,
) -> CommandError {
    let message = message.into();
    log_user_reservation_error(
        command_name,
        effective_correlation_id,
        message.as_str(),
        context,
    );
    CommandError::user(code, message)
}

fn is_invalid_reservation_nights_error(message: &str) -> bool {
    message == "Number of nights must be greater than 0"
        || message == "Check-out date must be after check-in date"
        || message.starts_with("Number of nights must match the date range")
}

fn map_create_reservation_error(
    command_name: &str,
    effective_correlation_id: &EffectiveCorrelationId,
    error: BookingError,
    context: Value,
) -> CommandError {
    match error {
        BookingError::Validation(message) if is_invalid_reservation_nights_error(&message) => {
            map_create_reservation_user_error(
                codes::BOOKING_INVALID_NIGHTS,
                command_name,
                effective_correlation_id,
                "Số đêm phải lớn hơn 0",
                &context,
            )
        }
        BookingError::NotFound(message) if message.starts_with("Không tìm thấy phòng ") => {
            map_create_reservation_user_error(
                codes::ROOM_NOT_FOUND,
                command_name,
                effective_correlation_id,
                message,
                &context,
            )
        }
        BookingError::Validation(message) | BookingError::Conflict(message) => {
            map_create_reservation_user_error(
                codes::BOOKING_INVALID_STATE,
                command_name,
                effective_correlation_id,
                message,
                &context,
            )
        }
        BookingError::NotFound(message)
        | BookingError::Database(message)
        | BookingError::DateTimeParse(message) => log_system_error(
            command_name,
            message,
            correlation_context(&effective_correlation_id.value, context),
        ),
    }
}

fn reservation_failure_context(req: &CreateReservationRequest) -> Value {
    let notes_present = req
        .notes
        .as_ref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);

    json!({
        "room_id": req.room_id.clone(),
        "check_in_date": req.check_in_date.clone(),
        "check_out_date": req.check_out_date.clone(),
        "nights": req.nights,
        "deposit_present": req.deposit_amount.is_some(),
        "source": req.source.clone(),
        "notes_present": notes_present,
    })
}

#[tauri::command]
pub async fn create_reservation(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: CreateReservationRequest,
    correlation_id: Option<String>,
) -> CommandResult<Booking> {
    let effective_correlation_id = normalize_correlation_id(correlation_id);
    let error_context = reservation_failure_context(&req);

    log::info!(
        "create_reservation start correlation_id={} source={:?} room_id={} nights={}",
        effective_correlation_id.value,
        effective_correlation_id.source,
        req.room_id,
        req.nights
    );

    if get_user(&state).is_none() {
        let command_error =
            CommandError::user(codes::AUTH_NOT_AUTHENTICATED, "Chưa đăng nhập");
        record_command_failure(
            "create_reservation",
            &command_error,
            &effective_correlation_id.value,
            error_context,
        );
        return Err(command_error);
    }

    let booking = reservation_lifecycle::create_reservation(&state.db, req)
        .await
        .map_err(|error| {
            let command_error = map_create_reservation_error(
                "create_reservation",
                &effective_correlation_id,
                error,
                error_context.clone(),
            );
            record_command_failure(
                "create_reservation",
                &command_error,
                &effective_correlation_id.value,
                error_context.clone(),
            );
            command_error
        })?;

    log::info!(
        "create_reservation success correlation_id={} source={:?} booking_id={} room_id={}",
        effective_correlation_id.value,
        effective_correlation_id.source,
        booking.id,
        booking.room_id
    );

    emit_db_update(&app, "rooms");

    Ok(booking)
}

// ─── Confirm Reservation (Check-in from reservation) ───

#[tauri::command]
pub async fn confirm_reservation(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    booking_id: String,
) -> Result<Booking, String> {
    get_user(&state).ok_or_else(|| "Chưa đăng nhập".to_string())?;
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
pub async fn cancel_reservation(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    booking_id: String,
) -> Result<(), String> {
    get_user(&state).ok_or_else(|| "Chưa đăng nhập".to_string())?;
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
    get_user(&state).ok_or_else(|| "Chưa đăng nhập".to_string())?;
    do_modify_reservation(&state.db, Some(&app), req).await
}

// ─── Get Room Calendar ───

#[tauri::command]
pub async fn get_room_calendar(
    state: State<'_, AppState>,
    room_id: String,
    from: String,
    to: String,
) -> Result<Vec<CalendarEntry>, String> {
    let rows = sqlx::query(
        "SELECT room_id, date, booking_id, status FROM room_calendar
         WHERE room_id = ? AND date >= ? AND date <= ?
         ORDER BY date ASC",
    )
    .bind(&room_id)
    .bind(&from)
    .bind(&to)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| CalendarEntry {
            room_id: r.get("room_id"),
            date: r.get("date"),
            booking_id: r.get("booking_id"),
            status: r.get("status"),
        })
        .collect())
}

// ─── Get Rooms Availability (Dashboard) ───

pub async fn do_get_rooms_availability(
    pool: &Pool<Sqlite>,
) -> Result<Vec<RoomWithAvailability>, String> {
    let room_rows = sqlx::query("SELECT id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status FROM rooms ORDER BY id")
        .fetch_all(pool).await.map_err(|e| e.to_string())?;

    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let mut results = Vec::new();

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

        let current_booking =
            sqlx::query("SELECT * FROM bookings WHERE room_id = ? AND status = 'active' LIMIT 1")
                .bind(&room.id)
                .fetch_optional(pool)
                .await
                .map_err(|e| e.to_string())?
                .map(|r| Booking {
                    id: r.get("id"),
                    room_id: r.get("room_id"),
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

        let res_rows = sqlx::query(
            "SELECT b.id, g.full_name, b.scheduled_checkin, b.scheduled_checkout, b.deposit_amount, b.status
             FROM bookings b
             JOIN guests g ON g.id = b.primary_guest_id
             WHERE b.room_id = ? AND b.status = 'booked' AND b.scheduled_checkin >= ?
             ORDER BY b.scheduled_checkin ASC"
        )
        .bind(&room.id).bind(&today)
        .fetch_all(pool).await.map_err(|e| e.to_string())?;

        let upcoming: Vec<UpcomingReservation> = res_rows
            .iter()
            .map(|r| UpcomingReservation {
                booking_id: r.get("id"),
                guest_name: r.get("full_name"),
                scheduled_checkin: r
                    .get::<Option<String>, _>("scheduled_checkin")
                    .unwrap_or_default(),
                scheduled_checkout: r
                    .get::<Option<String>, _>("scheduled_checkout")
                    .unwrap_or_default(),
                deposit_amount: r.try_get::<f64, _>("deposit_amount").unwrap_or(0.0),
                status: r.get("status"),
            })
            .collect();

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
pub async fn get_rooms_availability(
    state: State<'_, AppState>,
) -> Result<Vec<RoomWithAvailability>, String> {
    do_get_rooms_availability(&state.db).await
}

#[cfg(test)]
mod tests {
    use super::{map_create_reservation_error, reservation_failure_context};
    use crate::app_error::{codes, AppErrorKind, CorrelationIdSource, EffectiveCorrelationId};
    use crate::domain::booking::BookingError;
    use crate::models::CreateReservationRequest;
    use serde_json::json;

    fn frontend_correlation_id() -> EffectiveCorrelationId {
        EffectiveCorrelationId {
            value: "COR-1A2B3C4D".to_string(),
            source: CorrelationIdSource::Frontend,
            rejected_length: None,
        }
    }

    #[test]
    fn map_create_reservation_error_maps_invalid_nights_to_shared_code() {
        let error = map_create_reservation_error(
            "create_reservation",
            &frontend_correlation_id(),
            BookingError::validation(
                "Number of nights must match the date range (expected 2)".to_string(),
            ),
            json!({
                "room_id": "R101",
                "check_in_date": "2026-04-25",
                "check_out_date": "2026-04-27",
                "nights": 1,
            }),
        );

        assert_eq!(error.code, codes::BOOKING_INVALID_NIGHTS);
        assert_eq!(error.message, "Số đêm phải lớn hơn 0");
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
    }

    #[test]
    fn reservation_failure_context_keeps_only_scrubbed_flags_and_dates() {
        let context = reservation_failure_context(&CreateReservationRequest {
            room_id: "R101".to_string(),
            guest_name: "Nguyen Van A".to_string(),
            guest_phone: Some("0901234567".to_string()),
            guest_doc_number: Some("079123456789".to_string()),
            check_in_date: "2026-04-25".to_string(),
            check_out_date: "2026-04-27".to_string(),
            nights: 2,
            deposit_amount: Some(500000.0),
            source: Some("zalo".to_string()),
            notes: Some("Khách thích tầng cao".to_string()),
        });

        assert_eq!(
            context,
            json!({
                "room_id": "R101",
                "check_in_date": "2026-04-25",
                "check_out_date": "2026-04-27",
                "nights": 2,
                "deposit_present": true,
                "source": "zalo",
                "notes_present": true,
            })
        );
        assert!(context.get("guest_name").is_none());
        assert!(context.get("guest_phone").is_none());
        assert!(context.get("guest_doc_number").is_none());
        assert!(context.get("notes").is_none());
    }
}
