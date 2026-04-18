use super::{emit_db_update, require_admin, AppState};
use crate::app_identity;
use crate::{queries::booking::audit_queries, services::booking::audit_service};
use tauri::State;

// ═══════════════════════════════════════════════
// Phase 4: Night Audit Commands
// ═══════════════════════════════════════════════

#[tauri::command]
pub async fn run_night_audit(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    audit_date: String,
    notes: Option<String>,
) -> Result<crate::models::AuditLog, String> {
    let user = require_admin(&state)?;
    let log = audit_service::run_night_audit(&state.db, &audit_date, notes, &user.id)
        .await
        .map_err(|error| error.to_string())?;

    emit_db_update(&app, "audit");

    if let Err(error) =
        crate::backup::request_backup(&app, crate::backup::BackupReason::NightAudit).await
    {
        log::error!("autobackup failed after night audit: {}", error);
    }

    Ok(log)
}

#[tauri::command]
pub async fn get_audit_logs(
    state: State<'_, AppState>,
) -> Result<Vec<crate::models::AuditLog>, String> {
    audit_queries::list_audit_logs(&state.db)
        .await
        .map_err(|e| e.to_string())
}

// ═══════════════════════════════════════════════
// Phase 5: Backup & Data Export
// ═══════════════════════════════════════════════

#[tauri::command]
pub async fn backup_database(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<String, String> {
    require_admin(&state)?;
    let outcome = crate::backup::request_backup(&app, crate::backup::BackupReason::Manual).await?;
    Ok(outcome.path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn export_bookings_csv(
    state: State<'_, AppState>,
    from_date: Option<String>,
    to_date: Option<String>,
) -> Result<String, String> {
    require_admin(&state)?;

    let from = from_date.unwrap_or_else(|| "2000-01-01".to_string());
    let to = to_date.unwrap_or_else(|| "2099-12-31".to_string());

    let rows = audit_queries::load_booking_export_rows(&state.db, &from, &to)
        .await
        .map_err(|e| e.to_string())?;

    let mut csv = String::from(
        "ID,Room,Guest,DocNumber,Phone,CheckIn,CheckOut,ActualCheckout,Nights,RoomPrice,ChargeTotal,CancellationFeeTotal,FolioTotal,RecognizedRevenue,PaidAmount,Status,PricingType,Source\n",
    );

    for r in &rows {
        csv.push_str(&format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}\n",
            r.id,
            r.room_id,
            r.guest_name.replace(',', " "),
            r.doc_number,
            r.phone,
            r.check_in_at,
            r.expected_checkout,
            r.actual_checkout,
            r.nights,
            r.room_price,
            r.charge_total,
            r.cancellation_fee_total,
            r.folio_total,
            r.recognized_revenue,
            r.paid_amount,
            r.status,
            r.pricing_type,
            r.source,
        ));
    }

    // Save to file
    let export_dir = app_identity::exports_dir_opt().ok_or("Cannot find home directory")?;
    std::fs::create_dir_all(&export_dir).map_err(|e| e.to_string())?;

    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let file_path = export_dir.join(format!("bookings_{}.csv", timestamp));

    std::fs::write(&file_path, &csv).map_err(|e| e.to_string())?;

    Ok(file_path.to_string_lossy().to_string())
}
