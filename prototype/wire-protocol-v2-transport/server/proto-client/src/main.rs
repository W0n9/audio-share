//! wire-protocol-v2 prototype desktop test client CLI.
//!
//! Always prints one machine-readable result line:
//! `RESULT mode=<mode> ok=<bool> ...counters...` and exits 0 when the run
//! completed its intended flow (including intended negative outcomes), 1
//! otherwise.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use anyhow::Context;
use clap::Parser;
use proto_client::{ClientConfig, ClientError, MediaVerifier};
use proto_core::frame::{AuthErrCode, CloseReason, Frame};

/// wire-protocol-v2 prototype desktop test client (throwaway, ticket 01)
#[derive(Debug, Parser)]
#[command(about, version)]
struct Args {
    /// Server address (host:port).
    #[arg(long)]
    addr: SocketAddr,

    /// Pinned server SPKI sha256, 64 hex chars.
    #[arg(long)]
    pin: Option<String>,

    /// Read the pin from a file containing the 64 hex chars.
    #[arg(long)]
    pin_file: Option<std::path::PathBuf>,

    /// Negative test: pin an impossible value instead of --pin.
    #[arg(long)]
    wrong_pin: bool,

    /// ALPN to offer.
    #[arg(long, default_value = "audio-share/v2")]
    alpn: String,

    /// Negative test: offer audio-share/v9 (incompatible major).
    #[arg(long)]
    wrong_alpn: bool,

    /// AUTH credential token.
    #[arg(long)]
    credential: Option<String>,

    /// Negative test: present a wrong credential.
    #[arg(long)]
    wrong_credential: bool,

    /// Run mode: happy | hold | idle | no-auth-subscribe | bad-msg-type.
    #[arg(long, default_value = "happy")]
    mode: String,

    /// How long to receive media in happy mode (ms).
    #[arg(long, default_value_t = 1000)]
    duration_ms: u64,

    /// How long an `idle`-mode session stays connected-but-unsubscribed (ms).
    #[arg(long, default_value_t = 1000)]
    idle_ms: u64,

    /// Minimum datagrams to require in happy mode.
    #[arg(long, default_value_t = 50)]
    min_datagrams: u64,

    /// Simulate a slow reader: pause reading for this many ms after the
    /// first 10 datagrams (slow-reader demonstration; gaps are reported).
    #[arg(long, default_value_t = 0)]
    stop_reading_ms: u64,

    /// Enable DPLPMTUD (disabled by default for a deterministic size bound).
    #[arg(long)]
    mtud: bool,

    /// Increase log verbosity (-v debug, -vv trace).
    #[arg(short = 'v', long = "verbose", action = clap::ArgAction::Count)]
    verbose: u8,
}

const DEFAULT_CREDENTIAL: &str = "test-credential-do-not-use";
const WRONG_PIN: [u8; 32] = [0xab; 32];

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let filter = match args.verbose {
        0 => "warn",
        1 => "debug",
        _ => "trace",
    };
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(format!(
            "proto_client={filter},proto_core={filter},quinn=warn,quinn_proto=warn"
        )))
        .init();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(run(args));
    match &result {
        Ok(line) => {
            println!("{line}");
            Ok(())
        }
        Err(e) => {
            println!("RESULT ok=false error={e}");
            Err(anyhow::anyhow!("{e}"))
        }
    }
}

fn pin_from_args(args: &Args) -> anyhow::Result<[u8; 32]> {
    if args.wrong_pin {
        return Ok(WRONG_PIN);
    }
    let hex = if let Some(path) = &args.pin_file {
        std::fs::read_to_string(path)
            .with_context(|| format!("reading pin from {}", path.display()))?
            .trim()
            .to_string()
    } else {
        args.pin
            .clone()
            .context("either --pin or --pin-file is required")?
    };
    proto_core::spki::parse_hex32(&hex).map_err(|e| anyhow::anyhow!("--pin: {e}"))
}

