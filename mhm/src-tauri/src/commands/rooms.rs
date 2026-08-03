use super::{emit_db_update, get_user_id, AppState};
use crate::{
    app_error::{
        codes, normalize_correlation_id, record_command_failure_with_db_group, CommandError,
        CommandResult,
    },
    command_idempotency::WriteCommandContext,
    models::*,
    money::validate_non_negative_money_vnd,
    queries::booking::{expense_queries, revenue_queries, room_queries, stay_info_queries},
    repositories::booking::expense_repository,
    services::{
        booking::{backfill, room_change, stay_lifecycle},
        housekeeping::housekeeping_service,
    },
};
use serde_json::{json, Value};
use sqlx::{Pool, Sqlite};
use tauri::State;

// ─── Room Commands ───

pub async fn do_get_rooms(pool: &Pool<Sqlite>) -> Result<Vec<Room>, String> {
    room_queries::load_rooms(pool)
        .await
        .map_err(|e| e.to_string())
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

fn extend_stay_failure_context(booking_id: &str) -> Value {
    json!({
        "booking_id": booking_id,
        "operation": "add_one_night",
    })
}

fn change_room_failure_context(booking_id: &str, new_room_id: &str) -> Value {
    json!({
        "booking_id": booking_id,
        "new_room_id": new_room_id,
    })
}

fn should_request_checkout_backup(replayed: bool) -> bool {
    !replayed
}

#[tauri::command]
pub async fn check_in(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: CheckInRequest,
    correlation_id: Option<String>,
    idempotency_key: String,
) -> CommandResult<Booking> {
    let effective_correlation_id = normalize_correlation_id(correlation_id);
    let error_context = check_in_failure_context(&req);
    let actor_id = get_user_id(&state)
        .ok_or_else(|| CommandError::user(codes::AUTH_NOT_AUTHENTICATED, "Chưa đăng nhập"))?;
    let mut write_command_context = WriteCommandContext::for_scoped_command(
        effective_correlation_id.value.clone(),
        idempotency_key,
        "check_in",
    )?;
    write_command_context.actor_id = Some(actor_id.clone());
    log::info!(
        "check_in start correlation_id={} source={:?} room_id={} guest_count={} nights={}",
        effective_correlation_id.value,
        effective_correlation_id.source,
        req.room_id,
        req.guests.len(),
        req.nights
    );
    let result =
        stay_lifecycle::check_in_idempotent(&state.db, &write_command_context, req, Some(actor_id))
            .await
            .inspect_err(|command_error| {
                record_command_failure_with_db_group(
                    "check_in",
                    command_error,
                    &effective_correlation_id.value,
                    None,
                    error_context.clone(),
                );
            })?;
    let booking: Booking = serde_json::from_value(result.response).map_err(|error| {
        CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            format!("Invalid check_in idempotent response: {error}"),
        )
        .with_request_id(write_command_context.request_id.clone())
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

// ─── Backfill Stay Command ───

fn backfill_stay_failure_context(req: &BackfillStayRequest) -> Value {
    json!({
        "room_id": req.room_id.clone(),
        "guest_count": req.guests.len(),
        "still_staying": req.check_out_date.is_none(),
        "source": req.source.clone(),
    })
}

#[tauri::command]
pub async fn backfill_stay(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: BackfillStayRequest,
    correlation_id: Option<String>,
    idempotency_key: String,
) -> CommandResult<Booking> {
    let effective_correlation_id = normalize_correlation_id(correlation_id);
    let error_context = backfill_stay_failure_context(&req);
    let actor_id = get_user_id(&state)
        .ok_or_else(|| CommandError::user(codes::AUTH_NOT_AUTHENTICATED, "Chưa đăng nhập"))?;
    let mut write_command_context = WriteCommandContext::for_scoped_command(
        effective_correlation_id.value.clone(),
        idempotency_key,
        "backfill_stay",
    )?;
    write_command_context.actor_id = Some(actor_id.clone());
    log::info!(
        "backfill_stay start correlation_id={} room_id={} guest_count={}",
        effective_correlation_id.value,
        req.room_id,
        req.guests.len()
    );
    let result =
        backfill::backfill_stay_idempotent(&state.db, &write_command_context, req, Some(actor_id))
            .await
            .inspect_err(|command_error| {
                record_command_failure_with_db_group(
                    "backfill_stay",
                    command_error,
                    &effective_correlation_id.value,
                    None,
                    error_context.clone(),
                );
            })?;
    let booking: Booking = serde_json::from_value(result.response).map_err(|error| {
        CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            format!("Invalid backfill_stay idempotent response: {error}"),
        )
        .with_request_id(write_command_context.request_id.clone())
    })?;

    log::info!(
        "backfill_stay success correlation_id={} booking_id={} room_id={}",
        effective_correlation_id.value,
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
    room_queries::load_room_detail(pool, room_id)
        .await
        .map_err(|e| e.to_string())
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
    idempotency_key: String,
) -> CommandResult<CheckOutResponse> {
    let effective_correlation_id = normalize_correlation_id(correlation_id);
    let error_context = check_out_failure_context(&req);
    let actor_id = get_user_id(&state)
        .ok_or_else(|| CommandError::user(codes::AUTH_NOT_AUTHENTICATED, "Chưa đăng nhập"))?;
    let mut write_command_context = WriteCommandContext::for_scoped_command(
        effective_correlation_id.value.clone(),
        idempotency_key,
        "check_out",
    )?;
    write_command_context.actor_id = Some(actor_id);
    log::info!(
        "check_out start correlation_id={} source={:?} booking_id={} settlement_mode={:?} final_total={}",
        effective_correlation_id.value,
        effective_correlation_id.source,
        req.booking_id,
        req.settlement_mode,
        req.final_total
    );
    let result = stay_lifecycle::check_out_idempotent(&state.db, &write_command_context, req)
        .await
        .inspect_err(|command_error| {
            record_command_failure_with_db_group(
                "check_out",
                command_error,
                &effective_correlation_id.value,
                None,
                error_context.clone(),
            );
        })?;
    let response: CheckOutResponse = serde_json::from_value(result.response).map_err(|error| {
        CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            format!("Invalid check_out idempotent response: {error}"),
        )
        .with_request_id(write_command_context.request_id.clone())
    })?;

    log::info!(
        "check_out success correlation_id={} source={:?} booking_id={} room_id={}",
        effective_correlation_id.value,
        effective_correlation_id.source,
        response.booking_id,
        response.room_id
    );

    emit_db_update(&app, "rooms");

    if should_request_checkout_backup(result.replayed) {
        if let Err(error) =
            crate::backup::request_backup(&app, crate::backup::BackupReason::Checkout).await
        {
            crate::backup::log_backup_request_error("check_out", &error);
        }
    }

    Ok(response)
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
    app: tauri::AppHandle,
    correlation_id: Option<String>,
    idempotency_key: String,
) -> CommandResult<Booking> {
    let effective_correlation_id = normalize_correlation_id(correlation_id);
    let error_context = extend_stay_failure_context(&booking_id);
    let actor_id = get_user_id(&state)
        .ok_or_else(|| CommandError::user(codes::AUTH_NOT_AUTHENTICATED, "Chưa đăng nhập"))?;
    let mut write_command_context = WriteCommandContext::for_scoped_command(
        effective_correlation_id.value.clone(),
        idempotency_key,
        "extend_stay",
    )?;
    write_command_context.actor_id = Some(actor_id);
    log::info!(
        "extend_stay start correlation_id={} source={:?} booking_id={}",
        effective_correlation_id.value,
        effective_correlation_id.source,
        booking_id
    );
    let result =
        stay_lifecycle::extend_stay_idempotent(&state.db, &write_command_context, &booking_id)
            .await
            .inspect_err(|command_error| {
                record_command_failure_with_db_group(
                    "extend_stay",
                    command_error,
                    &effective_correlation_id.value,
                    None,
                    error_context.clone(),
                );
            })?;
    let booking: Booking = serde_json::from_value(result.response).map_err(|error| {
        CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            format!("Invalid extend_stay idempotent response: {error}"),
        )
        .with_request_id(write_command_context.request_id.clone())
    })?;

    log::info!(
        "extend_stay success correlation_id={} source={:?} booking_id={} room_id={}",
        effective_correlation_id.value,
        effective_correlation_id.source,
        booking.id,
        booking.room_id
    );

    emit_db_update(&app, "rooms");

    Ok(booking)
}

// ─── Change Room ───

#[tauri::command]
pub async fn get_room_change_options(
    state: State<'_, AppState>,
    booking_id: String,
) -> Result<RoomChangeOptions, String> {
    room_change::load_options(&state.db, &booking_id, chrono::Local::now().date_naive())
        .await
        .map_err(|error| error.to_string())
}

#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn change_room(
    state: State<'_, AppState>,
    booking_id: String,
    new_room_id: String,
    keep_price: bool,
    reason: Option<String>,
    app: tauri::AppHandle,
    correlation_id: Option<String>,
    idempotency_key: String,
) -> CommandResult<Booking> {
    let effective_correlation_id = normalize_correlation_id(correlation_id);
    let error_context = change_room_failure_context(&booking_id, &new_room_id);
    let actor_id = get_user_id(&state)
        .ok_or_else(|| CommandError::user(codes::AUTH_NOT_AUTHENTICATED, "Chưa đăng nhập"))?;
    let mut write_command_context = WriteCommandContext::for_scoped_command(
        effective_correlation_id.value.clone(),
        idempotency_key,
        "change_room",
    )?;
    write_command_context.actor_id = Some(actor_id);
    log::info!(
        "change_room start correlation_id={} source={:?} booking_id={} new_room_id={}",
        effective_correlation_id.value,
        effective_correlation_id.source,
        booking_id,
        new_room_id
    );

    let result = room_change::change_room_idempotent(
        &state.db,
        &write_command_context,
        &booking_id,
        &new_room_id,
        keep_price,
        reason,
    )
    .await
    .inspect_err(|command_error| {
        record_command_failure_with_db_group(
            "change_room",
            command_error,
            &effective_correlation_id.value,
            None,
            error_context.clone(),
        );
    })?;

    let booking: Booking = serde_json::from_value(result.response).map_err(|error| {
        CommandError::system(
            codes::SYSTEM_INTERNAL_ERROR,
            format!("Invalid change_room idempotent response: {error}"),
        )
        .with_request_id(write_command_context.request_id.clone())
    })?;

    log::info!(
        "change_room success correlation_id={} source={:?} booking_id={} room_id={}",
        effective_correlation_id.value,
        effective_correlation_id.source,
        booking.id,
        booking.room_id
    );

    emit_db_update(&app, "rooms");

    Ok(booking)
}

