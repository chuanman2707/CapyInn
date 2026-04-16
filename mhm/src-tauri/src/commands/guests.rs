use super::{get_f64, AppState};
use crate::models::*;
use sqlx::Row;
use tauri::State;

// ─── A2: Get All Guests ───

#[tauri::command]
pub async fn get_all_guests(
    state: State<'_, AppState>,
    search: Option<String>,
) -> Result<Vec<GuestSummary>, String> {
    let sql = if search.is_some() {
        "SELECT g.id, g.full_name, g.doc_number, g.nationality,
                COUNT(bg.booking_id) as total_stays,
                COALESCE(SUM(b.total_price), 0) as total_spent,
                MAX(b.check_in_at) as last_visit
         FROM guests g
         LEFT JOIN booking_guests bg ON bg.guest_id = g.id
         LEFT JOIN bookings b ON b.id = bg.booking_id
         WHERE g.full_name LIKE ? OR g.doc_number LIKE ?
         GROUP BY g.id
         ORDER BY last_visit DESC"
    } else {
        "SELECT g.id, g.full_name, g.doc_number, g.nationality,
                COUNT(bg.booking_id) as total_stays,
                COALESCE(SUM(b.total_price), 0) as total_spent,
                MAX(b.check_in_at) as last_visit
         FROM guests g
         LEFT JOIN booking_guests bg ON bg.guest_id = g.id
         LEFT JOIN bookings b ON b.id = bg.booking_id
         GROUP BY g.id
         ORDER BY last_visit DESC"
    };

    let rows = if let Some(ref s) = search {
        let pattern = format!("%{}%", s);
        sqlx::query(sql)
            .bind(&pattern)
            .bind(&pattern)
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())?
    } else {
        sqlx::query(sql)
            .fetch_all(&state.db)
            .await
            .map_err(|e| e.to_string())?
    };

    Ok(rows
        .iter()
        .map(|r| GuestSummary {
            id: r.get("id"),
            full_name: r.get("full_name"),
            doc_number: r.get("doc_number"),
            nationality: r.get("nationality"),
            total_stays: r.get::<i32, _>("total_stays"),
            total_spent: get_f64(r, "total_spent"),
            last_visit: r.get("last_visit"),
        })
        .collect())
}

// ─── A2: Get Guest History ───

#[tauri::command]
pub async fn get_guest_history(
    state: State<'_, AppState>,
    guest_id: String,
) -> Result<GuestHistoryResponse, String> {
    let row = sqlx::query("SELECT * FROM guests WHERE id = ?")
        .bind(&guest_id)
        .fetch_one(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    let guest = Guest {
        id: row.get("id"),
        guest_type: row.get("guest_type"),
        full_name: row.get("full_name"),
        doc_number: row.get("doc_number"),
        dob: row.get("dob"),
        gender: row.get("gender"),
        nationality: row.get("nationality"),
        address: row.get("address"),
        visa_expiry: row.get("visa_expiry"),
        scan_path: row.get("scan_path"),
        phone: row.get("phone"),
        created_at: row.get("created_at"),
    };

    let booking_rows = sqlx::query(
        "SELECT b.id as booking_id, b.room_id, b.check_in_at, b.expected_checkout, b.total_price, b.status
         FROM bookings b
         JOIN booking_guests bg ON bg.booking_id = b.id
         WHERE bg.guest_id = ?
         ORDER BY b.check_in_at DESC"
    )
    .bind(&guest_id)
    .fetch_all(&state.db).await.map_err(|e| e.to_string())?;

    let bookings = booking_rows
        .iter()
        .map(|r| BookingWithRoom {
            booking_id: r.get("booking_id"),
            room_id: r.get("room_id"),
            check_in_at: r.get("check_in_at"),
            expected_checkout: r.get("expected_checkout"),
            total_price: get_f64(r, "total_price"),
            status: r.get("status"),
        })
        .collect();

    Ok(GuestHistoryResponse { guest, bookings })
}