async fn run(args: Args) -> anyhow::Result<String> {
    let started = Instant::now();
    let credential = args
        .credential
        .clone()
        .or_else(|| std::env::var("PROTO_TEST_CREDENTIAL").ok())
        .unwrap_or_else(|| DEFAULT_CREDENTIAL.to_string());
    let expected_handshake_failure = args.wrong_alpn || args.wrong_pin;
    let expected_auth_failure = args.wrong_credential;

    let cfg = ClientConfig {
        server_addr: args.addr,
        pinned_spki: pin_from_args(&args)?,
        alpn: if args.wrong_alpn {
            "audio-share/v9".to_string()
        } else {
            args.alpn.clone()
        },
        credential: if args.wrong_credential {
            "wrong-credential-do-not-use".to_string()
        } else {
            credential.clone()
        },
        connect_timeout: Duration::from_secs(5),
        mtud_discovery: args.mtud,
    };

    let counters = |extra: String| {
        format!(
            "mode={} ok=true {} duration_ms={}",
            args.mode,
            extra,
            started.elapsed().as_millis()
        )
    };

    let mut conn = match proto_client::connect(&cfg).await {
        Ok(conn) => conn,
        Err(e @ ClientError::Handshake(_)) if expected_handshake_failure => {
            let cause = if args.wrong_alpn { "wrong-alpn" } else { "wrong-pin" };
            return Ok(counters(format!("expected_failure={cause} detail=\"{e}\"")));
        }
        Err(e) => return Err(anyhow::anyhow!("handshake: {e}")),
    };

    match args.mode.as_str() {
        "happy" => {
            let session_id = match conn.auth(&cfg.credential).await {
                Ok(id) => id,
                Err(ClientError::AuthRejected { code, close_after })
                    if expected_auth_failure =>
                {
                    return Ok(counters(format!(
                        "expected_failure=auth-failure auth_err={code:?} close_after={close_after:?}"
                    )));
                }
                Err(e) => return Err(anyhow::anyhow!("auth: {e}")),
            };

            let media_start = conn.subscribe().await.context("subscribe")?;
            let mds = conn
                .max_datagram_size()
                .context("datagrams unsupported by server")?;
            let mut verifier = MediaVerifier::new(media_start.channels);
            let deadline = Instant::now() + Duration::from_millis(args.duration_ms.max(100));
            while Instant::now() < deadline {
                match tokio::time::timeout(
                    Duration::from_secs(2),
                    conn.read_datagram(),
                )
                .await
                {
                    Ok(Ok(dg)) => {
                        verifier.check(&dg, mds);
                        if args.stop_reading_ms > 0 && verifier.datagrams == 10 {
                            // Slow-reader demonstration: stop consuming.
                            tokio::time::sleep(Duration::from_millis(args.stop_reading_ms)).await;
                        }
                    }
                    Ok(Err(e)) => return Err(anyhow::anyhow!("read_datagram: {e}")),
                    Err(_) => break, // quiet period; time is up
                }
            }
            if verifier.datagrams < args.min_datagrams {
                return Err(anyhow::anyhow!(
                    "only {} datagrams received (need {})",
                    verifier.datagrams,
                    args.min_datagrams
                ));
            }

            conn.unsubscribe().await.context("unsubscribe")?;
            conn.send_close_normal().await.context("send CLOSE")?;
            let reason = conn
                .expect_close(Duration::from_secs(3))
                .await
                .context("expect CLOSE ack")?;
            anyhow::ensure!(
                reason == CloseReason::Normal,
                "expected CLOSE(Normal), got {reason:?}"
            );

            Ok(counters(format!(
                "session_id={session_id} instance={} mds={mds} datagrams={} frames={} bytes={} gaps={} duplicates={} \
                 pattern_mismatches={} timestamp_mismatches={} oversized={} misaligned={} max_datagram_len={} close_reason={reason:?}",
                media_start.stream_instance_id,
                verifier.datagrams,
                verifier.sample_frames,
                verifier.bytes,
                verifier.gaps,
                verifier.duplicates,
                verifier.pattern_mismatches,
                verifier.timestamp_mismatches,
                verifier.oversized,
                verifier.misaligned,
                verifier.max_datagram_len,
            )))
        }
        "idle" => {
            // Resource-capture scenario: authenticated and READY, never
            // subscribed, then a normal client-initiated goodbye.
            let session_id = conn.auth(&cfg.credential).await.context("auth")?;
            let mds = conn
                .max_datagram_size()
                .context("datagrams unsupported by server")?;
            tokio::time::sleep(Duration::from_millis(args.idle_ms.max(1))).await;
            conn.send_close_normal().await.context("send CLOSE")?;
            let reason = conn
                .expect_close(Duration::from_secs(3))
                .await
                .context("expect CLOSE ack")?;
            anyhow::ensure!(
                reason == CloseReason::Normal,
                "expected CLOSE(Normal), got {reason:?}"
            );
            Ok(counters(format!(
                "session_id={session_id} mds={mds} datagrams=0 close_reason={reason:?}"
            )))
        }
        "hold" => {
            conn.auth(&cfg.credential).await.context("auth")?;
            conn.subscribe().await.context("subscribe")?;
            let reason = conn
                .expect_close(Duration::from_secs(30))
                .await
                .context("waiting for server-initiated close")?;
            let (code, text) = conn
                .wait_transport_closed(Duration::from_secs(2))
                .await
                .context("transport close")?;
            Ok(counters(format!(
                "close_reason={reason:?} transport_code={code} transport_text={text}"
            )))
        }
        "no-auth-subscribe" => {
            conn.send_frame(&Frame::Subscribe)
                .await
                .context("send SUBSCRIBE")?;
            let mut saw_auth_err = None;
            loop {
                match conn.read_frame().await {
                    Ok(Frame::AuthErr(code)) => saw_auth_err = Some(code),
                    Ok(Frame::Close(reason)) => {
                        anyhow::ensure!(
                            saw_auth_err == Some(AuthErrCode::NotAuthenticated),
                            "expected AUTH_ERR(NOT_AUTHENTICATED) before CLOSE, saw {saw_auth_err:?}"
                        );
                        anyhow::ensure!(
                            reason == CloseReason::ProtocolError,
                            "expected CLOSE(PROTOCOL_ERROR), got {reason:?}"
                        );
                        return Ok(counters(format!(
                            "auth_err={:?} close_reason={reason:?}",
                            saw_auth_err.unwrap()
                        )));
                    }
                    Ok(other) => return Err(anyhow::anyhow!("unexpected frame {other:?}")),
                    Err(e) => return Err(anyhow::anyhow!("read frame: {e}")),
                }
            }
        }
        "bad-msg-type" => {
            conn.auth(&cfg.credential).await.context("auth")?;
            // Unknown msg_type 0x99, one-byte empty payload.
            conn.send_raw(&[0, 0, 0, 1, 0x99])
                .await
                .context("send unknown type")?;
            let reason = conn
                .expect_close(Duration::from_secs(3))
                .await
                .context("expect CLOSE")?;
            anyhow::ensure!(
                reason == CloseReason::ProtocolError,
                "expected CLOSE(PROTOCOL_ERROR), got {reason:?}"
            );
            Ok(counters(format!("close_reason={reason:?}")))
        }
        other => Err(anyhow::anyhow!(
            "unknown mode {other:?} (happy|hold|idle|no-auth-subscribe|bad-msg-type)"
        )),
    }
}
