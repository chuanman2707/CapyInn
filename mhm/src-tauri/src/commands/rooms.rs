use super::{emit_db_update, get_user_id, AppState};
use crate::db::row::get_money_vnd;
use crate::{
    app_error::{
        codes, log_system_error, normalize_correlation_id, record_command_failure_with_db_group,
        CommandError, CommandResult,
    },
    command_idempotency::WriteCommandContext,
    models::*,
    money::validate_non_negative_money_vnd,
    queries::booking::{revenue_queries, room_queries},
    services::booking::stay_lifecycle,
};
use serde_json::{json, Value};
use sqlx::{Pool, Row, Sqlite};
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
    validate_create_expense_request(&req)?;

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
        should_request_checkout_backup, validate_create_expense_request,
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
        .bind(100000)
        .bind(2)
        .bind(0)
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
