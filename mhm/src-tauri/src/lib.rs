use log::{error, info};
use tauri::Manager;

pub mod app_identity;
mod backup;
mod commands;
mod db;
mod diagnostics;
mod domain;
pub mod gateway;
mod models;
mod ocr;
mod pricing;
mod queries;
mod repositories;
mod services;
mod watcher;

use commands::AppState;
use std::sync::{Arc, Mutex, Once};
use std::time::Duration;

static LOG_INIT: Once = Once::new();

struct GatewayRuntimeState {
    runtime: Mutex<Option<tokio::runtime::Runtime>>,
    shutdown_tx: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    server_task: Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl GatewayRuntimeState {
    fn new(runtime: tokio::runtime::Runtime, gateway: Option<gateway::RunningGateway>) -> Self {
        let (shutdown_tx, server_task) = match gateway {
            Some(gateway) => (Some(gateway.shutdown_tx), Some(gateway.server_task)),
            None => (None, None),
        };

        Self {
            runtime: Mutex::new(Some(runtime)),
            shutdown_tx: Mutex::new(shutdown_tx),
            server_task: Mutex::new(server_task),
        }
    }

    fn shutdown(&self) {
        let shutdown_tx = self
            .shutdown_tx
            .lock()
            .ok()
            .and_then(|mut guard| guard.take());
        let server_task = self
            .server_task
            .lock()
            .ok()
            .and_then(|mut guard| guard.take());

        if let Some(shutdown_tx) = shutdown_tx {
            let _ = shutdown_tx.send(());
        }

        gateway::cleanup_lockfile();

        if let Ok(mut runtime_guard) = self.runtime.lock() {
            if let Some(runtime) = runtime_guard.take() {
                if let Some(server_task) = server_task {
                    let _ = runtime.block_on(async move {
                        tokio::time::timeout(Duration::from_secs(2), server_task).await
                    });
                }
                drop(runtime);
            }
        }
    }
}

fn init_logging() {
    LOG_INIT.call_once(|| {
        let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
            .format_timestamp_secs()
            .try_init();
    });
}

/// Run the Tauri GUI application with MCP Gateway
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_logging();
    diagnostics::install_panic_hook(
        env!("CARGO_PKG_VERSION"),
        if cfg!(debug_assertions) {
            "development"
        } else {
            "production"
        },
    );

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            #[cfg(desktop)]
            {
                app.handle()
                    .plugin(tauri_plugin_updater::Builder::new().build())
                    .expect("failed to register updater plugin");
            }

            let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
            let pool = rt.block_on(db::init_db()).expect("Failed to init database");

            // Start MCP Gateway server on a dedicated background thread.
            // The runtime must outlive the setup closure, otherwise the spawned
            // axum server task gets cancelled when the runtime drops.
            let gateway_pool = pool.clone();
            let gateway_handle = app.handle().clone();
            let gateway_runtime = rt.block_on(async {
                match gateway::start_gateway(gateway_pool, gateway_handle).await {
                    Ok(gateway) => {
                        info!("MCP Gateway started on port {}", gateway.port);
                        Some(gateway)
                    }
                    Err(e) => {
                        error!("Failed to start MCP Gateway: {}", e);
                        None
                    }
                }
            });

            app.manage(AppState {
                db: pool,
                current_user: Arc::new(Mutex::new(None)),
            });
            app.manage(backup::BackupCoordinator::new());
            app.manage(GatewayRuntimeState::new(rt, gateway_runtime));

            let _ = std::fs::create_dir_all(app_identity::models_dir());

