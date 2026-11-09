//! Shared helpers for launching and connecting to a server process.

use std::net::SocketAddr;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use tokio::net::TcpStream;
use tokio::process::{Child, Command};

/// Pick a free TCP port on `host` by binding to port 0 and dropping it.
pub fn pick_free_port(host: &str) -> Result<u16> {
    let listener = std::net::TcpListener::bind((host, 0))
        .with_context(|| format!("pick a free port on {host}"))?;
    Ok(listener.local_addr()?.port())
}

/// Expand `{host}`, `{port}`, and `{bind}` placeholders in server arguments.
pub fn expand_args(raw: &[String], host: &str, port: u16) -> Vec<String> {
    let bind = format!("{host}:{port}");
    let port = port.to_string();
    raw.iter()
        .map(|argument| {
            argument
                .replace("{bind}", &bind)
                .replace("{host}", host)
                .replace("{port}", &port)
        })
        .collect()
}

/// Spawn an external server, killing it when the child handle is dropped.
pub fn spawn_server(command: &str, args: &[String]) -> Result<Child> {
    let mut process = Command::new(command);
    process.args(args).stdin(Stdio::null()).kill_on_drop(true);
    process
        .spawn()
        .with_context(|| format!("launch server {command:?} with args {args:?}"))
}

/// Resolve `ip:port` or `host:port` to one socket address.
pub async fn resolve_address(address: &str) -> Result<SocketAddr> {
    if let Ok(parsed) = address.parse::<SocketAddr>() {
        return Ok(parsed);
    }
    let mut resolved = tokio::net::lookup_host(address)
        .await
        .with_context(|| format!("resolve {address}"))?;
    resolved
        .next()
        .with_context(|| format!("{address} resolved to no addresses"))
}

/// Connect to a TCP address, retrying until `wait` elapses.
pub async fn connect_with_retry(address: SocketAddr, wait: Duration) -> Result<TcpStream> {
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        match TcpStream::connect(address).await {
            Ok(stream) => return Ok(stream),
            Err(error) if tokio::time::Instant::now() < deadline => {
                let _ = error;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(error) => {
                bail!("could not connect to {address} within {wait:?}: {error}");
            }
        }
    }
}
