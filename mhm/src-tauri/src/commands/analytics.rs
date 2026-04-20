use super::AppState;
use crate::{models::*, queries::booking::revenue_queries};
use sqlx::Row;
use tauri::State;

// ─── A3: Get Analytics ───

#[tauri::command]
pub async fn get_analytics(
    state: State<'_, AppState>,
    period: String,
) -> Result<AnalyticsData, String> {
    let now = chrono::Local::now();
    let days = match period.as_str() {
        "30d" => 30_i64,
        "90d" => 90_i64,
        _ => 7_i64,
    };
    let from = (now - chrono::Duration::days(days))
        .format("%Y-%m-%d")
        .to_string();
    let to = now.format("%Y-%m-%d").to_string();

    revenue_queries::load_analytics(&state.db, &from, &to, days)
        .await
        .map_err(|e| e.to_string())
}

// ─── A4: Get Recent Activity (Dashboard) ───

#[tauri::command]
pub async fn get_recent_activity(
    state: State<'_, AppState>,
    limit: i32,
) -> Result<Vec<ActivityItem>, String> {
    let mut activities: Vec<ActivityItem> = vec![];

    // Recent check-ins
    let checkins = sqlx::query(
        "SELECT b.room_id, g.full_name, b.check_in_at
         FROM bookings b JOIN guests g ON g.id = b.primary_guest_id
         ORDER BY b.check_in_at DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    for r in &checkins {
        let room_id: String = r.get("room_id");
        let name: String = r.get("full_name");
        let time_str: String = r.get("check_in_at");
        let time = extract_time(&time_str);
        activities.push(ActivityItem {
            icon: "🟢".to_string(),
            text: format!("Check-in {} → {}", name, room_id),
            time,
            color: "bg-emerald-50".to_string(),
            kind: "check_in".to_string(),
            room_id: Some(room_id),
            guest_name: Some(name),
            occurred_at: time_str,
            status_label: "Đã check-in".to_string(),
        });
    }

    // Recent check-outs
    let checkouts = sqlx::query(
        "SELECT b.room_id, g.full_name, b.actual_checkout
         FROM bookings b JOIN guests g ON g.id = b.primary_guest_id
         WHERE b.actual_checkout IS NOT NULL
         ORDER BY b.actual_checkout DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    for r in &checkouts {
        let room_id: String = r.get("room_id");
        let name: String = r.get("full_name");
        let time_str: String = r.get("actual_checkout");
        let time = extract_time(&time_str);
        activities.push(ActivityItem {
            icon: "🔴".to_string(),
            text: format!("Check-out {} — Room {}", name, room_id),
            time,
            color: "bg-red-50".to_string(),
            kind: "check_out".to_string(),
            room_id: Some(room_id),
            guest_name: Some(name),
            occurred_at: time_str,
            status_label: "Đã check-out".to_string(),
        });
    }

    // Recent housekeeping
    let hk = sqlx::query(
        "SELECT room_id, status, triggered_at FROM housekeeping
         ORDER BY triggered_at DESC LIMIT ?",
    )
    .bind(limit)
    .fetch_all(&state.db)
    .await
    .map_err(|e| e.to_string())?;

    for r in &hk {
        let room_id: String = r.get("room_id");
        let status: String = r.get("status");
        let time_str: String = r.get("triggered_at");
        let time = extract_time(&time_str);
        let label = if status == "clean" {
            "Cleaned"
        } else {
            "Needs cleaning"
        };
        activities.push(ActivityItem {
            icon: "🧹".to_string(),
            text: format!("{} — Room {}", label, room_id),
            time,
            color: "bg-amber-50".to_string(),
            kind: "housekeeping".to_string(),
            room_id: Some(room_id),
            guest_name: None,
            occurred_at: time_str,
            status_label: label.to_string(),
        });
    }

    // Sort by full timestamp descending to keep cross-day ordering stable.
    activities.sort_by(|a, b| b.occurred_at.cmp(&a.occurred_at));
    activities.truncate(limit as usize);

    Ok(activities)
}

fn extract_time(datetime_str: &str) -> String {
    // Extract HH:MM from ISO datetime or RFC3339
    if let Some(t_pos) = datetime_str.find('T') {
        let time_part = &datetime_str[t_pos + 1..];
        if time_part.len() >= 5 {
            return time_part[..5].to_string();
        }
    }
    // Fallback: try space separator
    if let Some(parts) = datetime_str.split(' ').nth(1) {
        if parts.len() >= 5 {
            return parts[..5].to_string();
        }
    }
    datetime_str.to_string()
}