            // Start file watcher in background
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                if let Err(e) = watcher::start_watcher(handle) {
                    error!("Failed to start file watcher: {}", e);
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Room & Booking Operations
            commands::rooms::get_rooms,
            commands::rooms::get_dashboard_stats,
            commands::rooms::check_in,
            commands::rooms::get_room_detail,
            commands::rooms::check_out,
            commands::rooms::preview_checkout_settlement,
            commands::rooms::extend_stay,
            commands::rooms::get_housekeeping_tasks,
            commands::rooms::update_housekeeping,
            commands::rooms::create_expense,
            commands::rooms::get_expenses,
            commands::rooms::get_revenue_stats,
            commands::rooms::get_stay_info_text,
            commands::rooms::scan_image,
            // Bookings & Guests
            commands::bookings::get_all_bookings,
            commands::guests::get_all_guests,
            commands::guests::get_guest_history,
            // Analytics
            commands::analytics::get_analytics,
            commands::analytics::get_recent_activity,
            // Room Management
            commands::room_management::update_room,
            commands::room_management::create_room,
            commands::room_management::delete_room,
            commands::room_management::get_room_types,
            commands::room_management::create_room_type,
            commands::room_management::delete_room_type,
            commands::room_management::export_csv,
            // Settings
            commands::settings::save_settings,
            commands::settings::get_settings,
            commands::settings::get_crash_reporting_preference,
            commands::settings::set_crash_reporting_preference,
            commands::diagnostics::record_js_crash,
            commands::diagnostics::get_pending_crash_report,
            commands::diagnostics::mark_crash_report_submitted,
            commands::diagnostics::mark_crash_report_dismissed,
            commands::diagnostics::mark_crash_report_send_failed,
            commands::diagnostics::export_crash_report,
            commands::onboarding::get_bootstrap_status,
            commands::onboarding::complete_onboarding,
            // Auth & RBAC
            commands::auth::login,
            commands::auth::logout,
            commands::auth::get_current_user,
            commands::auth::list_users,
            commands::auth::create_user,
            commands::auth::search_guest_by_phone,
            // Pricing Engine
            commands::pricing::get_pricing_rules,
            commands::pricing::save_pricing_rule,
            commands::pricing::calculate_price_preview,
            commands::pricing::get_special_dates,
            commands::pricing::save_special_date,
            // Folio/Billing
            commands::billing::add_folio_line,
            commands::billing::get_folio_lines,
            // Night Audit
            commands::audit::run_night_audit,
            commands::audit::get_audit_logs,
            // Backup & Export
            commands::audit::backup_database,
            commands::audit::export_bookings_csv,
            // Reservation Calendar
            commands::reservations::check_availability,
            commands::reservations::create_reservation,
            commands::reservations::confirm_reservation,
            commands::reservations::cancel_reservation,
            commands::reservations::modify_reservation,
            commands::reservations::get_room_calendar,
            commands::reservations::get_rooms_availability,
            // MCP Gateway
            gateway_generate_key,
            gateway_get_status,
            // Invoice PDF
            commands::invoices::generate_invoice,
            commands::invoices::get_invoice,
            // Group Booking
            commands::groups::group_checkin,
            commands::groups::group_checkout,
            commands::groups::get_group_detail,
            commands::groups::get_all_groups,
            commands::groups::add_group_service,
            commands::groups::remove_group_service,
            commands::groups::auto_assign_rooms,
            commands::groups::generate_group_invoice,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| match event {
        tauri::RunEvent::ExitRequested { api, .. } => {
            api.prevent_exit();
            if !app_handle
                .state::<backup::BackupCoordinator>()
                .begin_exit_drain()
            {
                return;
            }
            let handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = backup::drain_and_backup_on_exit(&handle).await {
                    error!("exit backup failed: {}", error);
                }
                handle.exit(0);
            });
        }
        tauri::RunEvent::Exit => {
            app_handle.state::<GatewayRuntimeState>().shutdown();
        }
        _ => {}
    });
}

/// Run the MCP stdio proxy (Process B)
pub fn run_proxy() {
    init_logging();
    gateway::proxy::run_proxy();
}

// ─── MCP Gateway Tauri Commands ───

#[tauri::command]
async fn gateway_generate_key(
    state: tauri::State<'_, AppState>,
    label: Option<String>,
) -> Result<String, String> {
    commands::require_admin(&state)?;

    let (key, hash) = gateway::auth::generate_api_key();
    gateway::auth::store_api_key(&state.db, &hash, label.as_deref().unwrap_or("default")).await?;
    Ok(key)
}

#[tauri::command]
async fn gateway_get_status(
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let has_keys = gateway::auth::has_api_keys(&state.db).await;
    let port = gateway::live_port_from_lockfile();

    Ok(serde_json::json!({
        "running": port.is_some(),
        "port": port,
        "has_api_keys": has_keys,
    }))
}
