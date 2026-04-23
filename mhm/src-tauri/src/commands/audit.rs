use super::{emit_db_update, get_user, require_admin, require_admin_user, AppState};
use crate::app_identity;
use crate::{
    app_error::{
        codes, correlation_context, log_system_error, normalize_correlation_id,
        record_command_failure, record_command_failure_with_db_group, CommandError, CommandResult,
        EffectiveCorrelationId,
    },
    db_error_monitoring::{
        classify_db_failure, inject_db_error_group, DbErrorGroup, MonitoredDbFailure,
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

fn record_audit_auth_error(
    effective_correlation_id: &EffectiveCorrelationId,
    error: CommandError,
    context: Value,
) -> CommandError {
    log_user_audit_error(
        "run_night_audit",
        effective_correlation_id,
        error.message.as_str(),
        &context,
    );
    record_command_failure(
        "run_night_audit",
        &error,
        &effective_correlation_id.value,
        context,
    );
    error
}

fn map_audit_error(
    command_name: &str,
    effective_correlation_id: &EffectiveCorrelationId,
    error: BookingError,
    context: Value,
) -> (CommandError, Option<DbErrorGroup>) {
    match error {
        BookingError::Validation(message) if message == "Ngày audit không hợp lệ" => {
            (
                map_audit_user_error(
                    codes::AUDIT_INVALID_DATE,
                    command_name,
                    effective_correlation_id,
                    message,
                    &context,
                ),
                None,
            )
        }
        BookingError::Validation(message) | BookingError::Conflict(message)
            if message.starts_with("Đã audit ngày ") =>
        {
            (
                map_audit_user_error(
                    codes::AUDIT_DATE_ALREADY_RUN,
                    command_name,
                    effective_correlation_id,
                    message,
                    &context,
                ),
                None,
            )
        }
        BookingError::Validation(message) | BookingError::Conflict(message) => {
            (
                map_audit_user_error(
                    codes::AUDIT_INVALID_DATE,
                    command_name,
                    effective_correlation_id,
                    message,
                    &context,
                ),
                None,
            )
        }
        BookingError::DatabaseWrite(message) => {
            let db_error_group = classify_db_failure(MonitoredDbFailure::DatabaseWrite(&message));
            (
                log_system_error(
                    command_name,
                    &message,
                    inject_db_error_group(
                        correlation_context(&effective_correlation_id.value, context),
                        db_error_group,
                    ),
                ),
                Some(db_error_group),
            )
        }
        BookingError::Database(message) => {
            let db_error_group = classify_db_failure(MonitoredDbFailure::DatabaseRead(&message));
            (
                log_system_error(
                    command_name,
                    &message,
                    inject_db_error_group(
                        correlation_context(&effective_correlation_id.value, context),
                        db_error_group,
                    ),
                ),
                Some(db_error_group),
            )
        }
        BookingError::DateTimeParse(message) => (
            log_system_error(
                command_name,
                message,
                correlation_context(&effective_correlation_id.value, context),
            ),
            None,
        ),
        BookingError::NotFound(message) => (
            log_system_error(
                command_name,
                message,
                correlation_context(&effective_correlation_id.value, context),
            ),
            None,
        ),
    }
}

fn audit_failure_context(audit_date: &str, notes: Option<&str>) -> Value {
    let notes_present = notes.map(|value| !value.trim().is_empty()).unwrap_or(false);

    json!({
        "audit_date": audit_date,
        "notes_present": notes_present,
    })
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
    let error_context = audit_failure_context(&audit_date, notes.as_deref());
    let notes_present = error_context["notes_present"].as_bool().unwrap_or(false);
    log::info!(
        "run_night_audit start correlation_id={} source={:?} audit_date={} notes_present={}",
        effective_correlation_id.value,
        effective_correlation_id.source,
        audit_date,
        notes_present
    );
    let user = require_admin_user(get_user(&state)).map_err(|error| {
        record_audit_auth_error(&effective_correlation_id, error, error_context.clone())
    })?;
    let log = audit_service::run_night_audit(&state.db, &audit_date, notes, &user.id)
        .await
        .map_err(|error| {
            let (command_error, db_error_group) = map_audit_error(
                "run_night_audit",
                &effective_correlation_id,
                error,
                error_context.clone(),
            );
            record_command_failure_with_db_group(
                "run_night_audit",
                &command_error,
                &effective_correlation_id.value,
                db_error_group,
                error_context.clone(),
            );
            command_error
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
    use super::{audit_failure_context, map_audit_error, record_audit_auth_error};
    use crate::app_error::{
        codes, record_command_failure_with_db_group, AppErrorKind, CorrelationIdSource,
        EffectiveCorrelationId,
    };
    use crate::commands::require_admin_user;
    use crate::db_error_monitoring::DbErrorGroup;
    use crate::domain::booking::BookingError;
    use crate::models::User;
    use serde_json::json;
    use std::fs;

    fn frontend_correlation_id() -> EffectiveCorrelationId {
        EffectiveCorrelationId {
            value: "COR-1A2B3C4D".to_string(),
            source: CorrelationIdSource::Frontend,
            rejected_length: None,
        }
    }

    fn mock_user(role: &str) -> User {
        User {
            id: "u1".to_string(),
            name: "Test".to_string(),
            role: role.to_string(),
            active: true,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    fn parse_json_lines(contents: &str) -> Vec<serde_json::Value> {
        contents
            .lines()
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
    fn map_audit_error_maps_invalid_date_to_shared_code() {
        let (error, db_error_group) = map_audit_error(
            "run_night_audit",
            &frontend_correlation_id(),
            BookingError::validation("Ngày audit không hợp lệ"),
            json!({ "audit_date": "2026-02-30" }),
        );

        assert_eq!(error.code, codes::AUDIT_INVALID_DATE);
        assert_eq!(error.message, "Ngày audit không hợp lệ");
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
        assert!(db_error_group.is_none());
    }

    #[test]
    fn map_audit_error_maps_duplicate_audit_to_shared_code() {
        let (error, db_error_group) = map_audit_error(
            "run_night_audit",
            &frontend_correlation_id(),
            BookingError::validation("Đã audit ngày 2026-04-20 rồi!"),
            json!({ "audit_date": "2026-04-20" }),
        );

        assert_eq!(error.code, codes::AUDIT_DATE_ALREADY_RUN);
        assert_eq!(error.message, "Đã audit ngày 2026-04-20 rồi!");
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
        assert!(db_error_group.is_none());
    }

    #[test]
    fn map_audit_error_maps_database_write_to_system_contract_with_write_failed_group() {
        let (error, db_error_group) = map_audit_error(
            "run_night_audit",
            &frontend_correlation_id(),
            BookingError::database_write("disk full"),
            json!({ "audit_date": "2026-04-20" }),
        );

        assert_eq!(error.code, codes::SYSTEM_INTERNAL_ERROR);
        assert_eq!(error.kind, AppErrorKind::System);
        assert!(error.support_id.is_some());
        assert_eq!(db_error_group, Some(DbErrorGroup::WriteFailed));
    }

    #[test]
    fn audit_failure_context_keeps_only_date_and_notes_flag() {
        let context = audit_failure_context("2026-04-20", Some("Đã kiểm tra kho"));

        assert_eq!(
            context,
            json!({
                "audit_date": "2026-04-20",
                "notes_present": true,
            })
        );
    }

    #[test]
    fn system_run_night_audit_failure_writes_write_failed_group_to_both_logs() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();
        let runtime_root = std::env::temp_dir().join(format!(
            "capyinn-night-audit-support-id-{}",
            uuid::Uuid::new_v4()
        ));

        let previous_runtime_root = std::env::var_os("CAPYINN_RUNTIME_ROOT");
        std::env::set_var("CAPYINN_RUNTIME_ROOT", &runtime_root);
        let context = audit_failure_context("2026-04-20", Some("Đã kiểm tra kho"));
        let (error, db_error_group) = map_audit_error(
            "run_night_audit",
            &frontend_correlation_id(),
            BookingError::database_write("disk full"),
            context.clone(),
        );
        let support_id = error.support_id.clone().expect("system error support id");
        record_command_failure_with_db_group(
            "run_night_audit",
            &error,
            "COR-1A2B3C4D",
            db_error_group,
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
                && record["command"] == "run_night_audit"
                && record["code"] == codes::SYSTEM_INTERNAL_ERROR
                && record["context"]["db_error_group"] == "write_failed"
        }));
        assert!(command_records.iter().any(|record| {
            record["support_id"] == support_id
                && record["command"] == "run_night_audit"
                && record["code"] == codes::SYSTEM_INTERNAL_ERROR
                && record["db_error_group"] == "write_failed"
        }));
        assert_eq!(db_error_group, Some(DbErrorGroup::WriteFailed));

        let _ = fs::remove_dir_all(&runtime_root);
    }

    #[test]
    fn forbidden_run_night_audit_auth_failure_is_recorded_with_scrubbed_context() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();
        let runtime_root =
            std::env::temp_dir().join(format!("capyinn-night-audit-auth-{}", uuid::Uuid::new_v4()));

        std::env::set_var("CAPYINN_RUNTIME_ROOT", &runtime_root);
        let context = audit_failure_context("2026-04-20", Some("Đã kiểm tra kho"));
        let auth_error =
            require_admin_user(Some(mock_user("receptionist"))).expect_err("non-admin must fail");
        let error = record_audit_auth_error(&frontend_correlation_id(), auth_error, context);
        std::env::remove_var("CAPYINN_RUNTIME_ROOT");

        let command_log_path = runtime_root
            .join("diagnostics")
            .join("command-failures.jsonl");
        let command_contents =
            fs::read_to_string(&command_log_path).expect("command failure log contents");
        let command_record: serde_json::Value = serde_json::from_str(
            command_contents
                .lines()
                .last()
                .expect("command failure log line"),
        )
        .expect("command failure json");

        assert_eq!(error.code, codes::AUTH_FORBIDDEN);
        assert_eq!(error.kind, AppErrorKind::User);
        assert!(error.support_id.is_none());
        assert_eq!(command_record["command"], "run_night_audit");
        assert_eq!(command_record["code"], codes::AUTH_FORBIDDEN);
        assert_eq!(command_record["kind"], "user");
        assert_eq!(command_record["correlation_id"], "COR-1A2B3C4D");
        assert_eq!(
            command_record["context"],
            json!({
                "audit_date": "2026-04-20",
                "notes_present": true,
            })
        );

        let _ = fs::remove_dir_all(&runtime_root);
    }
}
