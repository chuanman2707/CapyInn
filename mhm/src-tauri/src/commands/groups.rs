use super::{emit_db_update, get_f64, get_user_id, AppState};
use crate::models::*;
use crate::services::booking::group_lifecycle;
use sqlx::{Pool, Row, Sqlite};
use tauri::State;

// ─── Group Booking Commands ───

/// Check-in a group: creates booking_groups + N bookings atomically
#[tauri::command]
pub async fn group_checkin(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: GroupCheckinRequest,
) -> Result<BookingGroup, String> {
    let group = group_lifecycle::group_checkin(&state.db, get_user_id(&state), req)
        .await
        .map_err(|error| error.to_string())?;
    emit_db_update(&app, "rooms");
    emit_db_update(&app, "groups");
    Ok(group)
}

/// Checkout subset of group rooms
#[tauri::command]
pub async fn group_checkout(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: GroupCheckoutRequest,
) -> Result<(), String> {
    group_lifecycle::group_checkout(&state.db, req)
        .await
        .map_err(|error| error.to_string())?;
    emit_db_update(&app, "rooms");
    emit_db_update(&app, "groups");

    if let Err(error) =
        crate::backup::request_backup(&app, crate::backup::BackupReason::GroupCheckout).await
    {
        crate::backup::log_backup_request_error("group_checkout", &error);
    }

    Ok(())
}

/// Get group detail with bookings, services, totals
pub async fn do_get_group_detail(
    pool: &Pool<Sqlite>,
    group_id: &str,
) -> Result<GroupDetailResponse, String> {
    let row = sqlx::query("SELECT * FROM booking_groups WHERE id = ?")
        .bind(group_id)
        .fetch_one(pool)
        .await
        .map_err(|e| e.to_string())?;

    let group = BookingGroup {
        id: row.get("id"),
        group_name: row.get("group_name"),
        master_booking_id: row.get("master_booking_id"),
        organizer_name: row.get("organizer_name"),
        organizer_phone: row.get("organizer_phone"),
        total_rooms: row.get("total_rooms"),
        status: row.get("status"),
        notes: row.get("notes"),
        created_by: row.get("created_by"),
        created_at: row.get("created_at"),
    };

    // Get bookings
    let booking_rows = sqlx::query(
        "SELECT b.id, b.room_id, r.name as room_name, g.full_name as guest_name,
                b.check_in_at, b.expected_checkout, b.actual_checkout, b.nights,
                b.total_price, b.paid_amount, b.status, b.source,
                b.booking_type, b.deposit_amount, b.scheduled_checkin, b.scheduled_checkout, b.guest_phone
         FROM bookings b
         JOIN rooms r ON r.id = b.room_id
         JOIN guests g ON g.id = b.primary_guest_id
         WHERE b.group_id = ?
         ORDER BY r.floor, r.id"
    )
    .bind(group_id)
    .fetch_all(pool).await.map_err(|e| e.to_string())?;

    let bookings: Vec<BookingWithGuest> = booking_rows
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
        .collect();

    // Get services
    let service_rows =
        sqlx::query("SELECT * FROM group_services WHERE group_id = ? ORDER BY created_at")
            .bind(group_id)
            .fetch_all(pool)
            .await
            .map_err(|e| e.to_string())?;

    let services: Vec<GroupService> = service_rows
        .iter()
        .map(|r| GroupService {
            id: r.get("id"),
            group_id: r.get("group_id"),
            booking_id: r.get("booking_id"),
            name: r.get("name"),
            quantity: r.get("quantity"),
            unit_price: get_f64(r, "unit_price"),
            total_price: get_f64(r, "total_price"),
            note: r.get("note"),
            created_by: r.get("created_by"),
            created_at: r.get("created_at"),
        })
        .collect();

    let total_room_cost: f64 = bookings.iter().map(|b| b.total_price).sum();
    let total_service_cost: f64 = services.iter().map(|s| s.total_price).sum();
    let paid_amount: f64 = bookings.iter().map(|b| b.paid_amount).sum();

    Ok(GroupDetailResponse {
        group,
        bookings,
        services,
        total_room_cost,
        total_service_cost,
        grand_total: total_room_cost + total_service_cost,
        paid_amount,
    })
}

#[tauri::command]
pub async fn get_group_detail(
    state: State<'_, AppState>,
    group_id: String,
) -> Result<GroupDetailResponse, String> {
    do_get_group_detail(&state.db, &group_id).await
}

/// List all groups, optionally filtered by status
#[tauri::command]
pub async fn get_all_groups(
    state: State<'_, AppState>,
    status: Option<String>,
) -> Result<Vec<BookingGroup>, String> {
    let rows = if let Some(ref s) = status {
        sqlx::query("SELECT * FROM booking_groups WHERE status = ? ORDER BY created_at DESC")
            .bind(s)
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())?
    } else {
        sqlx::query("SELECT * FROM booking_groups ORDER BY created_at DESC")
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())?
    };

    Ok(rows
        .iter()
        .map(|r| BookingGroup {
            id: r.get("id"),
            group_name: r.get("group_name"),
            master_booking_id: r.get("master_booking_id"),
            organizer_name: r.get("organizer_name"),
            organizer_phone: r.get("organizer_phone"),
            total_rooms: r.get("total_rooms"),
            status: r.get("status"),
            notes: r.get("notes"),
            created_by: r.get("created_by"),
            created_at: r.get("created_at"),
        })
        .collect())
}

