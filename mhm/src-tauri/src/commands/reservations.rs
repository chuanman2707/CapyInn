use super::{emit_db_update, get_f64, get_user, AppState};
use crate::services::booking::reservation_lifecycle;
use crate::{
    app_error::{
        codes, correlation_context, log_system_error, normalize_correlation_id,
        record_command_failure, record_command_failure_with_db_group, CommandError, CommandResult,
        EffectiveCorrelationId,
    },
    command_idempotency::WriteCommandContext,
    db_error_monitoring::{
        classify_db_error_code, classify_db_failure, inject_db_error_group,
        is_room_unavailable_conflict_message, DbErrorGroup, MonitoredDbFailure,
    },
    domain::booking::{BookingError, BookingResult},
    models::*,
};
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
) -> BookingResult<Booking> {
    let booking = reservation_lifecycle::create_reservation(pool, req).await?;

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

fn map_known_reservation_error_code(
    command_name: &str,
    effective_correlation_id: &EffectiveCorrelationId,
    message: &str,
    context: &Value,
) -> Option<(CommandError, Option<DbErrorGroup>)> {
    if is_room_unavailable_conflict_message(message) {
        return Some((
            map_create_reservation_user_error(
                codes::CONFLICT_ROOM_UNAVAILABLE,
                command_name,
                effective_correlation_id,
                message,
                context,
            ),
            Some(DbErrorGroup::Constraint),
        ));
    }

    if message.contains(codes::CONFLICT_INVALID_STATE_TRANSITION) {
        return Some((
            map_create_reservation_user_error(
                codes::CONFLICT_INVALID_STATE_TRANSITION,
                command_name,
                effective_correlation_id,
                message,
                context,
            ),
            Some(DbErrorGroup::Constraint),
        ));
    }

    match classify_db_error_code(message) {
        Some(codes::DB_LOCKED_RETRYABLE) => Some((
            CommandError::system(codes::DB_LOCKED_RETRYABLE, message.to_string()).retryable(true),
            Some(DbErrorGroup::Locked),
        )),
        _ => None,
    }
}

fn map_reservation_write_error(
    command_name: &str,
    effective_correlation_id: &EffectiveCorrelationId,
    error: BookingError,
    context: Value,
) -> (CommandError, Option<DbErrorGroup>) {
    match error {
        BookingError::Validation(message)
            if message == "Number of nights must be greater than 0" =>
        {
            (
                map_create_reservation_user_error(
                    codes::BOOKING_INVALID_NIGHTS,
                    command_name,
                    effective_correlation_id,
                    "Số đêm phải lớn hơn 0",
                    &context,
                ),
                None,
            )
        }
        BookingError::NotFound(message) if message.starts_with("Không tìm thấy phòng ") => (
            map_create_reservation_user_error(
                codes::ROOM_NOT_FOUND,
                command_name,
                effective_correlation_id,
                message,
                &context,
            ),
            Some(DbErrorGroup::NotFound),
        ),
        BookingError::NotFound(message)
            if message.starts_with("Booking not found: ")
                || message.starts_with("Không tìm thấy booking ") =>
        {
            (
                map_create_reservation_user_error(
                    codes::BOOKING_NOT_FOUND,
                    command_name,
                    effective_correlation_id,
                    message,
                    &context,
                ),
                Some(DbErrorGroup::NotFound),
            )
        }
        BookingError::Validation(message) | BookingError::Conflict(message) => {
            if let Some(mapped) = map_known_reservation_error_code(
                command_name,
                effective_correlation_id,
                &message,
                &context,
            ) {
                return mapped;
            }
            (
                map_create_reservation_user_error(
                    codes::BOOKING_INVALID_STATE,
                    command_name,
                    effective_correlation_id,
                    message,
                    &context,
                ),
                None,
            )
        }
        BookingError::DatabaseWrite(message) => {
            let db_error_group = classify_db_failure(MonitoredDbFailure::DatabaseWrite(&message));
            if let Some(mapped) = map_known_reservation_error_code(
                command_name,
                effective_correlation_id,
                &message,
                &context,
            ) {
                return mapped;
            }
            (
                log_system_error(
                    command_name,
                    &message,
                    inject_db_error_group(
                        correlation_context(&effective_correlation_id.value, context),
                        db_error_group,
                    ),
                ),
                Some(db_error_group),
            )
        }
        BookingError::Database(message) => {
            let db_error_group = classify_db_failure(MonitoredDbFailure::DatabaseRead(&message));
            if let Some(mapped) = map_known_reservation_error_code(
                command_name,
                effective_correlation_id,
                &message,
                &context,
            ) {
                return mapped;
            }
            (
                log_system_error(
                    command_name,
                    &message,
                    inject_db_error_group(
                        correlation_context(&effective_correlation_id.value, context),
                        db_error_group,
                    ),
                ),
                Some(db_error_group),
            )
        }
        BookingError::DateTimeParse(message) => (
            log_system_error(
                command_name,
                message,
                correlation_context(&effective_correlation_id.value, context),
            ),
            None,
        ),
        BookingError::NotFound(message) => (
            log_system_error(
                command_name,
                message,
                correlation_context(&effective_correlation_id.value, context),
            ),
            None,
        ),
    }
}

#[allow(dead_code)]
fn map_create_reservation_error(
    command_name: &str,
    effective_correlation_id: &EffectiveCorrelationId,
    error: BookingError,
    context: Value,
) -> (CommandError, Option<DbErrorGroup>) {
    map_reservation_write_error(command_name, effective_correlation_id, error, context)
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

fn reservation_booking_failure_context(booking_id: &str) -> Value {
    json!({
        "booking_id": booking_id,
    })
}

fn modify_reservation_failure_context(req: &ModifyReservationRequest) -> Value {
    json!({
        "booking_id": req.booking_id.clone(),
        "check_in_date": req.new_check_in_date.clone(),
        "check_out_date": req.new_check_out_date.clone(),
        "nights": req.new_nights,
    })
}

fn unauthenticated_create_reservation_error(
    effective_correlation_id: &EffectiveCorrelationId,
    context: Value,
) -> CommandError {
    let command_error = CommandError::user(codes::AUTH_NOT_AUTHENTICATED, "Chưa đăng nhập");
    record_command_failure(
        "create_reservation",
        &command_error,
        &effective_correlation_id.value,
        context,
    );
    command_error
}

fn map_create_reservation_command_error_db_group(
    command_error: &CommandError,
) -> Option<DbErrorGroup> {
    match command_error.code.as_str() {
        codes::DB_LOCKED_RETRYABLE => Some(DbErrorGroup::Locked),
        codes::CONFLICT_ROOM_UNAVAILABLE => Some(DbErrorGroup::Constraint),
        codes::ROOM_NOT_FOUND | codes::BOOKING_NOT_FOUND => Some(DbErrorGroup::NotFound),
        _ => None,
    }
}

fn record_create_reservation_command_failure(
    effective_correlation_id: &EffectiveCorrelationId,
    command_error: &CommandError,
    context: Value,
) {
    record_command_failure_with_db_group(
        "create_reservation",
        command_error,
        &effective_correlation_id.value,
        map_create_reservation_command_error_db_group(command_error),
        context,
    );
}

#[tauri::command]
pub async fn create_reservation(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: CreateReservationRequest,
    idempotency_key: String,
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
        return Err(unauthenticated_create_reservation_error(
            &effective_correlation_id,
            error_context,
        ));
    }

    let write_command_context = WriteCommandContext::for_scoped_command(
        effective_correlation_id.value.clone(),
        idempotency_key,
        "create_reservation",
    )
    .inspect_err(|command_error| {
        record_create_reservation_command_failure(
            &effective_correlation_id,
            command_error,
            error_context.clone(),
        );
    })?;

    let create_result = reservation_lifecycle::create_reservation_idempotent(
        &state.db,
        &write_command_context,
        req,
    )
    .await
    .inspect_err(|command_error| {
        record_create_reservation_command_failure(
            &effective_correlation_id,
            command_error,
            error_context.clone(),
        );
    })?;

    let booking: Booking = serde_json::from_value(create_result.response).map_err(|error| {
        let command_error = CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            format!("Invalid create_reservation idempotent response: {error}"),
        )
        .with_request_id(effective_correlation_id.value.clone());
        record_create_reservation_command_failure(
            &effective_correlation_id,
            &command_error,
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
) -> BookingResult<()> {
    reservation_lifecycle::cancel_reservation(pool, booking_id).await?;

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
    correlation_id: Option<String>,
) -> CommandResult<()> {
    let effective_correlation_id = normalize_correlation_id(correlation_id);
    let error_context = reservation_booking_failure_context(&booking_id);

    if get_user(&state).is_none() {
        let command_error = CommandError::user(codes::AUTH_NOT_AUTHENTICATED, "Chưa đăng nhập");
        record_command_failure(
            "cancel_reservation",
            &command_error,
            &effective_correlation_id.value,
            error_context,
        );
        return Err(command_error);
    }

    do_cancel_reservation(&state.db, Some(&app), &booking_id)
        .await
        .map_err(|error| {
            let (command_error, db_error_group) = map_reservation_write_error(
                "cancel_reservation",
                &effective_correlation_id,
                error,
                error_context.clone(),
            );
            record_command_failure_with_db_group(
                "cancel_reservation",
                &command_error,
                &effective_correlation_id.value,
                db_error_group,
                error_context.clone(),
            );
            command_error
        })
}

// ─── Modify Reservation ───

pub async fn do_modify_reservation(
    pool: &Pool<Sqlite>,
    app_handle: Option<&tauri::AppHandle>,
    req: ModifyReservationRequest,
) -> BookingResult<Booking> {
    let booking = reservation_lifecycle::modify_reservation(pool, req).await?;
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
    correlation_id: Option<String>,
) -> CommandResult<Booking> {
    let effective_correlation_id = normalize_correlation_id(correlation_id);
    let error_context = modify_reservation_failure_context(&req);

    if get_user(&state).is_none() {
        let command_error = CommandError::user(codes::AUTH_NOT_AUTHENTICATED, "Chưa đăng nhập");
        record_command_failure(
            "modify_reservation",
            &command_error,
            &effective_correlation_id.value,
            error_context,
        );
        return Err(command_error);
    }

    do_modify_reservation(&state.db, Some(&app), req)
        .await
        .map_err(|error| {
            let (command_error, db_error_group) = map_reservation_write_error(
                "modify_reservation",
                &effective_correlation_id,
                error,
                error_context.clone(),
            );
            record_command_failure_with_db_group(
                "modify_reservation",
                &command_error,
                &effective_correlation_id.value,
                db_error_group,
                error_context.clone(),
            );
            command_error
        })
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
    use super::{
        map_create_reservation_command_error_db_group, map_create_reservation_error,
        reservation_failure_context, unauthenticated_create_reservation_error,
    };
    use crate::app_error::{
        codes, record_command_failure_with_db_group, AppErrorKind, CommandError,
        CorrelationIdSource, EffectiveCorrelationId,
    };
    use crate::db_error_monitoring::DbErrorGroup;
    use crate::domain::booking::BookingError;
    use crate::models::CreateReservationRequest;
    use serde_json::json;
    use std::fs;

    fn reservation_request() -> CreateReservationRequest {
        CreateReservationRequest {
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
        }
    }

    fn frontend_correlation_id() -> EffectiveCorrelationId {
        EffectiveCorrelationId {
            value: "COR-1A2B3C4D".to_string(),
            source: CorrelationIdSource::Frontend,
            rejected_length: None,
        }
    }

    fn parse_json_lines(contents: &str) -> Vec<serde_json::Value> {
        contents
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_str(line).expect("json line"))
            .collect()
    }

    fn restore_runtime_root(previous: Option<std::ffi::OsString>) {
        match previous {
            Some(value) => std::env::set_var("CAPYINN_RUNTIME_ROOT", value),
            None => std::env::remove_var("CAPYINN_RUNTIME_ROOT"),
        }
    }

    #[test]
    fn map_create_reservation_error_maps_invalid_nights_to_shared_code() {
        let (error, db_error_group) = map_create_reservation_error(
            "create_reservation",
            &frontend_correlation_id(),
            BookingError::validation("Number of nights must be greater than 0".to_string()),
            json!({
                "room_id": "R101",
                "check_in_date": "2026-04-25",
                "check_out_date": "2026-04-25",
                "nights": 0,
            }),
        );

        assert_eq!(error.code, codes::BOOKING_INVALID_NIGHTS);
        assert_eq!(error.message, "Số đêm phải lớn hơn 0");
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
        assert!(db_error_group.is_none());
    }

    #[test]
    fn map_create_reservation_error_keeps_missing_room_under_existing_contract_and_not_found_group()
    {
        let (error, db_error_group) = map_create_reservation_error(
            "create_reservation",
            &frontend_correlation_id(),
            BookingError::not_found("Không tìm thấy phòng R101"),
            json!({
                "room_id": "R101",
                "check_in_date": "2026-04-25",
                "check_out_date": "2026-04-27",
                "nights": 2,
            }),
        );

        assert_eq!(error.code, codes::ROOM_NOT_FOUND);
        assert_eq!(error.message, "Không tìm thấy phòng R101");
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
        assert_eq!(db_error_group, Some(DbErrorGroup::NotFound));
    }

    #[test]
    fn map_create_reservation_error_keeps_date_range_mismatch_feedback_under_invalid_state() {
        let (error, db_error_group) = map_create_reservation_error(
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

        assert_eq!(error.code, codes::BOOKING_INVALID_STATE);
        assert_eq!(
            error.message,
            "Number of nights must match the date range (expected 2)"
        );
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
        assert!(db_error_group.is_none());
    }

    #[test]
    fn map_create_reservation_error_maps_legacy_calendar_conflict_to_stable_code() {
        let (error, db_error_group) = map_create_reservation_error(
            "create_reservation",
            &frontend_correlation_id(),
            BookingError::conflict("Room R101 is booked on 2026-04-20. Cannot create reservation."),
            json!({
                "room_id": "R101",
                "check_in_date": "2026-04-20",
                "check_out_date": "2026-04-21",
                "nights": 1,
            }),
        );

        assert_eq!(error.code, codes::CONFLICT_ROOM_UNAVAILABLE);
        assert!(error.message.contains("is booked on"));
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
        assert_eq!(db_error_group, Some(DbErrorGroup::Constraint));
    }

    #[test]
    fn map_create_reservation_error_maps_invalid_state_transition_to_shared_conflict_code() {
        let (error, db_error_group) = map_create_reservation_error(
            "modify_reservation",
            &frontend_correlation_id(),
            BookingError::conflict(
                "CONFLICT_INVALID_STATE_TRANSITION: booking is no longer active",
            ),
            json!({ "booking_id": "B101" }),
        );

        assert_eq!(error.code, codes::CONFLICT_INVALID_STATE_TRANSITION);
        assert!(error.message.contains("CONFLICT_INVALID_STATE_TRANSITION"));
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
        assert_eq!(db_error_group, Some(DbErrorGroup::Constraint));
    }

    #[test]
    fn map_create_reservation_error_maps_booking_not_found_to_shared_code() {
        let (error, db_error_group) = map_create_reservation_error(
            "modify_reservation",
            &frontend_correlation_id(),
            BookingError::not_found("Booking not found: B101"),
            json!({ "booking_id": "B101" }),
        );

        assert_eq!(error.code, codes::BOOKING_NOT_FOUND);
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
        assert_eq!(db_error_group, Some(DbErrorGroup::NotFound));
    }

    #[test]
    fn map_create_reservation_error_maps_vietnamese_booking_not_found_to_shared_code() {
        let (error, db_error_group) = map_create_reservation_error(
            "modify_reservation",
            &frontend_correlation_id(),
            BookingError::not_found("Không tìm thấy booking B101"),
            json!({ "booking_id": "B101" }),
        );

        assert_eq!(error.code, codes::BOOKING_NOT_FOUND);
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
        assert_eq!(db_error_group, Some(DbErrorGroup::NotFound));
    }

    #[test]
    fn map_create_reservation_error_maps_locked_writes_to_retryable_code() {
        let (error, db_error_group) = map_create_reservation_error(
            "create_reservation",
            &frontend_correlation_id(),
            BookingError::database_write("database is locked"),
            json!({
                "room_id": "R101",
                "check_in_date": "2026-04-20",
                "check_out_date": "2026-04-21",
                "nights": 1,
            }),
        );

        assert_eq!(error.code, codes::DB_LOCKED_RETRYABLE);
        assert_eq!(error.kind, AppErrorKind::System);
        assert!(error.support_id.is_some());
        assert!(error.retryable);
        assert_eq!(db_error_group, Some(DbErrorGroup::Locked));
    }

    #[test]
    fn map_create_reservation_command_error_db_group_maps_expected_codes() {
        let mk = |code: &'static str| CommandError::user(code, "x");

        assert_eq!(
            map_create_reservation_command_error_db_group(&mk(codes::DB_LOCKED_RETRYABLE)),
            Some(DbErrorGroup::Locked)
        );
        assert_eq!(
            map_create_reservation_command_error_db_group(&mk(codes::CONFLICT_ROOM_UNAVAILABLE)),
            Some(DbErrorGroup::Constraint)
        );
        assert_eq!(
            map_create_reservation_command_error_db_group(&mk(codes::ROOM_NOT_FOUND)),
            Some(DbErrorGroup::NotFound)
        );
        assert_eq!(
            map_create_reservation_command_error_db_group(&mk(codes::BOOKING_NOT_FOUND)),
            Some(DbErrorGroup::NotFound)
        );
        assert_eq!(
            map_create_reservation_command_error_db_group(&mk(codes::SYSTEM_INTERNAL_ERROR)),
            None
        );
    }

    #[test]
    fn reservation_failure_context_keeps_only_scrubbed_flags_and_dates() {
        let context = reservation_failure_context(&reservation_request());

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

    #[test]
    fn unauthenticated_create_reservation_error_records_scrubbed_failure_context() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();
        let runtime_root = std::env::temp_dir().join(format!(
            "capyinn-create-reservation-auth-{}",
            uuid::Uuid::new_v4()
        ));

        std::env::set_var("CAPYINN_RUNTIME_ROOT", &runtime_root);
        let error = unauthenticated_create_reservation_error(
            &EffectiveCorrelationId {
                value: "COR-5E6F7A8B".to_string(),
                source: CorrelationIdSource::Frontend,
                rejected_length: None,
            },
            reservation_failure_context(&reservation_request()),
        );
        std::env::remove_var("CAPYINN_RUNTIME_ROOT");

        assert_eq!(error.code, codes::AUTH_NOT_AUTHENTICATED);
        assert_eq!(error.message, "Chưa đăng nhập");
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());

        let log_path = runtime_root
            .join("diagnostics")
            .join("command-failures.jsonl");
        let contents = fs::read_to_string(&log_path).expect("command failure log contents");
        let parsed: serde_json::Value =
            serde_json::from_str(contents.trim()).expect("command failure json");

        assert_eq!(parsed["command"], "create_reservation");
        assert_eq!(parsed["code"], codes::AUTH_NOT_AUTHENTICATED);
        assert_eq!(parsed["kind"], "user");
        assert_eq!(parsed["correlation_id"], "COR-5E6F7A8B");
        assert!(parsed["support_id"].is_null());
        assert_eq!(parsed["context"]["room_id"], "R101");
        assert_eq!(parsed["context"]["check_in_date"], "2026-04-25");
        assert_eq!(parsed["context"]["check_out_date"], "2026-04-27");
        assert_eq!(parsed["context"]["nights"], 2);
        assert_eq!(parsed["context"]["deposit_present"], true);
        assert_eq!(parsed["context"]["source"], "zalo");
        assert_eq!(parsed["context"]["notes_present"], true);
        assert!(parsed["context"].get("guest_name").is_none());
        assert!(parsed["context"].get("guest_phone").is_none());
        assert!(parsed["context"].get("guest_doc_number").is_none());
        assert!(parsed["context"].get("notes").is_none());

        let _ = fs::remove_dir_all(&runtime_root);
    }

    #[test]
    fn create_reservation_missing_room_failure_writes_not_found_group_without_support_log() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();
        let runtime_root = std::env::temp_dir().join(format!(
            "capyinn-create-reservation-missing-room-{}",
            uuid::Uuid::new_v4()
        ));

        let previous_runtime_root = std::env::var_os("CAPYINN_RUNTIME_ROOT");
        std::env::set_var("CAPYINN_RUNTIME_ROOT", &runtime_root);
        let context = reservation_failure_context(&reservation_request());
        let (error, db_error_group) = map_create_reservation_error(
            "create_reservation",
            &frontend_correlation_id(),
            BookingError::not_found("Không tìm thấy phòng R101"),
            context.clone(),
        );
        record_command_failure_with_db_group(
            "create_reservation",
            &error,
            "COR-1A2B3C4D",
            db_error_group,
            context,
        );
        restore_runtime_root(previous_runtime_root);

        let command_log_path = runtime_root
            .join("diagnostics")
            .join("command-failures.jsonl");
        let command_contents =
            fs::read_to_string(&command_log_path).expect("command failure log contents");
        let command_records = parse_json_lines(&command_contents);

        assert_eq!(error.code, codes::ROOM_NOT_FOUND);
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
        assert_eq!(db_error_group, Some(DbErrorGroup::NotFound));
        assert!(command_records.iter().any(|record| {
            record["command"] == "create_reservation"
                && record["code"] == codes::ROOM_NOT_FOUND
                && record["kind"] == "user"
                && record["db_error_group"] == "not_found"
        }));

        let support_log_path = runtime_root
            .join("diagnostics")
            .join("support-errors.jsonl");
        let support_records = if support_log_path.exists() {
            let support_contents =
                fs::read_to_string(&support_log_path).expect("support log contents");
            parse_json_lines(&support_contents)
        } else {
            Vec::new()
        };
        assert!(
            support_records.is_empty(),
            "expected no support log records for missing-room failure, found: {:?}",
            support_records
        );

        let _ = fs::remove_dir_all(&runtime_root);
    }
}