// ─── Housekeeping Commands ───

#[tauri::command]
pub async fn get_housekeeping_tasks(
    state: State<'_, AppState>,
) -> Result<Vec<HousekeepingTask>, String> {
    housekeeping_service::list_open_tasks(&state.db).await
}

#[tauri::command]
pub async fn update_housekeeping(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    task_id: String,
    new_status: String,
    note: Option<String>,
) -> Result<(), String> {
    housekeeping_service::update_status(&state.db, &task_id, &new_status, note.as_deref())
        .await
        .map_err(|error| format!("{}: {}", error.code, error.message))?;

    emit_db_update(&app, "housekeeping");

    Ok(())
}

// ─── Expense Commands ───

#[tauri::command]
pub async fn create_expense(
    state: State<'_, AppState>,
    req: CreateExpenseRequest,
) -> Result<Expense, String> {
    validate_create_expense_request(&req)?;

    let expense = Expense {
        id: uuid::Uuid::new_v4().to_string(),
        category: req.category,
        amount: req.amount,
        note: req.note,
        expense_date: req.expense_date,
        created_at: chrono::Local::now().to_rfc3339(),
    };

    expense_repository::insert_expense(&state.db, &expense)
        .await
        .map_err(|e| e.to_string())?;

    Ok(expense)
}

