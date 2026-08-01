use log::{error, info};
use tauri::Manager;
use tauri_plugin_window_state::AppHandleExt;

pub mod agent;
pub mod aggregate_locks;
pub mod app_error;
pub mod app_identity;
#[cfg(test)]
mod architecture_guard;
mod backup;
mod command_failure_log;
pub mod command_idempotency;
pub mod command_ledger;
pub mod command_recovery;
mod commands;
mod crash_index;
mod db;
pub mod db_error_monitoring;
pub mod declaration;
mod diagnostics;
mod domain;
pub mod gateway;
mod models;
pub mod money;
mod money_migration;
mod ocr;
pub mod outbox;
mod pricing;
mod queries;
mod repositories;
pub mod restore_drill;
mod runtime_config;
mod services;
pub mod support_log;
mod watcher;
mod window_state;
pub mod write_manifest;

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

    fn shutdown(&self, agent_supervisor: Option<&agent::supervisor::AgentSupervisor>) {
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
                if let Some(agent_supervisor) = agent_supervisor {
                    runtime.block_on(agent_supervisor.shutdown());
                }
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

fn write_smoke_ready_file() -> Result<(), String> {
    let Some(path) = runtime_config::smoke_ready_file() else {
        return Ok(());
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }

    let payload = serde_json::json!({
        "status": "ready",
        "runtime_root": crate::app_identity::runtime_root(),
    });

    std::fs::write(path, payload.to_string()).map_err(|error| error.to_string())
}

fn updater_enabled_from_env(debug_build: bool, env_enabled: bool) -> bool {
    !debug_build || env_enabled
}

fn updater_enabled() -> bool {
    updater_enabled_from_env(
        cfg!(debug_assertions),
        runtime_config::env_flag("CAPYINN_ENABLE_UPDATER"),
    )
}

/// Auto-update is a convenience. Losing it must never cost the operator their app.
///
/// `plugins.updater.pubkey` lives in a CI variable, not in `tauri.conf.json`, so any
/// release built outside the release workflow registers the plugin against a config
/// with no `pubkey` and the plugin refuses to initialise. This used to be an
/// `.expect()`, which turned a missing convenience into a startup panic: the app died
/// before it ever opened the database. Crash bundles from 2026-04-22 (0.1.4) and
/// 2026-07-26 (0.1.6) are both this exact failure.
///
/// Returns whether the plugin registered, so callers can report it; the app runs
/// either way.
fn report_updater_registration<E: std::fmt::Display>(result: Result<(), E>) -> bool {
    match result {
        Ok(()) => true,
        Err(error) => {
            error!(
                "Updater plugin failed to register, continuing without auto-update: {}",
                error
            );
            false
        }
    }
}

fn gateway_runtime_effective_enabled() -> bool {
    runtime_config::effective_experimental_gateway_runtime_enabled()
}

fn agent_runtime_effective_enabled() -> bool {
    runtime_config::effective_experimental_agent_runtime_enabled()
}

fn experimental_runtime_status_value() -> serde_json::Value {
    serde_json::json!({
        "experimental_runtime_enabled": runtime_config::experimental_runtime_enabled(),
        "gateway_runtime_enabled": gateway_runtime_effective_enabled(),
        "agent_runtime_enabled": agent_runtime_effective_enabled(),
        "gateway_disabled_by_override": runtime_config::gateway_runtime_disabled_by_override(),
        "agent_disabled_by_override": runtime_config::agent_runtime_disabled_by_override(),
    })
}

fn gateway_management_disabled_error() -> String {
    gateway::gateway_runtime_disabled_error()
}

fn ensure_gateway_management_enabled() -> Result<(), String> {
    if gateway_runtime_effective_enabled() {
        Ok(())
    } else {
        Err(gateway_management_disabled_error())
    }
}

fn outbox_subscribers_for_runtime_profile() -> Vec<Arc<dyn outbox::OutboxSubscriber>> {
    Vec::new()
}

fn spawn_crash_index_rebuild() {
    std::thread::spawn(|| {
        if let Err(error) = crash_index::rebuild_current_runtime_root() {
            error!("Failed to rebuild crash index: {}", error);
        }
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
        .plugin(window_state::plugin())
        .setup(|app| {
            #[cfg(desktop)]
            {
                if updater_enabled() {
                    report_updater_registration(
                        app.handle()
                            .plugin(tauri_plugin_updater::Builder::new().build()),
                    );
                } else {
                    info!("Updater plugin disabled for this build");
                }
            }

            // Before the database work below, so the window settles at its final size
            // rather than resizing after startup.
            window_state::apply_startup_geometry(app.handle());

            let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
            let pool = rt.block_on(db::init_db()).expect("Failed to init database");
            match rt.block_on(command_recovery::scan_command_recovery_startup(&pool)) {
                Ok(scan) if scan.expired_in_progress > 0 || scan.failed_retryable > 0 => info!(
                    "Command recovery attention required: expired_in_progress={} failed_retryable={}",
                    scan.expired_in_progress, scan.failed_retryable
                ),
                Ok(_) => {}
                Err(error) => error!(
                    "Command recovery startup scan failed: {} {}",
                    error.code, error.message
                ),
            }

            // Start MCP Gateway server on a dedicated background thread.
            // The runtime must outlive the setup closure, otherwise the spawned
            // axum server task gets cancelled when the runtime drops.
            let gateway_pool = pool.clone();
            let gateway_handle = app.handle().clone();
            let gateway_runtime = if !runtime_config::experimental_gateway_runtime_enabled() {
                info!("MCP Gateway experimental runtime disabled by default");
                None
            } else if runtime_config::gateway_runtime_disabled_by_override() {
                info!("MCP Gateway disabled by CAPYINN_DISABLE_GATEWAY");
                None
            } else {
                rt.block_on(async {
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
                })
            };

            let agent_supervisor = agent::supervisor::AgentSupervisor::new(pool.clone());
            if let Err(error) = rt.block_on(agent::supervisor::reconcile_managed_supervisor(
                &pool,
                Some(&agent_supervisor),
            )) {
                error!(
                    "Failed to reconcile CEO Telegram runtime at startup: {} {}",
                    error.code, error.message
                );
            }

            app.manage(AppState {
                db: pool.clone(),
                current_user: Arc::new(Mutex::new(None)),
            });
            app.manage(backup::BackupCoordinator::new());
            app.manage(backup::start_backup_scheduler(app.handle().clone()));
            app.manage(outbox::start_outbox_dispatcher(
                pool.clone(),
                outbox_subscribers_for_runtime_profile(),
            ));
            app.manage(agent_supervisor);
            app.manage(GatewayRuntimeState::new(rt, gateway_runtime));

            let _ = std::fs::create_dir_all(app_identity::models_dir());
            spawn_crash_index_rebuild();

            if runtime_config::env_flag("CAPYINN_DISABLE_WATCHER") {
                info!("File watcher disabled by CAPYINN_DISABLE_WATCHER");
            } else {
                // Start file watcher in background
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    if let Err(e) = watcher::start_watcher(handle) {
                        error!("Failed to start file watcher: {}", e);
                    }
                });
            }

            write_smoke_ready_file().map_err(std::io::Error::other)?;

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Room & Booking Operations
            commands::rooms::get_rooms,
            commands::rooms::get_dashboard_stats,
            commands::rooms::check_in,
            commands::rooms::backfill_stay,
            commands::rooms::get_room_detail,
            commands::rooms::check_out,
            commands::rooms::preview_checkout_settlement,
            commands::rooms::extend_stay,
            commands::rooms::shorten_stay,
            commands::rooms::set_booking_rate,
            commands::rooms::update_booking_notes,
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
            commands::agent_settings::get_ceo_cloud_data_opt_in,
            commands::agent_settings::set_ceo_cloud_data_opt_in,
            commands::agent_settings::get_ceo_telegram_config,
            commands::agent_settings::set_ceo_telegram_config,
            commands::agent_settings::set_ceo_telegram_bot_token,
            commands::agent_settings::clear_ceo_telegram_bot_token,
            commands::agent_settings::set_ceo_openai_api_key,
            commands::agent_settings::clear_ceo_openai_api_key,
            commands::agent_settings::get_ceo_telegram_gate_status,
            commands::agent_settings::get_ceo_digest_config,
            commands::agent_settings::get_ceo_digest_gate_status,
            commands::agent_settings::set_ceo_digest_config,
            commands::local_receptionist::local_receptionist_chat,
            commands::diagnostics::record_js_crash,
            commands::diagnostics::get_pending_crash_report,
            commands::diagnostics::mark_crash_report_submitted,
            commands::diagnostics::mark_crash_report_dismissed,
            commands::diagnostics::mark_crash_report_send_failed,
            commands::diagnostics::export_crash_report,
            commands::command_ledger::list_command_ledger,
            commands::command_ledger::list_command_ledger_attention,
            commands::command_ledger::get_command_ledger_detail,
            commands::command_recovery::list_command_recovery_queue,
            commands::command_recovery::inspect_command_recovery,
            commands::command_recovery::request_command_recovery_retry,
            commands::command_recovery::dismiss_command_recovery,
            commands::command_recovery::mark_command_recovery_terminal,
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
            commands::pricing::get_room_type_rates,
            commands::pricing::save_pricing_rule,
            commands::pricing::calculate_price_preview,
            commands::pricing::calculate_room_price_preview,
            commands::pricing::get_special_dates,
            commands::pricing::save_special_date_range,
            commands::pricing::delete_special_dates,
            // Folio/Billing
            commands::billing::add_folio_line,
            commands::billing::record_payment,
            commands::billing::get_folio_lines,
            // Night Audit
            commands::audit::run_night_audit,
            commands::audit::get_audit_logs,
            // Khai báo tạm trú
            commands::declaration::kbtt_extract_from_image,
            commands::declaration::kbtt_list_stays,
            commands::declaration::kbtt_save_identity,
            commands::declaration::kbtt_link,
            commands::declaration::kbtt_unlinked_identities,
            commands::declaration::kbtt_discard_identity,
            commands::declaration::kbtt_unlink,
            commands::declaration::kbtt_pending_rows,
            commands::declaration::kbtt_validate,
            commands::declaration::kbtt_export,
            commands::declaration::kbtt_reconcile,
            commands::declaration::kbtt_undeclared_count,
            commands::declaration::kbtt_list_batches,
            commands::declaration::kbtt_open_export_dir,
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
            get_experimental_runtime_status,
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
            // Must run before prevent_exit(): the exit path below drains backups on a
            // spawned task and only then calls exit(0), so the plugin's own save on
            // RunEvent::Exit can read a cache that was never refreshed from live
            // geometry when the user quits with Cmd+Q without closing the window.
            // Here the window is guaranteed to still exist.
            let _ = app_handle.save_window_state(window_state::state_flags());

            api.prevent_exit();
            if !app_handle
                .state::<backup::BackupCoordinator>()
                .begin_exit_drain()
            {
                return;
            }
            app_handle
                .state::<backup::BackupSchedulerHandle>()
                .shutdown();
            let handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                handle
                    .state::<agent::supervisor::AgentSupervisor>()
                    .shutdown()
                    .await;
                if let Err(error) = backup::drain_and_backup_on_exit(&handle).await {
                    error!("exit backup failed: {}", error);
                }
                handle.exit(0);
            });
        }
        tauri::RunEvent::Exit => {
            let agent_supervisor = app_handle.state::<agent::supervisor::AgentSupervisor>();
            app_handle
                .state::<GatewayRuntimeState>()
                .shutdown(Some(&*agent_supervisor));
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
fn get_experimental_runtime_status() -> serde_json::Value {
    experimental_runtime_status_value()
}

#[tauri::command]
async fn gateway_generate_key(
    state: tauri::State<'_, AppState>,
    label: Option<String>,
) -> Result<String, String> {
    commands::require_admin(&state)?;
    ensure_gateway_management_enabled()?;

    let (key, hash) = gateway::auth::generate_api_key();
    gateway::auth::store_api_key(&state.db, &hash, label.as_deref().unwrap_or("default")).await?;
    Ok(key)
}

#[tauri::command]
async fn gateway_get_status(
    state: tauri::State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let experimental_enabled = gateway_runtime_effective_enabled();
    let has_keys = gateway::auth::has_api_keys(&state.db).await;
    let port = if experimental_enabled {
        gateway::live_port_from_lockfile()
    } else {
        None
    };

    Ok(serde_json::json!({
        "experimental_enabled": experimental_enabled,
        "running": experimental_enabled && port.is_some(),
        "port": port,
        "has_api_keys": has_keys,
    }))
}

#[cfg(test)]
mod tests {
    use super::{
        experimental_runtime_status_value, gateway_management_disabled_error,
        gateway_runtime_effective_enabled, outbox_subscribers_for_runtime_profile,
        report_updater_registration, updater_enabled_from_env,
    };

    fn clear_experimental_runtime_env() {
        for name in [
            "CAPYINN_EXPERIMENTAL_RUNTIME",
            "CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME",
            "CAPYINN_EXPERIMENTAL_AGENT_RUNTIME",
            "CAPYINN_EXPERIMENTAL_PERIPHERAL_RUNTIME",
            "CAPYINN_DISABLE_GATEWAY",
            "CAPYINN_DISABLE_CEO_TELEGRAM",
        ] {
            std::env::remove_var(name);
        }
    }

    #[test]
    fn updater_stays_disabled_for_plain_dev_runs() {
        assert!(!updater_enabled_from_env(true, false));
    }

    #[test]
    fn updater_can_be_enabled_for_dev_smoke_runs() {
        assert!(updater_enabled_from_env(true, true));
    }

    #[test]
    fn updater_stays_enabled_outside_dev_even_without_env_flag() {
        assert!(updater_enabled_from_env(false, false));
    }

    /// The exact string the plugin returns when `tauri.conf.json` carries no
    /// `pubkey` — i.e. every release built outside the release workflow.
    #[test]
    fn a_release_built_without_the_updater_pubkey_still_starts() {
        let missing_pubkey = "PluginInitialization(\"updater\", \"Error deserializing \
             'plugins.updater' within your Tauri configuration: missing field `pubkey`\")";

        // The point of the test is that this line returns instead of unwinding.
        let registered = report_updater_registration(Err(missing_pubkey));

        assert!(
            !registered,
            "a config without pubkey must report the updater as unregistered"
        );
    }

    #[test]
    fn a_successful_registration_reports_the_updater_as_live() {
        assert!(report_updater_registration(Ok::<(), &str>(())));
    }

    #[test]
    fn experimental_runtime_status_reports_effective_runtime_gates() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();

        for name in [
            "CAPYINN_EXPERIMENTAL_RUNTIME",
            "CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME",
            "CAPYINN_EXPERIMENTAL_AGENT_RUNTIME",
            "CAPYINN_DISABLE_GATEWAY",
            "CAPYINN_DISABLE_CEO_TELEGRAM",
        ] {
            std::env::remove_var(name);
        }

        let disabled = experimental_runtime_status_value();
        assert_eq!(disabled["experimental_runtime_enabled"], false);
        assert_eq!(disabled["gateway_runtime_enabled"], false);
        assert_eq!(disabled["agent_runtime_enabled"], false);

        std::env::set_var("CAPYINN_EXPERIMENTAL_RUNTIME", "true");
        std::env::set_var("CAPYINN_DISABLE_GATEWAY", "true");

        let gateway_disabled = experimental_runtime_status_value();
        assert_eq!(gateway_disabled["experimental_runtime_enabled"], true);
        assert_eq!(gateway_disabled["gateway_runtime_enabled"], false);
        assert_eq!(gateway_disabled["gateway_disabled_by_override"], true);
        assert_eq!(gateway_disabled["agent_runtime_enabled"], true);

        std::env::remove_var("CAPYINN_EXPERIMENTAL_RUNTIME");
        std::env::remove_var("CAPYINN_DISABLE_GATEWAY");
    }

    #[test]
    fn gateway_management_error_is_returned_when_effective_gateway_runtime_is_disabled() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();

        std::env::remove_var("CAPYINN_EXPERIMENTAL_RUNTIME");
        std::env::remove_var("CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME");
        std::env::remove_var("CAPYINN_DISABLE_GATEWAY");

        assert!(!gateway_runtime_effective_enabled());
        assert_eq!(
            gateway_management_disabled_error(),
            "MCP Gateway experimental runtime is disabled. Set CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME=true or CAPYINN_EXPERIMENTAL_RUNTIME=true to enable gateway management."
        );
    }

    #[test]
    fn outbox_runtime_profile_has_no_subscribers_without_experimental_flags() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();
        clear_experimental_runtime_env();

        assert_eq!(outbox_subscribers_for_runtime_profile().len(), 0);
    }

    #[test]
    fn peripheral_runtime_flag_does_not_register_outbox_subscribers() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();
        clear_experimental_runtime_env();
        std::env::set_var("CAPYINN_EXPERIMENTAL_PERIPHERAL_RUNTIME", "true");

        assert_eq!(outbox_subscribers_for_runtime_profile().len(), 0);

        std::env::remove_var("CAPYINN_EXPERIMENTAL_PERIPHERAL_RUNTIME");
    }

    #[test]
    fn batch_2_runtime_flags_do_not_register_outbox_subscribers() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();

        for flag in [
            "CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME",
            "CAPYINN_EXPERIMENTAL_AGENT_RUNTIME",
            "CAPYINN_EXPERIMENTAL_RUNTIME",
        ] {
            clear_experimental_runtime_env();
            std::env::set_var(flag, "true");

            assert_eq!(
                outbox_subscribers_for_runtime_profile().len(),
                0,
                "{flag} should not register outbox subscribers in #143"
            );
        }

        clear_experimental_runtime_env();
    }
}
