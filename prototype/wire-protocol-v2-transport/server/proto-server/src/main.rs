//! wire-protocol-v2 prototype test server CLI.
//!
//! Prints one machine-readable readiness line:
//! `PROTO_SERVER_READY addr=<ip:port> spki=<64 hex chars>`
//! and one summary line per finished session:
//! `PROTO_SERVER_SESSION ...`.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::Context;
use clap::Parser;
use proto_server::{ServerConfig, ServerHandle};

/// wire-protocol-v2 prototype QUIC test server (throwaway, ticket 01)
#[derive(Debug, Parser)]
#[command(about, version)]
struct Args {
    /// UDP bind address. Defaults away from the v1 port 65530.
    #[arg(long, default_value = "127.0.0.1:65531")]
    bind: SocketAddr,

    /// The single accepted AUTH credential token. Falls back to
    /// $PROTO_TEST_CREDENTIAL, then the built-in test token.
    #[arg(long)]
    credential: Option<String>,

    /// Write the SPKI sha256 (64 lowercase hex chars) to this file.
    #[arg(long)]
    spki_out: Option<std::path::PathBuf>,

    /// Idle timeout in seconds for CONNECTING/READY sessions; 0 disables.
    #[arg(long, default_value_t = 60)]
    idle_timeout_secs: u64,

    /// Milliseconds of audio produced per media ticker tick.
    #[arg(long, default_value_t = 5)]
    chunk_ms: u64,

    /// Bounded outbound media queue capacity (datagrams).
    #[arg(long, default_value_t = 256)]
    queue_capacity: usize,

    /// Enable DPLPMTUD (disabled by default for a deterministic size bound).
    #[arg(long)]
    mtud: bool,

    /// Increase log verbosity (-v debug, -vv trace).
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
    verbose: u8,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let filter = match args.verbose {
        0 => "info",
        1 => "debug",
        _ => "trace",
    };
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(format!(
            "proto_server={filter},proto_core={filter},quinn=warn,quinn_proto=warn"
        )))
        .init();

    let credential = args
        .credential
        .or_else(|| std::env::var("PROTO_TEST_CREDENTIAL").ok())
        .unwrap_or_else(|| proto_server::DEFAULT_CREDENTIAL.to_string());

    let config = ServerConfig {
        bind: args.bind,
        credential,
        idle_timeout: if args.idle_timeout_secs == 0 {
            None
        } else {
            Some(Duration::from_secs(args.idle_timeout_secs))
        },
        chunk_target_ms: args.chunk_ms,
        queue_capacity: args.queue_capacity,
        forwarder_delay: None,
        mtud_discovery: args.mtud,
    };

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(run(config, args.spki_out))
}

async fn run(config: ServerConfig, spki_out: Option<std::path::PathBuf>) -> anyhow::Result<()> {
    let server: ServerHandle = proto_server::spawn(config)
        .map_err(|e| anyhow::anyhow!("server spawn: {e}"))
        .and_then(|handle| {
            if let Some(path) = &spki_out {
                std::fs::write(path, format!("{}\n", handle.spki_sha256_hex()))
                    .with_context(|| format!("writing SPKI to {}", path.display()))?;
            }
            Ok(handle)
        })?;

    println!(
        "PROTO_SERVER_READY addr={} spki={}",
        server.local_addr(),
        server.spki_sha256_hex()
    );
    use std::io::Write;
    std::io::stdout().flush().context("flush READY line")?;

    tokio::signal::ctrl_c()
        .await
        .context("ctrl_c listener")?;
    tracing::info!("shutdown requested");
    server.stop().await;

    for stats in server.session_stats() {
        println!(
            "PROTO_SERVER_SESSION id={} addr={} state={} frames_in=(auth:{} sub:{} unsub:{} close:{}) \
             frames_out=(auth_ok:{} auth_err:{} media_start:{} close:{} protocol_err:{}) \
             media=(produced:{} sent:{} queue_dropped:{} sendbuf_dropped:{} bytes:{} batches:{}) \
             close_sent={:?} close_received={:?}",
            stats.session_id,
            stats.remote_addr,
            stats.state,
            stats.frames_auth_in,
            stats.frames_subscribe_in,
            stats.frames_unsubscribe_in,
            stats.frames_close_in,
            stats.auth_ok_out,
            stats.auth_err_out,
            stats.media_start_out,
            stats.close_out,
            stats.protocol_errors_out,
            stats.media_produced,
            stats.media_sent,
            stats.media_queue_dropped,
            stats.media_sendbuf_dropped,
            stats.media_bytes_sent,
            stats.media_batches,
            stats.close_sent_reason,
            stats.close_received_reason,
        );
    }
    Ok(())
}
