pub mod auth;
pub mod models;
pub mod observer;
pub mod policy;
pub mod proxy;
pub mod server;
pub mod tools;

use log::info;
use sqlx::{Pool, Sqlite};
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::time::Duration;
use tauri::AppHandle;

use crate::app_identity;

pub use server::RunningGatewayServer as RunningGateway;

const GATEWAY_RUNTIME_DISABLED_MESSAGE: &str = "MCP Gateway experimental runtime is disabled. Set CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME=true or CAPYINN_EXPERIMENTAL_RUNTIME=true to enable gateway management.";

pub(crate) fn gateway_runtime_disabled_error() -> String {
    GATEWAY_RUNTIME_DISABLED_MESSAGE.to_string()
}

fn ensure_gateway_runtime_enabled() -> Result<(), String> {
    if crate::runtime_config::effective_experimental_gateway_runtime_enabled() {
        Ok(())
    } else {
        Err(gateway_runtime_disabled_error())
    }
}

/// Start the MCP Gateway SSE server on a background Tokio task.
/// Returns the port number the server is listening on.
pub async fn start_gateway(
    pool: Pool<Sqlite>,
    app_handle: AppHandle,
) -> Result<RunningGateway, String> {
    ensure_gateway_runtime_enabled()?;
    cleanup_stale_lockfile();
    let running_gateway = server::start_server(pool, app_handle).await?;

    if let Some(lockfile) = app_identity::gateway_lockfile_opt() {
        write_lockfile(&lockfile, running_gateway.port)?;
    }

    info!("MCP Gateway ready on :{}", running_gateway.port);
    Ok(running_gateway)
}

/// Clean up the lockfile on shutdown
pub fn cleanup_lockfile() {
    if let Some(lockfile) = app_identity::gateway_lockfile_opt() {
        cleanup_lockfile_path(&lockfile);
    }
}

pub fn live_port_from_lockfile() -> Option<u16> {
    let lockfile = app_identity::gateway_lockfile_opt()?;
    live_port_from_lockfile_path(&lockfile)
}

fn cleanup_stale_lockfile() {
    let _ = live_port_from_lockfile();
}

fn write_lockfile(lockfile: &Path, port: u16) -> Result<(), String> {
    if let Some(parent) = lockfile.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create lockfile directory: {}", error))?;
    }

    std::fs::write(lockfile, port.to_string())
        .map_err(|error| format!("Failed to write lockfile: {}", error))
}

fn cleanup_lockfile_path(lockfile: &Path) {
    let _ = std::fs::remove_file(lockfile);
}

fn live_port_from_lockfile_path(lockfile: &Path) -> Option<u16> {
    let port = read_port_from_lockfile_path(lockfile)?;
    if is_port_live(port) {
        Some(port)
    } else {
        cleanup_lockfile_path(lockfile);
        None
    }
}

fn read_port_from_lockfile_path(lockfile: &Path) -> Option<u16> {
    let content = std::fs::read_to_string(lockfile).ok()?;
    content.trim().parse().ok()
}

fn is_port_live(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(250)).is_ok()
}

#[cfg(test)]
mod tests {
    use super::{
        cleanup_lockfile_path, ensure_gateway_runtime_enabled, gateway_runtime_disabled_error,
        live_port_from_lockfile_path, write_lockfile,
    };
    use std::path::PathBuf;

    fn temp_lockfile_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("capyinn-{label}-{}.lock", uuid::Uuid::new_v4()))
    }

    #[test]
    fn cleanup_lockfile_removes_existing_file() {
        let lockfile = temp_lockfile_path("cleanup-lockfile");
        write_lockfile(&lockfile, 61234).expect("writes test lockfile");

        cleanup_lockfile_path(&lockfile);

        assert!(!lockfile.exists());
    }

    #[test]
    fn gateway_lockfile_removes_stale_port_file() {
        let lockfile = temp_lockfile_path("gateway-lockfile");
        let listener =
            std::net::TcpListener::bind(("127.0.0.1", 0)).expect("binds ephemeral test port");
        let stale_port = listener.local_addr().expect("gets local addr").port();
        drop(listener);

        write_lockfile(&lockfile, stale_port).expect("writes stale port");

        assert_eq!(live_port_from_lockfile_path(&lockfile), None);
        assert!(!lockfile.exists());
    }

    #[test]
    fn gateway_runtime_gate_rejects_startup_when_experimental_gateway_is_disabled() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();

        for name in [
            "CAPYINN_EXPERIMENTAL_RUNTIME",
            "CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME",
            "CAPYINN_DISABLE_GATEWAY",
        ] {
            std::env::remove_var(name);
        }

        let error = ensure_gateway_runtime_enabled()
            .expect_err("gateway startup should fail closed without experimental gateway runtime");

        assert_eq!(error, gateway_runtime_disabled_error());
    }

    #[test]
    fn gateway_runtime_gate_allows_startup_when_effective_gateway_is_enabled() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();

        std::env::remove_var("CAPYINN_EXPERIMENTAL_RUNTIME");
        std::env::set_var("CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME", "true");
        std::env::remove_var("CAPYINN_DISABLE_GATEWAY");

        ensure_gateway_runtime_enabled().expect("gateway runtime is enabled");

        std::env::remove_var("CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME");
    }

    #[test]
    fn gateway_runtime_gate_respects_disable_override() {
        let _guard = crate::runtime_config::env_lock().lock().unwrap();

        std::env::set_var("CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME", "true");
        std::env::set_var("CAPYINN_DISABLE_GATEWAY", "true");

        let error = ensure_gateway_runtime_enabled()
            .expect_err("disable override should force gateway startup closed");

        assert_eq!(error, gateway_runtime_disabled_error());

        std::env::remove_var("CAPYINN_EXPERIMENTAL_GATEWAY_RUNTIME");
        std::env::remove_var("CAPYINN_DISABLE_GATEWAY");
    }

    #[test]
    fn start_gateway_checks_runtime_gate_before_starting_server() {
        let source = include_str!("mod.rs");
        let gate_check = source
            .find("ensure_gateway_runtime_enabled()?")
            .expect("start_gateway calls the runtime gate");
        let server_start = source
            .find("server::start_server")
            .expect("start_gateway starts the server");

        assert!(
            gate_check < server_start,
            "start_gateway must check runtime gate before server startup"
        );
    }
}
