use super::{emit_db_update, get_money_vnd, get_user_id, AppState};
use crate::{
    app_error::{
        codes, correlation_context, log_system_error, normalize_correlation_id,
        record_command_failure_with_db_group, CommandError, CommandResult, EffectiveCorrelationId,
    },
    db_error_monitoring::{
        classify_db_error_code, classify_db_failure, inject_db_error_group,
        is_room_unavailable_conflict_message, DbErrorGroup, MonitoredDbFailure,
    },
    domain::booking::BookingError,
    models::*,
    queries::booking::revenue_queries,
    services::booking::stay_lifecycle,
};
use serde_json::{json, Value};
use sqlx::{Pool, Row, Sqlite};
use tauri::State;

// ─── Room Commands ───

pub async fn do_get_rooms(pool: &Pool<Sqlite>) -> Result<Vec<Room>, String> {
    let rows = sqlx::query(
        "SELECT id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status FROM rooms ORDER BY floor, id"
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let rooms: Vec<Room> = rows
        .iter()
        .map(|r| Room {
            id: r.get("id"),
            name: r.get("name"),
            room_type: r.get("type"),
            floor: r.get("floor"),
            has_balcony: r.get::<i32, _>("has_balcony") == 1,
            base_price: get_money_vnd(r, "base_price"),
            max_guests: r.try_get::<i32, _>("max_guests").unwrap_or(2),
            extra_person_fee: get_money_vnd(r, "extra_person_fee"),
            status: r.get("status"),
        })
        .collect();

    Ok(rooms)
}

#[tauri::command]
pub async fn get_rooms(state: State<'_, AppState>) -> Result<Vec<Room>, String> {
    do_get_rooms(&state.db).await
}

pub async fn do_get_dashboard_stats(pool: &Pool<Sqlite>) -> Result<DashboardStats, String> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    revenue_queries::load_dashboard_stats_for_date(pool, &today)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_dashboard_stats(state: State<'_, AppState>) -> Result<DashboardStats, String> {
    do_get_dashboard_stats(&state.db).await
}

// ─── Check-in Command ───

fn log_user_stay_error(
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

fn map_stay_user_error(
    code: &'static str,
    command_name: &str,
    effective_correlation_id: &EffectiveCorrelationId,
    message: String,
    context: &Value,
) -> CommandError {
    log_user_stay_error(
        command_name,
        effective_correlation_id,
        message.as_str(),
        context,
    );
    CommandError::user(code, message)
}

fn map_known_stay_error_code(
    command_name: &str,
    effective_correlation_id: &EffectiveCorrelationId,
    message: &str,
    context: &Value,
) -> Option<(CommandError, Option<DbErrorGroup>)> {
    if is_room_unavailable_conflict_message(message) {
        return Some((
            map_stay_user_error(
                codes::CONFLICT_ROOM_UNAVAILABLE,
                command_name,
                effective_correlation_id,
                message.to_string(),
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

fn map_stay_error(
    command_name: &str,
    effective_correlation_id: &EffectiveCorrelationId,
    error: BookingError,
    context: Value,
) -> (CommandError, Option<DbErrorGroup>) {
    match error {
        BookingError::NotFound(message) if message.starts_with("Không tìm thấy phòng ") => (
            map_stay_user_error(
                codes::ROOM_NOT_FOUND,
                command_name,
                effective_correlation_id,
                message,
                &context,
            ),
            Some(DbErrorGroup::NotFound),
        ),
        BookingError::NotFound(message)
            if message.starts_with("Không tìm thấy booking đang active ")
                || message.starts_with("Không tìm thấy booking ") =>
        {
            (
                map_stay_user_error(
                    codes::BOOKING_NOT_FOUND,
                    command_name,
                    effective_correlation_id,
                    message,
                    &context,
                ),
                Some(DbErrorGroup::NotFound),
            )
        }
        BookingError::Validation(message) if message == "Phải có ít nhất 1 khách" => (
            map_stay_user_error(
                codes::BOOKING_GUEST_REQUIRED,
                command_name,
                effective_correlation_id,
                message,
                &context,
            ),
            None,
        ),
        BookingError::Validation(message)
            if message == "Number of nights must be greater than 0" =>
        {
            (
                map_stay_user_error(
                    codes::BOOKING_INVALID_NIGHTS,
                    command_name,
                    effective_correlation_id,
                    message,
                    &context,
                ),
                None,
            )
        }
        BookingError::Validation(message)
            if message == "Tổng quyết toán phải lớn hơn hoặc bằng 0" =>
        {
            (
                map_stay_user_error(
                    codes::BOOKING_INVALID_SETTLEMENT_TOTAL,
                    command_name,
                    effective_correlation_id,
                    message,
                    &context,
                ),
                None,
            )
        }
        BookingError::Validation(message)
            if message == "Overpaid booking requires refund handling before checkout" =>
        {
            (
                map_stay_user_error(
                    codes::BOOKING_INVALID_STATE,
                    command_name,
                    effective_correlation_id,
                    message,
                    &context,
                ),
                None,
            )
        }
        BookingError::Validation(message) | BookingError::Conflict(message) => {
            if let Some(mapped) = map_known_stay_error_code(
                command_name,
                effective_correlation_id,
                &message,
                &context,
            ) {
                return mapped;
            }
            (
                map_stay_user_error(
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
            if let Some(mapped) = map_known_stay_error_code(
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
            if let Some(mapped) = map_known_stay_error_code(
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

fn check_in_failure_context(req: &CheckInRequest) -> Value {
    let notes_present = req
        .notes
        .as_ref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);

    json!({
        "room_id": req.room_id.clone(),
        "guest_count": req.guests.len(),
        "nights": req.nights,
        "source": req.source.clone(),
        "notes_present": notes_present,
    })
}

fn check_out_failure_context(req: &CheckOutRequest) -> Value {
    json!({
        "booking_id": req.booking_id.clone(),
        "settlement_mode": req.settlement_mode,
        "final_total": req.final_total,
    })
}

#[tauri::command]
pub async fn check_in(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: CheckInRequest,
    correlation_id: Option<String>,
) -> CommandResult<Booking> {
    let effective_correlation_id = normalize_correlation_id(correlation_id);
    let error_context = check_in_failure_context(&req);
    log::info!(
        "check_in start correlation_id={} source={:?} room_id={} guest_count={} nights={}",
        effective_correlation_id.value,
        effective_correlation_id.source,
        req.room_id,
        req.guests.len(),
        req.nights
    );
    let booking = stay_lifecycle::check_in(&state.db, req, get_user_id(&state))
        .await
        .map_err(|error| {
            let (command_error, db_error_group) = map_stay_error(
                "check_in",
                &effective_correlation_id,
                error,
                error_context.clone(),
            );
            record_command_failure_with_db_group(
                "check_in",
                &command_error,
                &effective_correlation_id.value,
                db_error_group,
                error_context.clone(),
            );
            command_error
        })?;

    log::info!(
        "check_in success correlation_id={} source={:?} booking_id={} room_id={}",
        effective_correlation_id.value,
        effective_correlation_id.source,
        booking.id,
        booking.room_id
    );

    emit_db_update(&app, "rooms");

    Ok(booking)
}

// ─── Room Detail Command ───

pub async fn do_get_room_detail(
    pool: &Pool<Sqlite>,
    room_id: &str,
) -> Result<RoomWithBooking, String> {
    let row = sqlx::query("SELECT id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status FROM rooms WHERE id = ?")
        .bind(room_id)
        .fetch_one(pool).await.map_err(|e| e.to_string())?;

    let room = Room {
        id: row.get("id"),
        name: row.get("name"),
        room_type: row.get("type"),
        floor: row.get("floor"),
        has_balcony: row.get::<i32, _>("has_balcony") == 1,
        base_price: get_money_vnd(&row, "base_price"),
        max_guests: row.try_get::<i32, _>("max_guests").unwrap_or(2),
        extra_person_fee: get_money_vnd(&row, "extra_person_fee"),
        status: row.get("status"),
    };

    let booking = sqlx::query(
        "SELECT id, room_id, primary_guest_id, check_in_at, expected_checkout, actual_checkout, nights, total_price, paid_amount, status, source, notes, created_at
         FROM bookings WHERE room_id = ? AND status = 'active' LIMIT 1"
    )
    .bind(room_id)
    .fetch_optional(pool).await.map_err(|e| e.to_string())?
    .map(|r| Booking {
        id: r.get("id"),
        room_id: r.get("room_id"),
        primary_guest_id: r.get("primary_guest_id"),
        check_in_at: r.get("check_in_at"),
        expected_checkout: r.get("expected_checkout"),
        actual_checkout: r.get("actual_checkout"),
        nights: r.get("nights"),
        total_price: get_money_vnd(&r, "total_price"),
        paid_amount: get_money_vnd(&r, "paid_amount"),
        status: r.get("status"),
        source: r.get("source"),
        notes: r.get("notes"),
        created_at: r.get("created_at"),
    });

    let guests = if let Some(ref b) = booking {
        let rows = sqlx::query(
            "SELECT g.* FROM guests g
             JOIN booking_guests bg ON bg.guest_id = g.id
             WHERE bg.booking_id = ?",
        )
        .bind(&b.id)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;

        rows.iter()
            .map(|r| Guest {
                id: r.get("id"),
                guest_type: r.get("guest_type"),
                full_name: r.get("full_name"),
                doc_number: r.get("doc_number"),
                dob: r.get("dob"),
                gender: r.get("gender"),
                nationality: r.get("nationality"),
                address: r.get("address"),
                visa_expiry: r.get("visa_expiry"),
                scan_path: r.get("scan_path"),
                phone: r.get("phone"),
                created_at: r.get("created_at"),
            })
            .collect()
    } else {
        vec![]
    };

    Ok(RoomWithBooking {
        room,
        booking,
        guests,
    })
}

#[tauri::command]
pub async fn get_room_detail(
    state: State<'_, AppState>,
    room_id: String,
) -> Result<RoomWithBooking, String> {
    do_get_room_detail(&state.db, &room_id).await
}

// ─── Check-out Command ───

#[tauri::command]
pub async fn check_out(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: CheckOutRequest,
    correlation_id: Option<String>,
) -> CommandResult<()> {
    let effective_correlation_id = normalize_correlation_id(correlation_id);
    let error_context = check_out_failure_context(&req);
    log::info!(
        "check_out start correlation_id={} source={:?} booking_id={} settlement_mode={:?} final_total={}",
        effective_correlation_id.value,
        effective_correlation_id.source,
        req.booking_id,
        req.settlement_mode,
        req.final_total
    );
    stay_lifecycle::check_out(&state.db, req)
        .await
        .map_err(|error| {
            let (command_error, db_error_group) = map_stay_error(
                "check_out",
                &effective_correlation_id,
                error,
                error_context.clone(),
            );
            record_command_failure_with_db_group(
                "check_out",
                &command_error,
                &effective_correlation_id.value,
                db_error_group,
                error_context.clone(),
            );
            command_error
        })?;

    log::info!(
        "check_out success correlation_id={} source={:?}",
        effective_correlation_id.value,
        effective_correlation_id.source
    );

    emit_db_update(&app, "rooms");

    if let Err(error) =
        crate::backup::request_backup(&app, crate::backup::BackupReason::Checkout).await
    {
        crate::backup::log_backup_request_error("check_out", &error);
    }

    Ok(())
}

#[allow(dead_code)]
#[tauri::command]
pub async fn preview_checkout_settlement(
    state: State<'_, AppState>,
    req: CheckoutSettlementPreviewRequest,
) -> Result<CheckoutSettlementPreview, String> {
    stay_lifecycle::preview_checkout_settlement(&state.db, req)
        .await
        .map_err(|error| error.to_string())
}

// ─── Extend Stay ───

#[tauri::command]
pub async fn extend_stay(
    state: State<'_, AppState>,
    booking_id: String,
) -> Result<Booking, String> {
    stay_lifecycle::extend_stay(&state.db, &booking_id)
        .await
        .map_err(|error| error.to_string())
}

// ─── Housekeeping Commands ───

#[tauri::command]
pub async fn get_housekeeping_tasks(
    state: State<'_, AppState>,
) -> Result<Vec<HousekeepingTask>, String> {
    let rows =
        sqlx::query("SELECT * FROM housekeeping WHERE status != 'clean' ORDER BY triggered_at ASC")
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| HousekeepingTask {
            id: r.get("id"),
            room_id: r.get("room_id"),
            status: r.get("status"),
            note: r.get("note"),
            triggered_at: r.get("triggered_at"),
            cleaned_at: r.get("cleaned_at"),
            created_at: r.get("created_at"),
        })
        .collect())
}

#[tauri::command]
pub async fn update_housekeeping(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    task_id: String,
    new_status: String,
    note: Option<String>,
) -> Result<(), String> {
    if new_status == "clean" {
        complete_housekeeping_clean_to_vacant(&state.db, &task_id, note.as_deref())
            .await
            .map_err(|error| format!("{}: {}", error.code, error.message))?;
        emit_db_update(&app, "housekeeping");
        return Ok(());
    }

    let now = chrono::Local::now();

    let cleaned_at = if new_status == "clean" {
        Some(now.to_rfc3339())
    } else {
        None
    };

    sqlx::query(
        "UPDATE housekeeping SET status = ?, note = COALESCE(?, note), cleaned_at = ? WHERE id = ?",
    )
    .bind(&new_status)
    .bind(&note)
    .bind(&cleaned_at)
    .bind(&task_id)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    emit_db_update(&app, "housekeeping");

    Ok(())
}

async fn complete_housekeeping_clean_to_vacant(
    pool: &Pool<Sqlite>,
    task_id: &str,
    note: Option<&str>,
) -> CommandResult<()> {
    let room_id: String = sqlx::query_scalar("SELECT room_id FROM housekeeping WHERE id = ?")
        .bind(task_id)
        .fetch_one(pool)
        .await
        .map_err(|error| {
            log_system_error(
                "update_housekeeping",
                error.to_string(),
                json!({
                    "task_id": task_id,
                    "step": "lookup_room",
                }),
            )
        })?;

    let _lock_guard = crate::aggregate_locks::global_manager()
        .acquire([crate::aggregate_locks::room_key(&room_id)?])
        .await?;

    let mut tx = pool.begin().await.map_err(|error| {
        log_system_error(
            "update_housekeeping",
            error.to_string(),
            json!({
                "task_id": task_id,
                "room_id": room_id,
                "step": "begin",
            }),
        )
    })?;

    let cleaned_at = chrono::Local::now().to_rfc3339();
    let housekeeping_result = sqlx::query(
        "UPDATE housekeeping
         SET status = 'clean', note = COALESCE(?, note), cleaned_at = ?
         WHERE id = ? AND status = 'cleaning'",
    )
    .bind(note)
    .bind(&cleaned_at)
    .bind(task_id)
    .execute(&mut *tx)
    .await
    .map_err(|error| {
        log_system_error(
            "update_housekeeping",
            error.to_string(),
            json!({
                "task_id": task_id,
                "room_id": room_id,
                "step": "update_housekeeping",
            }),
        )
    })?;

    if housekeeping_result.rows_affected() != 1 {
        let _ = tx.rollback().await;
        return Err(CommandError::user(
            codes::CONFLICT_INVALID_STATE_TRANSITION,
            "Housekeeping task is no longer cleaning",
        ));
    }

    let room_result =
        sqlx::query("UPDATE rooms SET status = 'vacant' WHERE id = ? AND status = 'cleaning'")
            .bind(&room_id)
            .execute(&mut *tx)
            .await
            .map_err(|error| {
                log_system_error(
                    "update_housekeeping",
                    error.to_string(),
                    json!({
                        "task_id": task_id,
                        "room_id": room_id,
                        "step": "update_room",
                    }),
                )
            })?;

    if room_result.rows_affected() != 1 {
        let _ = tx.rollback().await;
        return Err(CommandError::user(
            codes::CONFLICT_INVALID_STATE_TRANSITION,
            "Room is no longer waiting for cleaning completion",
        ));
    }

    tx.commit().await.map_err(|error| {
        log_system_error(
            "update_housekeeping",
            error.to_string(),
            json!({
                "task_id": task_id,
                "room_id": room_id,
                "step": "commit",
            }),
        )
    })?;

    Ok(())
}

// ─── Expense Commands ───

#[tauri::command]
pub async fn create_expense(
    state: State<'_, AppState>,
    req: CreateExpenseRequest,
) -> Result<Expense, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Local::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO expenses (id, category, amount, note, expense_date, created_at)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&req.category)
    .bind(req.amount)
    .bind(&req.note)
    .bind(&req.expense_date)
    .bind(&now)
    .execute(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    Ok(Expense {
        id,
        category: req.category,
        amount: req.amount,
        note: req.note,
        expense_date: req.expense_date,
        created_at: now,
    })
}

#[tauri::command]
pub async fn get_expenses(
    state: State<'_, AppState>,
    from: String,
    to: String,
) -> Result<Vec<Expense>, String> {
    let rows = sqlx::query(
        "SELECT * FROM expenses WHERE expense_date BETWEEN ? AND ? ORDER BY expense_date DESC",
    )
    .bind(&from)
    .bind(&to)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| Expense {
            id: r.get("id"),
            category: r.get("category"),
            amount: get_money_vnd(r, "amount"),
            note: r.get("note"),
            expense_date: r.get("expense_date"),
            created_at: r.get("created_at"),
        })
        .collect())
}

// ─── Statistics Commands ───

#[tauri::command]
pub async fn get_revenue_stats(
    state: State<'_, AppState>,
    from: String,
    to: String,
) -> Result<RevenueStats, String> {
    revenue_queries::load_revenue_stats(&state.db, &from, &to)
        .await
        .map_err(|e| e.to_string())
}

// ─── Copy Lưu Trú ───

#[tauri::command]
pub async fn get_stay_info_text(
    state: State<'_, AppState>,
    booking_id: String,
) -> Result<String, String> {
    let b = sqlx::query("SELECT * FROM bookings WHERE id = ?")
        .bind(&booking_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    let g = sqlx::query("SELECT * FROM guests WHERE id = ?")
        .bind(b.get::<String, _>("primary_guest_id"))
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    let room_id: String = b.get("room_id");
    let full_name: String = g.get("full_name");
    let doc_number: String = g.get("doc_number");
    let dob: String = g.get::<Option<String>, _>("dob").unwrap_or_default();
    let gender: String = g.get::<Option<String>, _>("gender").unwrap_or_default();
    let nationality: String = g
        .get::<Option<String>, _>("nationality")
        .unwrap_or_else(|| "Việt Nam".to_string());
    let address: String = g.get::<Option<String>, _>("address").unwrap_or_default();
    let check_in: String = b.get("check_in_at");
    let checkout: String = b.get("expected_checkout");

    let text = format!(
        "Họ và tên: {}\nSố CCCD: {}\nNgày sinh: {}\nGiới tính: {}\nQuốc tịch: {}\nĐịa chỉ: {}\nPhòng: {}\nNgày đến: {}\nNgày đi: {}",
        full_name, doc_number, dob, gender, nationality, address, room_id, check_in, checkout
    );

    Ok(text)
}

// ─── OCR Scan Command ───

#[tauri::command]
pub async fn scan_image(path: String) -> Result<crate::ocr::CccdInfo, String> {
    let image_path = std::path::Path::new(&path);
    if !image_path.exists() {
        return Err(format!("File not found: {}", path));
    }

    let engine = crate::ocr::create_engine()?;
    let lines = crate::ocr::ocr_image(&engine, image_path)?;
    let cccd = crate::ocr::parse_cccd(&lines);

    Ok(cccd)
}

#[cfg(test)]
mod tests {
    use super::{
        check_in_failure_context, check_out_failure_context, complete_housekeeping_clean_to_vacant,
        map_stay_error,
    };
    use crate::app_error::{
        codes, record_command_failure_with_db_group, AppErrorKind, CorrelationIdSource,
        EffectiveCorrelationId,
    };
    use crate::db_error_monitoring::DbErrorGroup;
    use crate::domain::booking::BookingError;
    use crate::models::{
        CheckInRequest, CheckOutRequest, CheckoutSettlementMode, CreateGuestRequest,
    };
    use serde_json::json;
    use sqlx::{sqlite::SqlitePoolOptions, Row};
    use std::fs;

    async fn migrated_pool() -> sqlx::Pool<sqlx::Sqlite> {
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

    #[tokio::test]
    async fn housekeeping_clean_does_not_mark_occupied_room_vacant() {
        let pool = migrated_pool().await;
        sqlx::query(
            "INSERT INTO rooms (id, name, type, floor, has_balcony, base_price, max_guests, extra_person_fee, status)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("R-HK")
        .bind("Housekeeping Guard")
        .bind("standard")
        .bind(1)
        .bind(0)
        .bind(100000.0)
        .bind(2)
        .bind(0.0)
        .bind("occupied")
        .execute(&pool)
        .await
        .expect("insert occupied room");
        sqlx::query(
            "INSERT INTO housekeeping (id, room_id, status, note, triggered_at, cleaned_at, created_at)
             VALUES (?, ?, ?, ?, datetime('now'), NULL, datetime('now'))",
        )
        .bind("HK1")
        .bind("R-HK")
        .bind("cleaning")
        .bind("started")
        .execute(&pool)
        .await
        .expect("insert housekeeping task");

        let error = complete_housekeeping_clean_to_vacant(&pool, "HK1", None)
            .await
            .expect_err("occupied room should reject clean-to-vacant");

        assert_eq!(error.code, codes::CONFLICT_INVALID_STATE_TRANSITION);

        let room = sqlx::query("SELECT status FROM rooms WHERE id = ?")
            .bind("R-HK")
            .fetch_one(&pool)
            .await
            .expect("room status");
        assert_eq!(room.get::<String, _>("status"), "occupied");

        let housekeeping = sqlx::query("SELECT status FROM housekeeping WHERE id = ?")
            .bind("HK1")
            .fetch_one(&pool)
            .await
            .expect("housekeeping status");
        assert_eq!(housekeeping.get::<String, _>("status"), "cleaning");
    }

    #[test]
    fn map_stay_error_maps_missing_room_to_room_not_found_contract() {
        let (error, db_error_group) = map_stay_error(
            "check_in",
            &frontend_correlation_id(),
            BookingError::not_found("Không tìm thấy phòng R101"),
            json!({ "room_id": "R101" }),
        );

        assert_eq!(error.code, codes::ROOM_NOT_FOUND);
        assert_eq!(error.message, "Không tìm thấy phòng R101");
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
        assert_eq!(db_error_group, Some(DbErrorGroup::NotFound));
    }

    #[test]
    fn map_stay_error_maps_guest_required_validation_to_shared_code() {
        let (error, db_error_group) = map_stay_error(
            "check_in",
            &frontend_correlation_id(),
            BookingError::validation("Phải có ít nhất 1 khách"),
            json!({ "room_id": "R101" }),
        );

        assert_eq!(error.code, codes::BOOKING_GUEST_REQUIRED);
        assert_eq!(error.message, "Phải có ít nhất 1 khách");
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
        assert!(db_error_group.is_none());
    }

    #[test]
    fn map_stay_error_maps_invalid_settlement_total_to_shared_code() {
        let (error, db_error_group) = map_stay_error(
            "check_out",
            &frontend_correlation_id(),
            BookingError::validation("Tổng quyết toán phải lớn hơn hoặc bằng 0"),
            json!({ "booking_id": "booking-1" }),
        );

        assert_eq!(error.code, codes::BOOKING_INVALID_SETTLEMENT_TOTAL);
        assert_eq!(error.message, "Tổng quyết toán phải lớn hơn hoặc bằng 0");
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
        assert!(db_error_group.is_none());
    }

    #[test]
    fn map_stay_error_maps_invalid_checkout_state_to_shared_code() {
        let (error, db_error_group) = map_stay_error(
            "check_out",
            &frontend_correlation_id(),
            BookingError::validation("Overpaid booking requires refund handling before checkout"),
            json!({ "booking_id": "booking-1" }),
        );

        assert_eq!(error.code, codes::BOOKING_INVALID_STATE);
        assert_eq!(
            error.message,
            "Overpaid booking requires refund handling before checkout"
        );
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
        assert!(db_error_group.is_none());
    }

    #[test]
    fn map_stay_error_maps_legacy_calendar_conflict_to_stable_code() {
        let (error, db_error_group) = map_stay_error(
            "check_in",
            &frontend_correlation_id(),
            BookingError::conflict(
                "Room R101 has a reservation starting 2026-04-20 (Guest). Max 2 nights.",
            ),
            json!({ "room_id": "R101" }),
        );

        assert_eq!(error.code, codes::CONFLICT_ROOM_UNAVAILABLE);
        assert!(error.message.contains("has a reservation starting"));
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
        assert_eq!(db_error_group, Some(DbErrorGroup::Constraint));
    }

    #[test]
    fn map_stay_error_maps_database_write_to_system_contract_with_write_failed_group() {
        let (error, db_error_group) = map_stay_error(
            "check_out",
            &frontend_correlation_id(),
            BookingError::database_write("disk full"),
            json!({ "booking_id": "booking-1" }),
        );

        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
        assert_eq!(error.kind, AppErrorKind::System);
        assert!(error.support_id.is_some());
        assert_eq!(db_error_group, Some(DbErrorGroup::WriteFailed));
    }

    #[test]
    fn map_stay_error_maps_locked_database_reads_to_retryable_code() {
        let (error, db_error_group) = map_stay_error(
            "check_out",
            &frontend_correlation_id(),
            BookingError::database("database is locked"),
            json!({ "booking_id": "booking-1" }),
        );

        assert_eq!(error.code, codes::DB_LOCKED_RETRYABLE);
        assert_eq!(error.kind, AppErrorKind::System);
        assert!(error.support_id.is_some());
        assert!(error.retryable);
        assert_eq!(db_error_group, Some(DbErrorGroup::Locked));
    }

    #[test]
    fn check_in_failure_context_uses_counts_and_flags_only() {
        let context = check_in_failure_context(&CheckInRequest {
            room_id: "R101".to_string(),
            guests: vec![
                CreateGuestRequest {
                    guest_type: Some("domestic".to_string()),
                    full_name: "Nguyen Van A".to_string(),
                    doc_number: "012345678901".to_string(),
                    dob: None,
                    gender: None,
                    nationality: None,
                    address: Some("Hanoi".to_string()),
                    visa_expiry: None,
                    scan_path: None,
                    phone: Some("0901".to_string()),
                },
                CreateGuestRequest {
                    guest_type: Some("domestic".to_string()),
                    full_name: "Tran Thi B".to_string(),
                    doc_number: "109876543210".to_string(),
                    dob: None,
                    gender: None,
                    nationality: None,
                    address: None,
                    visa_expiry: None,
                    scan_path: None,
                    phone: None,
                },
            ],
            nights: 2,
            source: Some("walk-in".to_string()),
            notes: Some("Late arrival".to_string()),
            paid_amount: Some(500_000),
            pricing_type: None,
        });

        assert_eq!(
            context,
            json!({
                "room_id": "R101",
                "guest_count": 2,
                "nights": 2,
                "source": "walk-in",
                "notes_present": true,
            })
        );
        assert!(context.get("guests").is_none());
        assert!(context.get("notes").is_none());
    }

    #[test]
    fn check_out_failure_context_keeps_booking_settlement_and_total_only() {
        let context = check_out_failure_context(&CheckOutRequest {
            booking_id: "booking-1".to_string(),
            settlement_mode: CheckoutSettlementMode::Hourly,
            final_total: 400_000,
        });

        assert_eq!(
            context,
            json!({
                "booking_id": "booking-1",
                "settlement_mode": "hourly",
                "final_total": 400000,
            })
        );
    }

    #[test]
    fn system_check_out_failure_writes_same_db_error_group_to_both_logs() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();
        let runtime_root = std::env::temp_dir().join(format!(
            "capyinn-check-out-support-id-{}",
            uuid::Uuid::new_v4()
        ));

        let previous_runtime_root = std::env::var_os("CAPYINN_RUNTIME_ROOT");
        std::env::set_var("CAPYINN_RUNTIME_ROOT", &runtime_root);
        let context = check_out_failure_context(&CheckOutRequest {
            booking_id: "booking-1".to_string(),
            settlement_mode: CheckoutSettlementMode::BookedNights,
            final_total: 2_500_000,
        });
        let (error, db_error_group) = map_stay_error(
            "check_out",
            &frontend_correlation_id(),
            BookingError::database("disk I/O failure"),
            context.clone(),
        );
        let support_id = error.support_id.clone().expect("system error support id");
        record_command_failure_with_db_group(
            "check_out",
            &error,
            "COR-1A2B3C4D",
            db_error_group,
            context,
        );
        restore_runtime_root(previous_runtime_root);

        let support_log_path = runtime_root
            .join("diagnostics")
            .join("support-errors.jsonl");
        let support_contents = fs::read_to_string(&support_log_path).expect("support log contents");
        let support_records = parse_json_lines(&support_contents);

        let command_log_path = runtime_root
            .join("diagnostics")
            .join("command-failures.jsonl");
        let command_contents =
            fs::read_to_string(&command_log_path).expect("command failure log contents");
        let command_records = parse_json_lines(&command_contents);

        assert!(support_records.iter().any(|record| {
            record["support_id"] == support_id
                && record["command"] == "check_out"
                && record["code"] == codes::SYSTEM_INTERNAL_ERROR
                && record["context"]["db_error_group"] == "unknown"
        }));
        assert!(command_records.iter().any(|record| {
            record["support_id"] == support_id
                && record["command"] == "check_out"
                && record["code"] == codes::SYSTEM_INTERNAL_ERROR
                && record["db_error_group"] == "unknown"
        }));
        assert_eq!(db_error_group, Some(DbErrorGroup::Unknown));

        let _ = fs::remove_dir_all(&runtime_root);
    }
}
