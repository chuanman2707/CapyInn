pub mod auth;
pub mod models;
pub mod proxy;
pub mod server;
pub mod tools;

use sqlx::{Pool, Sqlite};
use tauri::AppHandle;
use std::net::{SocketAddr, TcpStream};
use std::path::Path;
use std::time::Duration;

use crate::app_identity;

pub use server::RunningGatewayServer as RunningGateway;

/// Start the MCP Gateway SSE server on a background Tokio task.
/// Returns the port number the server is listening on.
pub async fn start_gateway(
    pool: Pool<Sqlite>,
    app_handle: AppHandle,
) -> Result<RunningGateway, String> {
    cleanup_stale_lockfile();
    let running_gateway = server::start_server(pool, app_handle).await?;

    if let Some(lockfile) = app_identity::gateway_lockfile_opt() {
        write_lockfile(&lockfile, running_gateway.port)?;
    }

    eprintln!("MCP Gateway ready on :{}", running_gateway.port);
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
    use super::{cleanup_lockfile_path, live_port_from_lockfile_path, write_lockfile};
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
}
