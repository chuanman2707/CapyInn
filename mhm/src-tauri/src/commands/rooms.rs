use super::{emit_db_update, get_f64, get_user_id, AppState};
use crate::{
    app_error::{
        codes, correlation_context, log_system_error, normalize_correlation_id, CommandError,
        CommandResult, EffectiveCorrelationId,
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
            base_price: get_f64(r, "base_price"),
            max_guests: r.try_get::<i32, _>("max_guests").unwrap_or(2),
            extra_person_fee: r.try_get::<f64, _>("extra_person_fee").unwrap_or(0.0),
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

fn map_stay_error(
    command_name: &str,
    effective_correlation_id: &EffectiveCorrelationId,
    error: BookingError,
    context: Value,
) -> CommandError {
    match error {
        BookingError::NotFound(message) if message.starts_with("Không tìm thấy phòng ") => {
            map_stay_user_error(
                codes::ROOM_NOT_FOUND,
                command_name,
                effective_correlation_id,
                message,
                &context,
            )
        }
        BookingError::NotFound(message)
            if message.starts_with("Không tìm thấy booking đang active ")
                || message.starts_with("Không tìm thấy booking ") =>
        {
            map_stay_user_error(
                codes::BOOKING_NOT_FOUND,
                command_name,
                effective_correlation_id,
                message,
                &context,
            )
        }
        BookingError::Validation(message) if message == "Phải có ít nhất 1 khách" => {
            map_stay_user_error(
                codes::BOOKING_GUEST_REQUIRED,
                command_name,
                effective_correlation_id,
                message,
                &context,
            )
        }
        BookingError::Validation(message) if message == "Number of nights must be greater than 0" => {
            map_stay_user_error(
                codes::BOOKING_INVALID_NIGHTS,
                command_name,
                effective_correlation_id,
                message,
                &context,
            )
        }
        BookingError::Validation(message)
            if message == "Tổng quyết toán phải lớn hơn hoặc bằng 0" =>
        {
            map_stay_user_error(
                codes::BOOKING_INVALID_SETTLEMENT_TOTAL,
                command_name,
                effective_correlation_id,
                message,
                &context,
            )
        }
        BookingError::Validation(message)
            if message == "Overpaid booking requires refund handling before checkout" =>
        {
            map_stay_user_error(
                codes::BOOKING_INVALID_STATE,
                command_name,
                effective_correlation_id,
                message,
                &context,
            )
        }
        BookingError::Validation(message) | BookingError::Conflict(message) => {
            map_stay_user_error(
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

#[tauri::command]
pub async fn check_in(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: CheckInRequest,
    correlation_id: Option<String>,
) -> CommandResult<Booking> {
    let effective_correlation_id = normalize_correlation_id(correlation_id);
    let error_context = json!({
        "room_id": req.room_id.clone(),
        "guest_count": req.guests.len(),
        "nights": req.nights,
    });
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
            map_stay_error("check_in", &effective_correlation_id, error, error_context)
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
        base_price: get_f64(&row, "base_price"),
        max_guests: row.try_get::<i32, _>("max_guests").unwrap_or(2),
        extra_person_fee: row.try_get::<f64, _>("extra_person_fee").unwrap_or(0.0),
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
        total_price: get_f64(&r, "total_price"),
        paid_amount: get_f64(&r, "paid_amount"),
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
    let error_context = json!({
        "booking_id": req.booking_id.clone(),
        "settlement_mode": req.settlement_mode,
        "final_total": req.final_total,
    });
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
            map_stay_error("check_out", &effective_correlation_id, error, error_context)
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

    // If clean, update room to vacant
    if new_status == "clean" {
        let room_id: (String,) = sqlx::query_as("SELECT room_id FROM housekeeping WHERE id = ?")
            .bind(&task_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| e.to_string())?;

        sqlx::query("UPDATE rooms SET status = 'vacant' WHERE id = ?")
            .bind(&room_id.0)
            .execute(&state.db)
            .await
            .map_err(|e| e.to_string())?;
    }

    emit_db_update(&app, "housekeeping");

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
            amount: get_f64(r, "amount"),
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
    use super::map_stay_error;
    use crate::app_error::{codes, AppErrorKind, CorrelationIdSource, EffectiveCorrelationId};
    use crate::domain::booking::BookingError;
    use serde_json::json;

    fn frontend_correlation_id() -> EffectiveCorrelationId {
        EffectiveCorrelationId {
            value: "COR-1A2B3C4D".to_string(),
            source: CorrelationIdSource::Frontend,
            rejected_length: None,
        }
    }

    #[test]
    fn map_stay_error_maps_missing_room_to_room_not_found_contract() {
        let error = map_stay_error(
            "check_in",
            &frontend_correlation_id(),
            BookingError::not_found("Không tìm thấy phòng R101"),
            json!({ "room_id": "R101" }),
        );

        assert_eq!(error.code, codes::ROOM_NOT_FOUND);
        assert_eq!(error.message, "Không tìm thấy phòng R101");
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
    }

    #[test]
    fn map_stay_error_maps_guest_required_validation_to_shared_code() {
        let error = map_stay_error(
            "check_in",
            &frontend_correlation_id(),
            BookingError::validation("Phải có ít nhất 1 khách"),
            json!({ "room_id": "R101" }),
        );

        assert_eq!(error.code, codes::BOOKING_GUEST_REQUIRED);
        assert_eq!(error.message, "Phải có ít nhất 1 khách");
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
    }

    #[test]
    fn map_stay_error_maps_invalid_settlement_total_to_shared_code() {
        let error = map_stay_error(
            "check_out",
            &frontend_correlation_id(),
            BookingError::validation("Tổng quyết toán phải lớn hơn hoặc bằng 0"),
            json!({ "booking_id": "booking-1" }),
        );

        assert_eq!(error.code, codes::BOOKING_INVALID_SETTLEMENT_TOTAL);
        assert_eq!(error.message, "Tổng quyết toán phải lớn hơn hoặc bằng 0");
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
    }

    #[test]
    fn map_stay_error_maps_invalid_checkout_state_to_shared_code() {
        let error = map_stay_error(
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
    }

    #[test]
    fn map_stay_error_keeps_system_errors_in_system_contract() {
        let error = map_stay_error(
            "check_out",
            &frontend_correlation_id(),
            BookingError::database("disk I/O failure"),
            json!({ "booking_id": "booking-1" }),
        );

        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
        assert_eq!(error.kind, AppErrorKind::System);
        assert!(error.support_id.is_some());
    }
}
