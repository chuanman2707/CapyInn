use super::{emit_db_update, require_admin, AppState};
use crate::app_identity;
use crate::{
    app_error::{
        codes, correlation_context, log_system_error, normalize_correlation_id, CommandError,
        CommandResult, EffectiveCorrelationId,
    },
    domain::booking::BookingError,
    queries::booking::audit_queries,
    services::booking::audit_service,
};
use serde_json::{json, Value};
use tauri::State;

// ═══════════════════════════════════════════════
// Phase 4: Night Audit Commands
// ═══════════════════════════════════════════════

fn log_user_audit_error(
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

fn map_audit_user_error(
    code: &'static str,
    command_name: &str,
    effective_correlation_id: &EffectiveCorrelationId,
    message: String,
    context: &Value,
) -> CommandError {
    log_user_audit_error(
        command_name,
        effective_correlation_id,
        message.as_str(),
        context,
    );
    CommandError::user(code, message)
}

fn map_audit_error(
    command_name: &str,
    effective_correlation_id: &EffectiveCorrelationId,
    error: BookingError,
    context: Value,
) -> CommandError {
    match error {
        BookingError::Validation(message) if message == "Ngày audit không hợp lệ" => {
            map_audit_user_error(
                codes::AUDIT_INVALID_DATE,
                command_name,
                effective_correlation_id,
                message,
                &context,
            )
        }
        BookingError::Validation(message) | BookingError::Conflict(message)
            if message.starts_with("Đã audit ngày ") =>
        {
            map_audit_user_error(
                codes::AUDIT_DATE_ALREADY_RUN,
                command_name,
                effective_correlation_id,
                message,
                &context,
            )
        }
        BookingError::Validation(message) | BookingError::Conflict(message) => {
            map_audit_user_error(
                codes::AUDIT_INVALID_DATE,
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
pub async fn run_night_audit(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    audit_date: String,
    notes: Option<String>,
    correlation_id: Option<String>,
) -> CommandResult<crate::models::AuditLog> {
    let effective_correlation_id = normalize_correlation_id(correlation_id);
    let notes_present = notes
        .as_ref()
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    let error_context = json!({
        "audit_date": audit_date.clone(),
        "notes_present": notes_present,
    });
    log::info!(
        "run_night_audit start correlation_id={} source={:?} audit_date={} notes_present={}",
        effective_correlation_id.value,
        effective_correlation_id.source,
        audit_date,
        notes_present
    );
    let user = require_admin(&state)?;
    let log = audit_service::run_night_audit(&state.db, &audit_date, notes, &user.id)
        .await
        .map_err(|error| {
            map_audit_error(
                "run_night_audit",
                &effective_correlation_id,
                error,
                error_context,
            )
        })?;

    log::info!(
        "run_night_audit success correlation_id={} source={:?} audit_log_id={} audit_date={}",
        effective_correlation_id.value,
        effective_correlation_id.source,
        log.id,
        log.audit_date
    );

    emit_db_update(&app, "audit");

    if let Err(error) =
        crate::backup::request_backup(&app, crate::backup::BackupReason::NightAudit).await
    {
        crate::backup::log_backup_request_error("night audit", &error);
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
    let outcome = crate::backup::request_backup(&app, crate::backup::BackupReason::Manual)
        .await
        .map_err(|error| {
            crate::backup::log_backup_request_error("manual backup", &error);
            error.to_string()
        })?;
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

#[cfg(test)]
mod tests {
    use super::map_audit_error;
    use crate::app_error::{
        codes, AppErrorKind, CorrelationIdSource, EffectiveCorrelationId,
    };
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
    fn map_audit_error_maps_invalid_date_to_shared_code() {
        let error = map_audit_error(
            "run_night_audit",
            &frontend_correlation_id(),
            BookingError::validation("Ngày audit không hợp lệ"),
            json!({ "audit_date": "2026-02-30" }),
        );

        assert_eq!(error.code, codes::AUDIT_INVALID_DATE);
        assert_eq!(error.message, "Ngày audit không hợp lệ");
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
    }

    #[test]
    fn map_audit_error_maps_duplicate_audit_to_shared_code() {
        let error = map_audit_error(
            "run_night_audit",
            &frontend_correlation_id(),
            BookingError::validation("Đã audit ngày 2026-04-20 rồi!"),
            json!({ "audit_date": "2026-04-20" }),
        );

        assert_eq!(error.code, codes::AUDIT_DATE_ALREADY_RUN);
        assert_eq!(error.message, "Đã audit ngày 2026-04-20 rồi!");
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
    }

    #[test]
    fn map_audit_error_keeps_system_failures_in_system_contract() {
        let error = map_audit_error(
            "run_night_audit",
            &frontend_correlation_id(),
            BookingError::database("disk I/O failure"),
            json!({ "audit_date": "2026-04-20" }),
        );

        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
        assert_eq!(error.kind, AppErrorKind::System);
        assert!(error.support_id.is_some());
    }
}
