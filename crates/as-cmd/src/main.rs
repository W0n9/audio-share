//! `as-cmd` command-line entry point.
//!
//! Ticket 04 implements the minimal server path: `-b/--bind [host][:port]`
//! starts the Server with synthetic capture and runs until Ctrl-C. Full CLI
//! parity (listing endpoints/encodings, all capture flags, version layout) is
//! ticket 07.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use audio_share_core::capture::{CaptureConfig, SyntheticCapture};
use audio_share_core::protocol::{AudioFormat, Encoding, HEARTBEAT_INTERVAL, SESSION_TIMEOUT};
use audio_share_core::server::{default_host, Server, ServerConfig};
use clap::{CommandFactory, Parser};

/// v0.3.4 hard-codes port 65530 when the bind value has no port.
const DEFAULT_PORT: u16 = 65_530;

/// Synthetic capture format for this ticket: matches the Linux v0.3.4 default
/// (F32, stereo, 48 kHz) so the recorded fixture's GET_FORMAT bytes match.
const SYNTHETIC_FORMAT: AudioFormat = AudioFormat::new(Encoding::PcmFloat, 2, 48_000);

/// Bytes per synthetic capture callback (1024 stereo f32 frames).
const SYNTHETIC_FRAME_BYTES: usize = 8192;

/// Synthetic callback interval.
const SYNTHETIC_FRAME_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Debug, Parser)]
#[command(name = "as-cmd", version, about = "Audio Share Server command line")]
struct Cli {
    /// The server bind address: [host][:port]. A value without a port uses
    /// port 65530; a bare flag uses the default network address.
    #[arg(
        short = 'b',
        long = "bind",
        value_name = "[host][:port]",
        num_args = 0..=1,
        default_missing_value = ""
    )]
    bind: Option<String>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("as-cmd: {message}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<(), String> {
    let Some(bind) = cli.bind else {
        let mut command = Cli::command();
        let _ = command.write_help(&mut std::io::stderr());
        return Err("missing -b/--bind".to_owned());
    };
    let (host, port) = parse_bind(&bind)?;

    let capture = SyntheticCapture::new(
        SYNTHETIC_FORMAT,
        SYNTHETIC_FRAME_BYTES,
        SYNTHETIC_FRAME_INTERVAL,
    )
    .map_err(|error| error.to_string())?;
    let handle = Server::start(
        ServerConfig {
            host,
            port,
            heartbeat_interval: HEARTBEAT_INTERVAL,
            heartbeat_timeout: SESSION_TIMEOUT,
        },
        Arc::new(capture),
        CaptureConfig::default(),
    )
    .await
    .map_err(|error| error.to_string())?;

    println!("as-cmd: listening on {}", handle.local_addr());
    tokio::signal::ctrl_c()
        .await
        .map_err(|error| format!("install Ctrl-C handler: {error}"))?;
    println!("as-cmd: stopping");
    handle.stop().await.map_err(|error| error.to_string())
}

/// Parse v0.3.4 `[host][:port]` bind syntax for the minimal ticket-04 CLI.
///
/// A bare `-b` selects the default private IPv4 address and port 65530. An
/// explicit value uses its first `:` as the host/port separator; a value with
/// no `:` uses port 65530.
fn parse_bind(raw: &str) -> Result<(String, u16), String> {
    if raw.is_empty() {
        let host = default_host()
            .ok_or_else(|| "no default network address; use --bind=host:port".to_owned())?;
        return Ok((host.to_string(), DEFAULT_PORT));
    }
    match raw.split_once(':') {
        Some((host, port)) => {
            if host.is_empty() {
                return Err(format!("invalid bind address {raw:?}: host is empty"));
            }
            let port = port
                .parse::<u16>()
                .map_err(|_| format!("invalid port in bind address {raw:?}"))?;
            Ok((host.to_owned(), port))
        }
        None => Ok((raw.to_owned(), DEFAULT_PORT)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bind_uses_default_port_when_missing() {
        assert_eq!(
            parse_bind("127.0.0.1").unwrap(),
            ("127.0.0.1".to_owned(), DEFAULT_PORT)
        );
        assert_eq!(
            parse_bind("192.168.1.20:65080").unwrap(),
            ("192.168.1.20".to_owned(), 65_080)
        );
        assert_eq!(parse_bind("127.0.0.1:0").unwrap().1, 0);
    }

    #[test]
    fn parse_bind_rejects_empty_hosts_and_bad_ports() {
        assert!(parse_bind(":65080").is_err());
        assert!(parse_bind("127.0.0.1:abc").is_err());
        assert!(parse_bind("127.0.0.1:99999").is_err());
        assert!(parse_bind("127.0.0.1:-1").is_err());
    }
}
