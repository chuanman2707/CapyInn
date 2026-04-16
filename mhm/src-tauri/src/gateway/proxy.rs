use std::io::{self, BufRead, Write};
use std::net::TcpStream;

use crate::app_identity;

/// Run the dumb pipe proxy: stdin → TCP → stdout.
/// This is Process B — called when the binary is invoked with `--mcp-stdio`.
/// All logic/DB/state lives in Process A (Tauri GUI app).
pub fn run_proxy() {
    // Read port from lockfile
    let port = match read_port_from_lockfile() {
        Some(p) => p,
        None => {
            send_error("CapyInn is not running. Please open the app first.");
            std::process::exit(1);
        }
    };

    // Connect to Process A
    let mut stream = match TcpStream::connect(format!("127.0.0.1:{}", port)) {
        Ok(s) => s,
        Err(_) => {
            send_error("CapyInn is not running. Cannot connect to MCP Gateway.");
            std::process::exit(1);
        }
    };

    // Set timeouts
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
    let _ = stream.set_write_timeout(Some(std::time::Duration::from_secs(10)));

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout_lock = stdout.lock();

    // Proxy loop: read JSON-RPC from stdin, forward to TCP, read response, write to stdout
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        if line.trim().is_empty() {
            continue;
        }

        // Forward to Process A via TCP
        if stream.write_all(line.as_bytes()).is_err() {
            send_error("Lost connection to CapyInn.");
            break;
        }
        if stream.write_all(b"\n").is_err() {
            break;
        }
        if stream.flush().is_err() {
            break;
        }

        // Read response from Process A
        let mut response = String::new();
        let mut reader = io::BufReader::new(&stream);
        match reader.read_line(&mut response) {
            Ok(0) | Err(_) => {
                send_error("CapyInn closed the connection.");
                break;
            }
            Ok(_) => {}
        }

        // Forward response to stdout
        let _ = stdout_lock.write_all(response.as_bytes());
        let _ = stdout_lock.flush();
    }
}

fn read_port_from_lockfile() -> Option<u16> {
    let _ = app_identity::gateway_lockfile_opt()?;
    super::live_port_from_lockfile()
}

fn send_error(message: &str) {
    let error = serde_json::json!({
        "jsonrpc": "2.0",
        "error": {
            "code": -32000,
            "message": message
        },
        "id": null
    });
    let stdout = io::stdout();
    let mut stdout_lock = stdout.lock();
    let _ = writeln!(stdout_lock, "{}", error);
    let _ = stdout_lock.flush();
}