/// Add a group service (laundry, tour, motorbike, etc.)
#[tauri::command]
pub async fn add_group_service(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    req: AddGroupServiceRequest,
) -> Result<GroupService, String> {
    let user_id = get_user_id(&state);
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Local::now().to_rfc3339();
    let total_price = req.quantity as f64 * req.unit_price;

    sqlx::query(
        "INSERT INTO group_services (id, group_id, booking_id, name, quantity, unit_price, total_price, note, created_by, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
    )
    .bind(&id)
    .bind(&req.group_id)
    .bind(&req.booking_id)
    .bind(&req.name)
    .bind(req.quantity)
    .bind(req.unit_price)
    .bind(total_price)
    .bind(&req.note)
    .bind(&user_id)
    .bind(&now)
    .execute(&state.db).await.map_err(|e| e.to_string())?;

    emit_db_update(&app, "groups");

    Ok(GroupService {
        id,
        group_id: req.group_id,
        booking_id: req.booking_id,
        name: req.name,
        quantity: req.quantity,
        unit_price: req.unit_price,
        total_price,
        note: req.note,
        created_by: user_id,
        created_at: now,
    })
}

/// Remove a group service
#[tauri::command]
pub async fn remove_group_service(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    service_id: String,
) -> Result<(), String> {
    sqlx::query("DELETE FROM group_services WHERE id = ?")
        .bind(&service_id)
        .execute(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    emit_db_update(&app, "groups");
    Ok(())
}

/// Auto-assign rooms: prefer same floor, greedy fill
#[tauri::command]
pub async fn auto_assign_rooms(
    state: State<'_, AppState>,
    req: AutoAssignRequest,
) -> Result<AutoAssignResult, String> {
    if req.room_count <= 0 {
        return Err("Số phòng phải > 0".to_string());
    }

    let rows = if let Some(ref rt) = req.room_type {
        sqlx::query("SELECT * FROM rooms WHERE status = 'vacant' AND type = ? ORDER BY floor, id")
            .bind(rt)
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())?
    } else {
        sqlx::query("SELECT * FROM rooms WHERE status = 'vacant' ORDER BY floor, id")
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())?
    };

    let vacant_rooms: Vec<Room> = rows
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

    if vacant_rooms.len() < req.room_count as usize {
        return Err(format!(
            "Chỉ có {} phòng trống, cần {} phòng",
            vacant_rooms.len(),
            req.room_count
        ));
    }

    // Group by floor, sort by count descending (greedy fill)
    let mut floor_groups: std::collections::HashMap<i32, Vec<&Room>> =
        std::collections::HashMap::new();
    for room in &vacant_rooms {
        floor_groups.entry(room.floor).or_default().push(room);
    }

    let mut floors_sorted: Vec<(i32, Vec<&Room>)> = floor_groups.into_iter().collect();
    floors_sorted.sort_by_key(|(_, rooms)| std::cmp::Reverse(rooms.len()));

    let mut assignments = Vec::new();
    let needed = req.room_count as usize;

    for (floor, rooms) in &floors_sorted {
        if assignments.len() >= needed {
            break;
        }
        for room in rooms {
            if assignments.len() >= needed {
                break;
            }
            assignments.push(RoomAssignment {
                room: (*room).clone(),
                floor: *floor,
            });
        }
    }

    Ok(AutoAssignResult { assignments })
}

/// Generate group invoice data
pub async fn do_generate_group_invoice(
    pool: &Pool<Sqlite>,
    group_id: &str,
) -> Result<GroupInvoiceData, String> {
    let detail = do_get_group_detail(pool, group_id).await?;

    // Get hotel info from settings
    let hotel_info =
        sqlx::query_as::<_, (String,)>("SELECT value FROM settings WHERE key = 'hotel_info'")
            .fetch_optional(pool)
            .await
            .map_err(|e| e.to_string())?;

    let (hotel_name, hotel_address, hotel_phone) = if let Some((val,)) = hotel_info {
        let parsed: serde_json::Value = serde_json::from_str(&val).unwrap_or_default();
        (
            parsed["name"]
                .as_str()
                .unwrap_or(crate::app_identity::APP_NAME)
                .to_string(),
            parsed["address"].as_str().unwrap_or("").to_string(),
            parsed["phone"].as_str().unwrap_or("").to_string(),
        )
    } else {
        (
            crate::app_identity::APP_NAME.to_string(),
            String::new(),
            String::new(),
        )
    };

    // Build room lines
    let rooms: Vec<GroupInvoiceRoomLine> = detail
        .bookings
        .iter()
        .map(|b| {
            let price_per_night = if b.nights > 0 {
                b.total_price / b.nights as f64
            } else {
                b.total_price
            };
            GroupInvoiceRoomLine {
                room_name: b.room_name.clone(),
                room_type: String::new(), // simplified
                nights: b.nights,
                price_per_night,
                total: b.total_price,
                guest_name: b.guest_name.clone(),
            }
        })
        .collect();

    Ok(GroupInvoiceData {
        group: detail.group,
        rooms,
        services: detail.services,
        subtotal_rooms: detail.total_room_cost,
        subtotal_services: detail.total_service_cost,
        grand_total: detail.grand_total,
        paid_amount: detail.paid_amount,
        balance_due: detail.grand_total - detail.paid_amount,
        hotel_name,
        hotel_address,
        hotel_phone,
    })
}

#[tauri::command]
pub async fn generate_group_invoice(
    state: State<'_, AppState>,
    group_id: String,
) -> Result<GroupInvoiceData, String> {
    do_generate_group_invoice(&state.db, &group_id).await
}