fn validate_create_expense_request(req: &CreateExpenseRequest) -> Result<(), String> {
    validate_non_negative_money_vnd(req.amount, "amount")
        .map(|_| ())
        .map_err(|error| error.message)
}

#[tauri::command]
pub async fn get_expenses(
    state: State<'_, AppState>,
    from: String,
    to: String,
) -> Result<Vec<Expense>, String> {
    expense_queries::load_expenses_between(&state.db, &from, &to)
        .await
        .map_err(|e| e.to_string())
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
    let info = stay_info_queries::load_stay_info(&state.db, &booking_id)
        .await
        .map_err(|e| e.to_string())?;

    Ok(format!(
        "Họ và tên: {}\nSố CCCD: {}\nNgày sinh: {}\nGiới tính: {}\nQuốc tịch: {}\nĐịa chỉ: {}\nPhòng: {}\nNgày đến: {}\nNgày đi: {}",
        info.full_name,
        info.doc_number,
        info.dob,
        info.gender,
        info.nationality,
        info.address,
        info.room_id,
        info.check_in,
        info.checkout
    ))
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
        check_in_failure_context, check_out_failure_context, should_request_checkout_backup,
        validate_create_expense_request,
    };
    use crate::app_error::{
        codes, correlation_context, log_system_error, record_command_failure_with_db_group,
    };
    use crate::db_error_monitoring::{
        classify_db_failure, inject_db_error_group, DbErrorGroup, MonitoredDbFailure,
    };
    use crate::models::{
        CheckInRequest, CheckOutRequest, CheckoutSettlementMode, CreateExpenseRequest,
        CreateGuestRequest,
    };
    use serde_json::json;
    use std::fs;

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
    fn create_expense_rejects_negative_amount() {
        let error = validate_create_expense_request(&CreateExpenseRequest {
            category: "supplies".to_string(),
            amount: -1,
            note: None,
            expense_date: "2026-04-30".to_string(),
        })
        .expect_err("negative amount must fail");

        assert!(error.contains("amount"));
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
        // Mirrors what a failing check_out produces: a system error carrying a
        // support id, tagged with the group the read failure classified into.
        let db_error_group =
            classify_db_failure(MonitoredDbFailure::DatabaseRead("disk I/O failure"));
        let error = log_system_error(
            "check_out",
            "disk I/O failure",
            inject_db_error_group(
                correlation_context("COR-1A2B3C4D", context.clone()),
                db_error_group,
            ),
        );
        let support_id = error.support_id.clone().expect("system error support id");
        record_command_failure_with_db_group(
            "check_out",
            &error,
            "COR-1A2B3C4D",
            Some(db_error_group),
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
        assert_eq!(db_error_group, DbErrorGroup::Unknown);

        let _ = fs::remove_dir_all(&runtime_root);
    }

    #[test]
    fn should_request_checkout_backup_skips_replayed_idempotency_response() {
        assert!(should_request_checkout_backup(false));
        assert!(!should_request_checkout_backup(true));
    }
}
