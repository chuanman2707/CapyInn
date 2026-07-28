//! Pricing configuration and price-preview commands.
//!
//! Boundary adapters only: check the caller is an admin, shape the JSON the
//! settings screen expects, and delegate. The preview shares
//! `services::booking::pricing_service` with the lifecycle charge, so the quote
//! the UI shows and the amount the guest is billed come from one implementation.

use super::{emit_db_update, require_admin, AppState};
use crate::money::MoneyVnd;
use crate::queries::booking::pricing_queries;
use crate::repositories::booking::pricing_repository;
use crate::services::booking::pricing_service::{self, SavePricingRule};
use sqlx::{Pool, Sqlite};
use tauri::State;

// ═══════════════════════════════════════════════
// Phase 2: Pricing Engine Commands
// ═══════════════════════════════════════════════

pub async fn do_get_pricing_rules(pool: &Pool<Sqlite>) -> Result<Vec<serde_json::Value>, String> {
    let listings = pricing_queries::load_pricing_rule_listings(pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(listings
        .iter()
        .map(|listing| {
            serde_json::json!({
                "id": listing.id,
                "room_type": listing.rule.room_type,
                "hourly_rate": listing.rule.hourly_rate,
                "overnight_rate": listing.rule.overnight_rate,
                "daily_rate": listing.rule.daily_rate,
                "overnight_start": listing.rule.overnight_start,
                "overnight_end": listing.rule.overnight_end,
                "daily_checkin": listing.rule.daily_checkin,
                "daily_checkout": listing.rule.daily_checkout,
                "early_checkin_surcharge_pct": listing.rule.early_checkin_surcharge_pct,
                "late_checkout_surcharge_pct": listing.rule.late_checkout_surcharge_pct,
                "weekend_uplift_pct": listing.rule.weekend_uplift_pct,
            })
        })
        .collect())
}

#[tauri::command]
pub async fn get_pricing_rules(
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    do_get_pricing_rules(&state.db).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn save_pricing_rule(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    room_type: String,
    hourly_rate: MoneyVnd,
    overnight_rate: MoneyVnd,
    daily_rate: MoneyVnd,
    overnight_start: Option<String>,
    overnight_end: Option<String>,
    daily_checkin: Option<String>,
    daily_checkout: Option<String>,
    early_pct: Option<f64>,
    late_pct: Option<f64>,
    weekend_pct: Option<f64>,
) -> Result<(), String> {
    require_admin(&state)?;

    pricing_service::save_pricing_rule(
        &state.db,
        SavePricingRule {
            room_type,
            hourly_rate,
            overnight_rate,
            daily_rate,
            overnight_start,
            overnight_end,
            daily_checkin,
            daily_checkout,
            early_pct,
            late_pct,
            weekend_pct,
        },
        uuid::Uuid::new_v4().to_string(),
        chrono::Local::now().to_rfc3339(),
    )
    .await?;

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
    pricing_service::calculate_price_preview(
        pool,
        room_type,
        check_in,
        check_out,
        pricing_type,
        None,
    )
    .await
    .map_err(|error| error.to_string())
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

#[tauri::command]
pub async fn calculate_room_price_preview(
    state: State<'_, AppState>,
    room_id: String,
    check_in: String,
    check_out: String,
    pricing_type: String,
    guests: Option<i32>,
) -> Result<crate::pricing::PricingResult, String> {
    pricing_service::calculate_room_price_preview(
        &state.db,
        &room_id,
        &check_in,
        &check_out,
        &pricing_type,
        guests,
    )
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn get_special_dates(
    state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let dates = pricing_queries::load_special_dates(&state.db)
        .await
        .map_err(|e| e.to_string())?;

    Ok(dates
        .iter()
        .map(|date| {
            serde_json::json!({
                "id": date.id,
                "date": date.date,
                "label": date.label,
                "uplift_pct": date.uplift_pct,
            })
        })
        .collect())
}

#[tauri::command]
pub async fn save_special_date(
    state: State<'_, AppState>,
    date: String,
    label: String,
    uplift_pct: f64,
) -> Result<(), String> {
    require_admin(&state)?;

    pricing_repository::upsert_special_date(
        &state.db,
        &uuid::Uuid::new_v4().to_string(),
        &date,
        &label,
        uplift_pct,
        &chrono::Local::now().to_rfc3339(),
    )
    .await
    .map_err(|e| e.to_string())
}
