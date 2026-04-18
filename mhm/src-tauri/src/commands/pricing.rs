use sqlx::{Pool, Sqlite, Row};
use tauri::State;
use super::{AppState, get_f64, emit_db_update, require_admin};


// ═══════════════════════════════════════════════
// Phase 2: Pricing Engine Commands
// ═══════════════════════════════════════════════

pub async fn do_get_pricing_rules(pool: &Pool<Sqlite>) -> Result<Vec<serde_json::Value>, String> {
    let rows = sqlx::query(
        "SELECT id, room_type, hourly_rate, overnight_rate, daily_rate,
                overnight_start, overnight_end, daily_checkin, daily_checkout,
                early_checkin_surcharge_pct, late_checkout_surcharge_pct,
                weekend_uplift_pct
         FROM pricing_rules ORDER BY room_type"
    ).fetch_all(pool).await.map_err(|e| e.to_string())?;

    Ok(rows.iter().map(|r| serde_json::json!({
        "id": r.get::<String, _>("id"),
        "room_type": r.get::<String, _>("room_type"),
        "hourly_rate": get_f64(r, "hourly_rate"),
        "overnight_rate": get_f64(r, "overnight_rate"),
        "daily_rate": get_f64(r, "daily_rate"),
        "overnight_start": r.get::<String, _>("overnight_start"),
        "overnight_end": r.get::<String, _>("overnight_end"),
        "daily_checkin": r.get::<String, _>("daily_checkin"),
        "daily_checkout": r.get::<String, _>("daily_checkout"),
        "early_checkin_surcharge_pct": get_f64(r, "early_checkin_surcharge_pct"),
        "late_checkout_surcharge_pct": get_f64(r, "late_checkout_surcharge_pct"),
        "weekend_uplift_pct": get_f64(r, "weekend_uplift_pct"),
    })).collect())
}

