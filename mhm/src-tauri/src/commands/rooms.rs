use super::{emit_db_update, get_f64, get_user_id, AppState};
use crate::{models::*, queries::booking::revenue_queries, services::booking::stay_lifecycle};
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

#[tauri::command]
pub async fn check_in(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: CheckInRequest,
) -> Result<Booking, String> {
    let booking = stay_lifecycle::check_in(&state.db, req, get_user_id(&state))
        .await
        .map_err(|error| error.to_string())?;

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
) -> Result<(), String> {
    stay_lifecycle::check_out(&state.db, req)
        .await
        .map_err(|error| error.to_string())?;

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
