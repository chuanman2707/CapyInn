use axum::Router;
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpService, StreamableHttpServerConfig};
use sqlx::{Pool, Sqlite};
use std::net::SocketAddr;
use std::sync::Arc;
use tauri::AppHandle;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::tools::HotelTools;

const DEFAULT_PORT: u16 = 61234;
const PORT_RANGE: std::ops::Range<u16> = 61234..61244;

pub struct RunningGatewayServer {
    pub port: u16,
    pub shutdown_tx: oneshot::Sender<()>,
    pub server_task: JoinHandle<()>,
}

/// Start the MCP Streamable HTTP server on localhost.
/// Tries DEFAULT_PORT first, then falls back to next available port in range.
/// Returns the port it's listening on.
pub async fn start_server(
    pool: Pool<Sqlite>,
    app_handle: AppHandle,
) -> Result<RunningGatewayServer, String> {
    let tools = HotelTools::new(pool, Some(app_handle));

    let session_manager = Arc::new(LocalSessionManager::default());
    let config = StreamableHttpServerConfig::default();

    let mcp_service = StreamableHttpService::new(
        move || Ok(tools.clone()),
        session_manager,
        config,
    );

    // Build axum router: health at /health, MCP at /mcp
    let app = Router::new()
        .route("/health", axum::routing::get(|| async { "OK" }))
        .route_service("/mcp", mcp_service.clone())
        .route_service("/mcp/{*path}", mcp_service);

    // Try ports in range
    let mut port = DEFAULT_PORT;
    let listener = loop {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => break listener,
            Err(_) if port < PORT_RANGE.end => {
                port += 1;
                continue;
            }
            Err(e) => return Err(format!("Failed to bind to any port in range {}-{}: {}", PORT_RANGE.start, PORT_RANGE.end, e)),
        }
    };

    let actual_port = listener.local_addr().map_err(|e| e.to_string())?.port();

    let (shutdown_tx, shutdown_rx) = oneshot::channel();

    let server_task = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
        {
            eprintln!("MCP Gateway server error: {}", e);
        }
    });

    Ok(RunningGatewayServer {
        port: actual_port,
        shutdown_tx,
        server_task,
    })
}