#[tauri::command]
pub async fn get_pricing_rules(state: State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    do_get_pricing_rules(&state.db).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn save_pricing_rule(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    room_type: String,
    hourly_rate: f64,
    overnight_rate: f64,
    daily_rate: f64,
    overnight_start: Option<String>,
    overnight_end: Option<String>,
    daily_checkin: Option<String>,
    daily_checkout: Option<String>,
    early_pct: Option<f64>,
    late_pct: Option<f64>,
    weekend_pct: Option<f64>,
) -> Result<(), String> {
    require_admin(&state)?;

    let now = chrono::Local::now().to_rfc3339();
    let id = uuid::Uuid::new_v4().to_string();

    sqlx::query(
        "INSERT INTO pricing_rules
         (id, room_type, hourly_rate, overnight_rate, daily_rate,
          overnight_start, overnight_end, daily_checkin, daily_checkout,
          early_checkin_surcharge_pct, late_checkout_surcharge_pct,
          weekend_uplift_pct, created_at, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT(room_type) DO UPDATE SET
            hourly_rate = excluded.hourly_rate,
            overnight_rate = excluded.overnight_rate,
            daily_rate = excluded.daily_rate,
            overnight_start = excluded.overnight_start,
            overnight_end = excluded.overnight_end,
            daily_checkin = excluded.daily_checkin,
            daily_checkout = excluded.daily_checkout,
            early_checkin_surcharge_pct = excluded.early_checkin_surcharge_pct,
            late_checkout_surcharge_pct = excluded.late_checkout_surcharge_pct,
            weekend_uplift_pct = excluded.weekend_uplift_pct,
            updated_at = excluded.updated_at"
    )
    .bind(&id)
    .bind(&room_type)
    .bind(hourly_rate)
    .bind(overnight_rate)
    .bind(daily_rate)
    .bind(overnight_start.as_deref().unwrap_or("22:00"))
    .bind(overnight_end.as_deref().unwrap_or("11:00"))
    .bind(daily_checkin.as_deref().unwrap_or("14:00"))
    .bind(daily_checkout.as_deref().unwrap_or("12:00"))
    .bind(early_pct.unwrap_or(30.0))
    .bind(late_pct.unwrap_or(30.0))
    .bind(weekend_pct.unwrap_or(0.0))
    .bind(&now)
    .bind(&now)
    .execute(&state.db).await.map_err(|e| e.to_string())?;

    emit_db_update(&app, "pricing");
    Ok(())
}

pub async fn do_calculate_price_preview(
    pool: &Pool<Sqlite>,
    room_type: &str,
    check_in: &str,
    check_out: &str,
    pricing_type: &str,
) -> Result<crate::pricing::PricingResult, String> {
    let room_type_lower = room_type.to_lowercase();
    let row = sqlx::query(
        "SELECT room_type, hourly_rate, overnight_rate, daily_rate,
                overnight_start, overnight_end, daily_checkin, daily_checkout,
                early_checkin_surcharge_pct, late_checkout_surcharge_pct,
                weekend_uplift_pct
         FROM pricing_rules WHERE LOWER(room_type) = ?"
    )
    .bind(&room_type_lower)
    .fetch_optional(pool).await.map_err(|e| e.to_string())?;

    let rule = match row {
        Some(r) => crate::pricing::PricingRule {
            room_type: r.get("room_type"),
            hourly_rate: get_f64(&r, "hourly_rate"),
            overnight_rate: get_f64(&r, "overnight_rate"),
            daily_rate: get_f64(&r, "daily_rate"),
            overnight_start: r.get("overnight_start"),
            overnight_end: r.get("overnight_end"),
            daily_checkin: r.get("daily_checkin"),
            daily_checkout: r.get("daily_checkout"),
            early_checkin_surcharge_pct: get_f64(&r, "early_checkin_surcharge_pct"),
            late_checkout_surcharge_pct: get_f64(&r, "late_checkout_surcharge_pct"),
            weekend_uplift_pct: get_f64(&r, "weekend_uplift_pct"),
        },
        None => {
            let fallback_row = sqlx::query(
                "SELECT base_price FROM rooms WHERE LOWER(type) = ? LIMIT 1"
            ).bind(&room_type_lower).fetch_optional(pool).await.map_err(|e| e.to_string())?;
            let fallback_price = fallback_row.as_ref().map(|r| get_f64(r, "base_price")).unwrap_or(350_000.0);

            crate::pricing::PricingRule {
                room_type: room_type.to_string(),
                hourly_rate: fallback_price / 5.0,
                overnight_rate: fallback_price * 0.75,
                daily_rate: fallback_price,
                ..Default::default()
            }
        }
    };

    let special_uplift = do_get_special_uplift(pool, check_in).await;

    crate::pricing::calculate_price(&rule, check_in, check_out, pricing_type, special_uplift)
}

#[tauri::command]
pub async fn calculate_price_preview(
    state: State<'_, AppState>,
    room_type: String,
    check_in: String,
    check_out: String,
    pricing_type: String,
) -> Result<crate::pricing::PricingResult, String> {
    do_calculate_price_preview(&state.db, &room_type, &check_in, &check_out, &pricing_type).await
}

pub async fn do_get_special_uplift(pool: &Pool<Sqlite>, date_str: &str) -> f64 {
    let date = if date_str.len() >= 10 { &date_str[..10] } else { date_str };
    let row: Option<(f64,)> = sqlx::query_as(
        "SELECT CAST(uplift_pct AS REAL) FROM special_dates WHERE date = ?"
    ).bind(date).fetch_optional(pool).await.ok().flatten();
    row.map(|r| r.0).unwrap_or(0.0)
}



#[tauri::command]
pub async fn get_special_dates(state: State<'_, AppState>) -> Result<Vec<serde_json::Value>, String> {
    let rows = sqlx::query("SELECT id, date, label, uplift_pct FROM special_dates ORDER BY date")
        .fetch_all(&state.db).await.map_err(|e| e.to_string())?;

    Ok(rows.iter().map(|r| serde_json::json!({
        "id": r.get::<String, _>("id"),
        "date": r.get::<String, _>("date"),
        "label": r.get::<String, _>("label"),
        "uplift_pct": get_f64(r, "uplift_pct"),
    })).collect())
}

#[tauri::command]
pub async fn save_special_date(
    state: State<'_, AppState>,
    date: String,
    label: String,
    uplift_pct: f64,
) -> Result<(), String> {
    require_admin(&state)?;

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Local::now().to_rfc3339();

    sqlx::query(
        "INSERT INTO special_dates (id, date, label, uplift_pct, created_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(date) DO UPDATE SET
            label = excluded.label,
            uplift_pct = excluded.uplift_pct"
    )
    .bind(&id).bind(&date).bind(&label).bind(uplift_pct).bind(&now)
    .execute(&state.db).await.map_err(|e| e.to_string())?;

    Ok(())
}
