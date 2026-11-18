//! `as-cmd` command-line entry point.
//!
//! `as-cmd` preserves the v0.3.4 flags and defaults: `-b/--bind` starts the
//! Server with the platform capture backend and runs until Ctrl-C; `-l` lists
//! endpoints; `--list-encoding` lists encodings; `-h`, `-v` and `-V` print
//! help, version, and enable trace logging. On Windows the backend is WASAPI
//! shared-mode loopback and on Linux PipeWire monitor capture; other platforms
//! use the deterministic synthetic backend. `--fake-capture` forces synthetic
//! capture for tests.
//!
//! Logging goes to stdout and to `server.log` next to the resolved config file
//! (see [`audio_share_core::paths`]). The old CLI only logged to stdout; the
//! file sink is an intentional improvement required by the rewrite spec.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use audio_share_core::capture::media::MediaTap;
use audio_share_core::capture::{
    platform_backend, synthetic_backend, CaptureBackend, CaptureConfig, CaptureObserver,
};
use audio_share_core::config::{EncodingSetting, DEFAULT_PORT};
use audio_share_core::paths;
use audio_share_core::protocol::{HEARTBEAT_INTERVAL, SESSION_TIMEOUT};
use audio_share_core::server::{default_host, Server, ServerConfig};
use clap::{ArgAction, CommandFactory, FromArgMatches, Parser, Subcommand};
use qrcodegen::{QrCode, QrCodeEcc};
use tracing::Level;
use tracing_subscriber::filter::{filter_fn, LevelFilter};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields, MakeWriter};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;
use v2_runtime::admin::RequestedPairingMode;
use v2_runtime::{RuntimeConfig, V2Runtime};
use v2_transport::auth_store::{AuthorizationStore, StoreLock};
use v2_transport::identity::IdentityConfig;
use v2_transport::pairing;

/// Homepage printed by `-v/--version`.
const HOMEPAGE: &str = "https://github.com/mkckr0/audio-share";

/// Binary version, from the crate metadata.
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Parser)]
#[command(
    name = "as-cmd",
    about = "Audio Share Server command line",
    disable_version_flag = true
)]
struct Cli {
    /// List available endpoints
    #[arg(short = 'l', long = "list-endpoint")]
    list_endpoint: bool,

    /// The server bind address: [host][:port]. Default port: 65530. A bare
    /// -b/--bind uses the default network address; an empty host does too.
    #[arg(
        short = 'b',
        long = "bind",
        value_name = "host[:port]",
        num_args = 0..=1,
        default_missing_value = ""
    )]
    bind: Option<String>,

    /// Specify the endpoint id. If not set or set "default", will use default
    #[arg(
        short = 'e',
        long = "endpoint",
        value_name = "id|default",
        default_value = "default"
    )]
    endpoint: String,

    /// Specify the capture encoding. If not set or set "default", will use default
    #[arg(
        long = "encoding",
        value_name = "default|f32|s8|s16|s24|s32",
        default_value = "default",
        value_parser = ["default", "f32", "s8", "s16", "s24", "s32"]
    )]
    encoding: String,

    /// List available encoding
    #[arg(long = "list-encoding")]
    list_encoding: bool,

    /// Specify the capture channels. If not set or set "0", will use default
    #[arg(long = "channels", value_name = "channels", default_value_t = 0)]
    channels: u16,

    /// Specify the capture sample rate(Hz). If not set or set "0", will use
    /// default. The common values are 44100, 48000, etc.
    #[arg(long = "sample-rate", value_name = "sample_rate", default_value_t = 0)]
    sample_rate: u32,

    /// Set log level to "trace"
    #[arg(short = 'V', long = "verbose", action = ArgAction::SetTrue)]
    verbose: bool,

    /// Show version
    #[arg(short = 'v', long = "version", action = ArgAction::SetTrue)]
    version: bool,

    /// Use deterministic synthetic capture instead of the platform backend
    /// (testing aid; not present in v0.3.4).
    #[arg(long = "fake-capture")]
    fake_capture: bool,

    /// The v2 (QUIC, wire-protocol v2) listen port. Default 65531, parallel
    /// to v1's 65530 (spec D1). 0 disables the v2 transport entirely; v1 is
    /// unaffected either way.
    #[arg(
        long = "v2-port",
        value_name = "port",
        default_value_t = v2_transport::codes::DEFAULT_V2_PORT,
        global = true
    )]
    v2_port: u16,

    /// POSIX override (pairing.md §10.2): enable v2 despite loose
    /// identity-key permissions (discouraged; logged). Windows warns only.
    #[arg(long = "allow-loose-key-permissions", global = true)]
    allow_loose_key_permissions: bool,

    /// Headless v2 pairing and administration (does not start the v1 Server).
    #[command(subcommand)]
    command: Option<Commands>,
}

/// Headless v2 administration verbs. Absent for the v0.3.4-compatible paths.
#[derive(Debug, Clone, Subcommand)]
enum Commands {
    /// Begin a foreground pairing session on the v2 listener.
    ///
    /// Pairing is local: run this on the Server computer. This command owns
    /// the v2 UDP port for the pairing, so stop a running Server first if it
    /// already holds the port. It does not start the v1 Server.
    Pair(PairArgs),
    /// List or revoke paired Clients (offline; takes the headless lock).
    ///
    /// Run this while no Server Core is running; the lock also excludes
    /// concurrent as-cmd admin operations.
    Clients {
        /// The client operation.
        #[command(subcommand)]
        action: ClientsAction,
    },
    /// Server identity operations (offline; takes the headless lock).
    Identity {
        /// The identity operation.
        #[command(subcommand)]
        action: IdentityAction,
    },
}

/// `as-cmd pair` options.
#[derive(Debug, Clone, clap::Args)]
struct PairArgs {
    /// Omit QR output and open only the SAS admission window.
    #[arg(long = "sas")]
    sas_only: bool,
    /// Accept the first verified device without an interactive prompt.
    #[arg(long = "accept", alias = "yes")]
    accept: bool,
    /// Advertised `host:port` in the QR payload (default: detected LAN
    /// address and the actual v2 port).
    #[arg(long = "addr", value_name = "host:port")]
    addr: Option<String>,
    /// Seconds to wait for a device before exiting.
    #[arg(long = "timeout", value_name = "seconds", default_value_t = 600)]
    timeout: u64,
}

/// `as-cmd clients` verbs.
#[derive(Debug, Clone, Subcommand)]
enum ClientsAction {
    /// List authorized Clients with their audit state.
    List,
    /// Revoke one Client by canonical `client_id` (the record is kept).
    Revoke {
        /// Canonical 16-hex-character client id.
        client_id: String,
    },
}

/// `as-cmd identity` verbs.
#[derive(Debug, Clone, Subcommand)]
enum IdentityAction {
    /// Generate a new Server identity and invalidate every old pairing.
    Reset {
        /// Skip the interactive confirmation.
        #[arg(long = "yes", alias = "accept")]
        yes: bool,
    },
}

/// The action selected by the flags, in v0.3.4 dispatch order:
/// version > list-endpoint > list-encoding > bind > help.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Help,
    ListEndpoints,
    ListEncoding,
    StartServer,
    Command,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = match parse_cli() {
        Ok(cli) => cli,
        Err(exit_code) => return exit_code,
    };
    let action = select_action(&cli);
    // Logging is only installed when the server actually starts, so `--help`,
    // `--version`, `--list-encoding`, `-l` and the no-argument help path never
    // create or truncate a (possibly live) server.log.
    if matches!(action, Action::StartServer) {
        init_logging(cli.verbose);
    }

    match run(cli, action).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            // Errors go to stderr; the file sink still records them because
            // the stdout layer intentionally filters ERROR out.
            tracing::error!("{message}");
            eprintln!("as-cmd: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Parse the command line, honouring clap's help output and remapping usage
/// errors to v0.3.4's exit code 1.
fn parse_cli() -> Result<Cli, ExitCode> {
    match build_command().try_get_matches() {
        Ok(matches) => Ok(
            Cli::from_arg_matches(&matches).expect("clap matches always map to the derived CLI")
        ),
        Err(error) => {
            let success = !error.use_stderr();
            let _ = error.print();
            Err(if success {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            })
        }
    }
}

fn build_command() -> clap::Command {
    Cli::command().after_help(help_examples())
}

/// The old help's examples, with the machine's default address interpolated.
fn help_examples() -> String {
    let default_address = default_host()
        .map(|address| address.to_string())
        .unwrap_or_else(|| "192.168.3.2".to_owned());
    format!(
        "Example:\n  as-cmd -b\n  as-cmd --bind={default_address}\n  \
         as-cmd --bind={default_address} --encoding=f32 --channels=2 --sample-rate=48000\n  \
         as-cmd -l\n  as-cmd --list-encoding\n  \
         as-cmd pair\n  as-cmd pair --sas\n  \
         as-cmd clients list\n  as-cmd clients revoke <client_id>\n  \
         as-cmd identity reset --yes\n\n\
         Status:\n  \
         v1 (TCP/UDP -b, the default) is UNENCRYPTED and UNAUTHENTICATED:\n  \
         use it only on a trusted network.\n  \
         v2 (QUIC, --v2-port) is encrypted and requires explicit pairing;\n  \
         --v2-port=0 disables v2 without affecting v1."
    )
}

fn select_action(cli: &Cli) -> Action {
    if cli.version {
        // Version is handled inside run; it wins over the other flags.
        Action::Help
    } else if cli.command.is_some() {
        Action::Command
    } else if cli.list_endpoint {
        Action::ListEndpoints
    } else if cli.list_encoding {
        Action::ListEncoding
    } else if cli.bind.is_some() {
        Action::StartServer
    } else {
        Action::Help
    }
}

async fn run(cli: Cli, action: Action) -> Result<(), String> {
    if cli.version {
        println!("as-cmd\nversion: {VERSION}\nurl: {HOMEPAGE}\n");
        return Ok(());
    }

    match action {
        Action::ListEncoding => {
            print_encoding_list();
            Ok(())
        }
        Action::Help => {
            // v0.3.4 prints help to stderr and exits 0 when there is no bind.
            let mut command = build_command();
            let _ = command.write_help(&mut std::io::stderr());
            Ok(())
        }
        Action::ListEndpoints => list_endpoints(),
        Action::Command => {
            let command = cli
                .command
                .clone()
                .expect("Action::Command is selected only when a subcommand exists");
            run_command(&cli, command).await
        }
        Action::StartServer => {
            let (host, port) = parse_bind(cli.bind.as_deref().unwrap_or_default())?;
            start_server(cli, host, port).await
        }
    }
}

fn print_encoding_list() {
    print!(
        "encoding list:\n\
         \tdefault\t\tDefault encoding\n\
         \tf32\t\t32 bit floating-point PCM\n\
         \ts8\t\t8 bit integer PCM\n\
         \ts16\t\t16 bit integer PCM\n\
         \ts24\t\t24 bit integer PCM\n\
         \ts32\t\t32 bit integer PCM\n"
    );
    let _ = std::io::stdout().flush();
}

fn list_endpoints() -> Result<(), String> {
    let backend =
        platform_backend().map_err(|error| format!("cannot open the audio backend: {error}"))?;
    let endpoints = backend
        .endpoints()
        .map_err(|error| format!("cannot list audio endpoints: {error}"))?;
    let default_id = backend
        .default_endpoint()
        .map_err(|error| format!("cannot read the default audio endpoint: {error}"))?
        .map(|endpoint| endpoint.id);

    let mut output = String::from("endpoint list:\n");
    for endpoint in &endpoints {
        let marker = if Some(&endpoint.id) == default_id.as_ref() {
            '*'
        } else {
            ' '
        };
        output.push_str(&format!(
            "\t{} id: {:<4} name: {}\n",
            marker, endpoint.id, endpoint.name
        ));
    }
    output.push_str(&format!("total: {}\n", endpoints.len()));
    print!("{output}");
    std::io::stdout()
        .flush()
        .map_err(|error| format!("write endpoint list: {error}"))
}

/// The Server data directory: the directory holding the resolved config file.
/// Identity and authorization files live below it (pairing.md §10.2).
fn data_dir() -> PathBuf {
    paths::paths()
        .config
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Dispatch a headless v2 administration command.
async fn run_command(cli: &Cli, command: Commands) -> Result<(), String> {
    match command {
        Commands::Pair(args) => run_pair(cli, args).await,
        Commands::Clients { action } => run_clients(action),
        Commands::Identity { action } => run_identity(cli, action),
    }
}

/// Take the offline admin lock without blocking. A running Core owns the
/// store for its lifetime, so the operator is told to stop it (pairing.md
/// §10.3) instead of hanging behind an exclusive lock.
fn offline_store_lock(data_dir: &std::path::Path) -> Result<StoreLock, String> {
    match StoreLock::try_acquire(data_dir).map_err(|error| error.to_string())? {
        Some(lock) => Ok(lock),
        None => Err(
            "the Server is running and owns the authorization store; stop the Server first"
                .to_owned(),
        ),
    }
}

/// `as-cmd clients list|revoke`: offline admin under the headless lock.
fn run_clients(action: ClientsAction) -> Result<(), String> {
    let data_dir = data_dir();
    let _lock = offline_store_lock(&data_dir)?;
    let store = AuthorizationStore::open(&data_dir).map_err(|error| error.to_string())?;
    match action {
        ClientsAction::List => {
            let clients = store.clients();
            if clients.is_empty() {
                println!("no paired clients");
                return Ok(());
            }
            println!("client_id\tstate\tdisplay_name\tcreated_at\tlast_seen_at");
            for client in clients {
                println!(
                    "{}\t{}\t{}\t{}\t{}",
                    client.client_id,
                    match client.state {
                        v2_transport::auth_store::ClientState::Active => "active",
                        v2_transport::auth_store::ClientState::Revoked => "revoked",
                    },
                    client.display_name,
                    if client.created_at.is_empty() {
                        "-"
                    } else {
                        &client.created_at
                    },
                    if client.last_seen_at.is_empty() {
                        "-"
                    } else {
                        &client.last_seen_at
                    },
                );
            }
            Ok(())
        }
        ClientsAction::Revoke { client_id } => {
            let revoked = store
                .revoke_client(&client_id)
                .map_err(|error| error.to_string())?;
            println!(
                "revoked client {} ({})",
                revoked.client_id, revoked.display_name
            );
            println!("new connections are refused immediately; a live session closes within 5 s");
            // Ticket 33: offline admin runs without the truncating server
            // logger, so append the audit line instead of installing it.
            append_line(
                &paths::paths().log,
                &format!("event=client.revoked client_id={}", revoked.client_id),
            );
            Ok(())
        }
    }
}

/// `as-cmd identity reset [--yes]`: clear pairings, then rotate the key.
fn run_identity(cli: &Cli, action: IdentityAction) -> Result<(), String> {
    let IdentityAction::Reset { yes } = action;
    let data_dir = data_dir();
    let _lock = offline_store_lock(&data_dir)?;
    let store = AuthorizationStore::open(&data_dir).map_err(|error| error.to_string())?;
    println!("WARNING: this generates a new Server identity key.");
    println!("Every paired device is invalidated and must pair again; there is no");
    println!("universal recovery backdoor. Fail-closed order: client records are");
    println!("cleared first, then the identity key is rotated atomically.");
    if !yes
        && !confirm(
            "Type 'yes' to reset the Server identity: ",
            accept_identity_reset,
        )?
    {
        return Err("identity reset cancelled".to_owned());
    }
    let config = IdentityConfig {
        data_dir: data_dir.clone(),
        allow_loose_permissions: cli.allow_loose_key_permissions,
    };
    let identity = pairing::reset_identity(&config, &store).map_err(|error| error.to_string())?;
    println!("identity reset complete; old pairings invalidated");
    println!("Server fingerprint: {}", identity.fingerprint());
    // Ticket 33: append-only audit; the server logger would truncate.
    append_line(
        &paths::paths().log,
        &format!(
            "event=identity.reset fingerprint={}",
            identity.fingerprint()
        ),
    );
    Ok(())
}

/// Ask a question on stdin (blocking read off the async runtime) and apply
/// the caller's comparison to the trimmed line.
fn confirm(prompt: &str, accept: impl Fn(&str) -> bool) -> Result<bool, String> {
    print!("{prompt}");
    std::io::stdout()
        .flush()
        .map_err(|error| format!("write prompt: {error}"))?;
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .map_err(|error| format!("read confirmation: {error}"))?;
    Ok(accept(line.trim()))
}

/// The identity reset requires the full word (a plain `y` is not enough).
fn accept_identity_reset(line: &str) -> bool {
    line.eq_ignore_ascii_case("yes")
}

/// The pairing prompt accepts the usual yes spellings, defaulting to no.
fn accept_pairing(line: &str) -> bool {
    matches!(line.to_ascii_lowercase().as_str(), "y" | "yes")
}

/// `as-cmd pair`: a foreground v2 pairing session.
///
/// The Server Core is not required to be running: this process owns the v2
/// listener for the duration of the pairing. `pair` opens both a QR nonce and
/// the SAS admission window; `pair --sas` opens only the SAS window and omits
/// QR output. Closing the process cancels the invitation.
async fn run_pair(cli: &Cli, args: PairArgs) -> Result<(), String> {
    if cli.v2_port == 0 {
        return Err("pair needs a v2 port; --v2-port=0 disables the v2 transport".to_owned());
    }
    // Pairing does not serve media, so no capture observer is attached. The
    // shared runtime still owns the listener, identity, store, and Pairing
    // Core — exactly what the GUI starts.
    let runtime = V2Runtime::start(
        RuntimeConfig {
            data_dir: data_dir(),
            allow_loose_permissions: cli.allow_loose_key_permissions,
            bind_host: "::".to_owned(),
            port: cli.v2_port,
            addr_hint: args.addr.clone(),
        },
        None,
    )
    .await
    .map_err(|error| error.to_string())?;
    let core = runtime.pairing();
    let admin = runtime.admin();

    let invitation = admin
        .begin(if args.sas_only {
            RequestedPairingMode::Sas
        } else {
            RequestedPairingMode::Qr
        })
        .map_err(|error| error.to_string())?;
    if args.sas_only {
        println!("Pairing mode: SAS (no QR; the phone connects manually)");
    } else {
        println!("Pairing mode: QR (SAS fallback window is also open)");
        let uri = invitation
            .qr_uri
            .ok_or_else(|| "cannot build the QR payload".to_owned())?;
        println!("Pairing URI: {uri}");
        print_qr(&uri);
    }
    println!("Server fingerprint: {}", runtime.fingerprint());
    println!("Pairing code: printed here as soon as the phone connects.");
    println!(
        "Waiting up to {} s for a device; press Ctrl-C to cancel.",
        args.timeout
    );
    println!("The invitation is single-use; if pairing fails, run `as-cmd pair` again.");
    let _ = std::io::stdout().flush();

    let mut approvals = core.subscribe_approvals();
    let mut presented = core.subscribe_presented();
    let mut paired = core.subscribe_paired();
    let deadline = tokio::time::sleep(std::time::Duration::from_secs(args.timeout));
    tokio::pin!(deadline);
    let mut paired_ok = false;
    let outcome = loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!(
                    "\npairing cancelled; the invitation is single-use, so run `as-cmd pair` again if you want to retry"
                );
                break Ok(());
            }
            _ = &mut deadline => {
                break Err(
                    "timed out waiting for a device; the invitation is single-use, so run `as-cmd pair` again"
                        .to_owned(),
                );
            }
            result = paired.recv() => {
                match result {
                    Ok(client) => {
                        println!(
                            "Paired: {} ({}) — stored in the authorization store",
                            client.display_name, client.client_id
                        );
                        paired_ok = true;
                        break Ok(());
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => continue,
                }
            }
            result = presented.recv() => {
                let request = match result {
                    Ok(request) => request,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => continue,
                };
                // The live SAS is available as soon as the Server has written
                // PairingResponse; the phone needs it to confirm (ticket 29).
                println!(
                    "\nDevice connected: {} [{}] from {}",
                    request.display_name, request.client_id, request.address
                );
                if let Some(sas) = request.sas.as_deref() {
                    println!("Pairing code: {sas}");
                }
                println!("Compare this code with the phone before accepting.");
                let _ = std::io::stdout().flush();
            }
            result = approvals.recv() => {
                let request = match result {
                    Ok(request) => request,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => continue,
                };
                println!(
                    "\nIncoming device: {} [{}] from {}",
                    request.display_name, request.client_id, request.address
                );
                if let Some(sas) = request.sas.as_deref() {
                    println!("Pairing code: {sas}");
                }
                println!("Compare the code with the phone before accepting.");
                let _ = std::io::stdout().flush();
                let accepted = if args.accept {
                    true
                } else {
                    confirm("Accept this device? [y/N]: ", accept_pairing)?
                };
                if accepted {
                    core.approve(request.approval_id);
                    println!("Accepted; waiting for the phone to finish pairing...");
                } else {
                    core.decline(request.approval_id);
                    println!(
                        "Declined; nothing was stored. The invitation is single-use; run `as-cmd pair` again if you want to retry."
                    );
                }
            }
        }
    };

    if paired_ok {
        // The driver announces a pairing only after writing PairingResult.
        // Wait for the Client to consume it and close its session before the
        // endpoint closes, so the success frame is never discarded. This is
        // an event-driven wait (bounded only for a persistently open Client),
        // not a fixed delay.
        let _ = runtime
            .wait_for_idle(std::time::Duration::from_secs(2))
            .await;
    }
    admin.cancel();
    runtime.stop();
    outcome
}

/// Render a QR as compact UTF-8 half-block terminal art (4-module quiet zone).
fn print_qr(text: &str) {
    let qr = match QrCode::encode_text(text, QrCodeEcc::Medium) {
        Ok(qr) => qr,
        Err(error) => {
            println!("(QR rendering unavailable: {error}; use the URI above)");
            return;
        }
    };
    const BORDER: i32 = 4;
    let size = qr.size() + BORDER * 2;
    let dark = |x: i32, y: i32| {
        let x = x - BORDER;
        let y = y - BORDER;
        x >= 0 && y >= 0 && x < qr.size() && y < qr.size() && qr.get_module(x, y)
    };
    let mut y = 0;
    while y < size {
        let mut line = String::with_capacity(size as usize * 3);
        for x in 0..size {
            let upper = dark(x, y);
            let lower = y + 1 < size && dark(x, y + 1);
            line.push(match (upper, lower) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => ' ',
            });
        }
        println!("{line}");
        y += 2;
    }
}

async fn start_server(cli: Cli, host: String, port: u16) -> Result<(), String> {
    let capture: Arc<dyn CaptureBackend> = if cli.fake_capture {
        Arc::new(synthetic_backend().map_err(|error| error.to_string())?)
    } else {
        platform_backend().map_err(|error| error.to_string())?
    };

    let encoding = EncodingSetting::from_name(&cli.encoding)
        .ok_or_else(|| format!("unsupported encoding {:?}", cli.encoding))?;
    let request = CaptureConfig {
        endpoint_id: cli.endpoint.clone(),
        encoding: encoding.preference(),
        channels: cli.channels,
        sample_rate: cli.sample_rate,
    };
    tracing::info!(
        "starting server: bind={}, endpoint={:?}, encoding={}, channels={}, sample_rate={}",
        format_args!("{host}:{port}"),
        cli.endpoint,
        cli.encoding,
        cli.channels,
        cli.sample_rate
    );

    let handle = Server::start(
        ServerConfig {
            host: host.clone(),
            port,
            heartbeat_interval: HEARTBEAT_INTERVAL,
            heartbeat_timeout: SESSION_TIMEOUT,
        },
        capture,
        request,
    )
    .await
    .map_err(|error| format!("cannot start the server: {error}"))?;

    let address = handle.local_addr();
    tracing::info!("tcp listen success on {address}");
    tracing::info!("udp listen success on {address}");
    tracing::info!("server started");
    let format = handle.format();
    tracing::info!(
        "capture format: encoding={:?}, channels={}, sample_rate={}",
        format.encoding_kind(),
        format.channels,
        format.sample_rate
    );

    // The v2 (QUIC) transport: enabled by default on 65531, independently
    // disableable (`--v2-port=0`). v2 failures never take down the v1 server
    // (pairing.md §10.2: "v1 and capture are unaffected").
    let v2 = if cli.v2_port == 0 {
        tracing::info!("v2 transport disabled (--v2-port=0); v1 TCP/UDP unaffected");
        None
    } else {
        match start_v2_transport(
            &cli,
            Some((handle.capture_observer(), handle.media_tap().clone())),
        )
        .await
        {
            Ok(runtime) => {
                tracing::info!(
                    "v2 listen success on {}, alpn={}",
                    runtime.local_addr(),
                    String::from_utf8_lossy(v2_transport::codes::ALPN_PROTOCOL)
                );
                Some(runtime)
            }
            Err(reason) => {
                tracing::error!("v2 transport not enabled: {reason}");
                None
            }
        }
    };

    tokio::signal::ctrl_c()
        .await
        .map_err(|error| format!("install Ctrl-C handler: {error}"))?;
    tracing::info!("stopping server");
    if let Some(runtime) = v2 {
        runtime.stop();
        tracing::info!("v2 listener stopped");
    }
    handle
        .stop()
        .await
        .map_err(|error| format!("cannot stop the server: {error}"))?;
    tracing::info!("server stopped");
    Ok(())
}

/// Start the shared v2 runtime for the long-running Server.
///
/// The runtime loads the Ed25519 identity key, opens the authorization store
/// (fail-closed on corruption), starts the shared Pairing Core, binds the QUIC
/// endpoint, and runs the accept loop. `as-cmd` and the GUI use this exact
/// assembly; a failure is reported so v1 can stay running.
async fn start_v2_transport(
    cli: &Cli,
    capture: Option<(CaptureObserver, MediaTap)>,
) -> Result<V2Runtime, String> {
    if cli.allow_loose_key_permissions {
        tracing::warn!(
            "v2 identity-key permissions are loose and the override is active (--allow-loose-key-permissions); this is discouraged (pairing.md §10.2)"
        );
    }
    let runtime = V2Runtime::start(
        RuntimeConfig {
            data_dir: data_dir(),
            allow_loose_permissions: cli.allow_loose_key_permissions,
            bind_host: "::".to_owned(),
            port: cli.v2_port,
            addr_hint: None,
        },
        capture,
    )
    .await
    .map_err(|error| error.to_string())?;
    tracing::info!("v2 identity ready: fingerprint={}", runtime.fingerprint());
    let store = runtime.pairing().store();
    let authorized = store.clients().len();
    tracing::info!(
        "v2 authorization store ready: {authorized} client record(s), {}",
        if store.is_persistent() {
            "persistent"
        } else {
            "in-memory"
        }
    );
    Ok(runtime)
}

/// Parse `[host][:port]`, including raw and bracketed IPv6. A bare flag or
/// empty host retains v1's default private IPv4 address and port 65530; the
/// v2 resolver owns its own :: wildcard default.
fn parse_bind(raw: &str) -> Result<(String, u16), String> {
    if raw.parse::<std::net::Ipv6Addr>().is_ok() {
        return Ok((raw.to_owned(), DEFAULT_PORT));
    }
    if let Some(rest) = raw.strip_prefix('[') {
        let (host, suffix) = rest
            .split_once(']')
            .ok_or_else(|| format!("invalid bracketed bind address {raw:?}"))?;
        host.parse::<std::net::Ipv6Addr>()
            .map_err(|_| format!("invalid IPv6 bind address {raw:?}"))?;
        let port = match suffix.strip_prefix(':') {
            Some(text) => text
                .parse::<u16>()
                .map_err(|_| format!("invalid port {text:?}"))?,
            None if suffix.is_empty() => DEFAULT_PORT,
            _ => return Err(format!("invalid bind address {raw:?}")),
        };
        return Ok((host.to_owned(), port));
    }
    let (host, port_text) = match raw.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => (host, Some(port)),
        Some(_) => return Err(format!("invalid bind address {raw:?}")),
        None => (raw, None),
    };
    let host = if host.is_empty() {
        default_host()
            .map(|address| address.to_string())
            .ok_or_else(|| "no default network address; use --bind=host:port".to_owned())?
    } else {
        host.to_owned()
    };
    let port = match port_text {
        None => DEFAULT_PORT,
        Some(text) => text.parse::<u16>().map_err(|_| {
            format!(
                "invalid port {text:?} in bind address {raw:?}: expected an integer from 0 to 65535"
            )
        })?,
    };
    Ok((host, port))
}

/// Install stdout and `server.log` sinks with the v0.3.4-style line format.
fn init_logging(verbose: bool) {
    let level = if verbose {
        LevelFilter::TRACE
    } else {
        LevelFilter::INFO
    };
    let paths = paths::paths();
    let file_layer = match open_log_file(&paths.log) {
        Ok(file) => Some(
            tracing_subscriber::fmt::layer()
                .with_writer(LogFile::new(file))
                .with_ansi(false)
                .event_format(LegacyFormat)
                .with_filter(level),
        ),
        Err(error) => {
            eprintln!(
                "as-cmd: warning: cannot open log file {}: {error}",
                paths.log.display()
            );
            None
        }
    };
    // Errors are reported on stderr by main; keep them out of the stdout sink
    // so a failure is not printed twice.
    let max_level = if verbose { Level::TRACE } else { Level::INFO };
    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stdout)
        .with_ansi(false)
        .event_format(LegacyFormat)
        .with_filter(filter_fn(move |metadata| {
            *metadata.level() != Level::ERROR && *metadata.level() <= max_level
        }));
    let _ = tracing_subscriber::registry()
        .with(stdout_layer)
        .with(file_layer)
        .try_init();
    tracing::trace!("verbose logging enabled (trace level)");
}

/// Append one `[info]` line to `path`, keeping everything already there.
///
/// Offline admin commands (`clients revoke`, `identity reset`) refuse to run
/// while a Server owns the store, so no live writer competes for the file.
/// They must not call [`init_logging`] — that truncates the file and would
/// erase the previous session's events. Support still needs these events in
/// the same log, so the line is appended directly.
fn append_line(path: &Path, message: &str) {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = std::fs::create_dir_all(parent);
        }
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "[{}] [info] {}", utc_timestamp(), message);
    }
}

fn open_log_file(path: &Path) -> std::io::Result<std::fs::File> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    // Truncate on every launch, matching the legacy Windows GUI's
    // basic_logger_mt usage.
    std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
}

/// A shareable log file writer for tracing's [`MakeWriter`] seam.
#[derive(Clone)]
struct LogFile(Arc<Mutex<std::fs::File>>);

impl LogFile {
    fn new(file: std::fs::File) -> Self {
        Self(Arc::new(Mutex::new(file)))
    }
}

impl<'a> MakeWriter<'a> for LogFile {
    type Writer = LogFileGuard<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        LogFileGuard(
            self.0
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }
}

struct LogFileGuard<'a>(std::sync::MutexGuard<'a, std::fs::File>);

impl Write for LogFileGuard<'_> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        self.0.write(buffer)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

/// Formats events like v0.3.4's spdlog default logger:
/// `[2026-11-09 15:08:46.055] [info] message`.
struct LegacyFormat;

impl<S, N> FormatEvent<S, N> for LegacyFormat
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        context: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        write!(
            writer,
            "[{}] [{}] ",
            utc_timestamp(),
            event.metadata().level().as_str().to_ascii_lowercase()
        )?;
        context
            .field_format()
            .format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

/// `YYYY-MM-DD HH:MM:SS.mmm` in UTC, matching the old spdlog timestamp shape.
fn utc_timestamp() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = now.as_secs() as i64;
    let millis = now.subsec_millis();
    let days = seconds.div_euclid(86_400);
    let second_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = second_of_day / 3_600;
    let minute = (second_of_day % 3_600) / 60;
    let second = second_of_day % 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}.{millis:03}")
}

/// Convert days since 1970-01-01 to a civil (year, month, day).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = (days - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use as_cmd::v2_capture_snapshot;
    use audio_share_core::capture::EncodingPreference;
    use audio_share_core::protocol::{AudioFormat, Encoding};
    use v2_transport::schema::{Codec, SampleFormat};

    #[test]
    fn v2_snapshot_publishes_the_capture_format_as_pcm_configuration() {
        // §5.2: Subscribe/RequestConfiguration carry the live capture format
        // as a CODEC_PCM Configuration (generation 1: first, decode-affecting
        // state of this capture instance). No stream is active in this phase.
        for (encoding, expected) in [
            (Encoding::PcmFloat, SampleFormat::F32Le),
            (Encoding::Pcm8Bit, SampleFormat::U8),
            (Encoding::Pcm16Bit, SampleFormat::S16Le),
            (Encoding::Pcm24Bit, SampleFormat::S24Le),
            (Encoding::Pcm32Bit, SampleFormat::S32Le),
        ] {
            let snapshot = v2_capture_snapshot(&AudioFormat::new(encoding, 2, 48_000));
            let configuration = snapshot.configuration.expect("configuration published");
            assert_eq!(configuration.config_generation, 1);
            assert_eq!(configuration.codec, Codec::Pcm as i32);
            let pcm = configuration.pcm.expect("pcm parameters");
            assert_eq!((pcm.sample_rate, pcm.channels), (48_000, 2));
            assert_eq!(pcm.sample_format, expected as i32);
            assert!(snapshot.healthy);
            assert!(!snapshot.stream_active, "no media stream in this phase");
        }
    }

    #[test]
    fn v2_snapshot_without_a_valid_format_is_capture_absent() {
        // Invalid encoding / non-positive geometry cannot form a valid
        // Configuration (§4.4), so Subscribe stays Error(501).
        for format in [
            AudioFormat::new(Encoding::Invalid, 2, 48_000),
            AudioFormat::new(Encoding::Pcm16Bit, 0, 48_000),
            AudioFormat::new(Encoding::Pcm16Bit, 2, 0),
        ] {
            assert!(v2_capture_snapshot(&format).configuration.is_none());
        }
    }

    #[tokio::test]
    async fn ipv6_bind_syntax_and_default_are_supported() {
        assert_eq!(parse_bind("::1").unwrap(), ("::1".to_owned(), DEFAULT_PORT));
        assert_eq!(
            parse_bind("[::1]:65080").unwrap(),
            ("::1".to_owned(), 65_080)
        );
        let bind = v2_runtime::resolve_bind_address("::", 65531).await.unwrap();
        assert_eq!(bind, "[::]:65531".parse().unwrap());
        assert!(bind.is_ipv6());
        for family in ["::1", "127.0.0.1"] {
            let bind = v2_runtime::resolve_bind_address(family, 0).await.unwrap();
            let socket = std::net::UdpSocket::bind(bind).expect("loopback listener");
            assert_eq!(socket.local_addr().unwrap().ip(), bind.ip());
        }
    }

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
    fn bare_and_empty_host_use_the_default_address() {
        let Some(default) = default_host() else {
            eprintln!("skipping: no default network address on this host");
            return;
        };
        let default = default.to_string();
        assert_eq!(parse_bind("").unwrap(), (default.clone(), DEFAULT_PORT));
        assert_eq!(parse_bind(":65080").unwrap(), (default, 65_080));
    }

    #[test]
    fn parse_bind_rejects_bad_ports_and_extra_colons() {
        for bind in [
            "127.0.0.1:abc",
            "127.0.0.1:65536",
            "127.0.0.1:99999",
            "127.0.0.1:-1",
            "127.0.0.1:",
            "127.0.0.1:65083:extra",
        ] {
            assert!(parse_bind(bind).is_err(), "{bind} must be rejected");
        }
    }

    #[test]
    fn encoding_names_map_to_capture_preferences() {
        for (name, preference) in [
            ("default", EncodingPreference::Default),
            ("f32", EncodingPreference::F32),
            ("s8", EncodingPreference::S8),
            ("s16", EncodingPreference::S16),
            ("s24", EncodingPreference::S24),
            ("s32", EncodingPreference::S32),
        ] {
            assert_eq!(
                EncodingSetting::from_name(name).map(EncodingSetting::preference),
                Some(preference),
                "{name}"
            );
        }
        assert_eq!(EncodingSetting::from_name("unknown"), None);
    }

    #[test]
    fn utc_timestamp_matches_the_legacy_shape() {
        let stamp = utc_timestamp();
        let bytes = stamp.as_bytes();
        assert_eq!(bytes.len(), 23, "{stamp}");
        assert_eq!(bytes[4], b'-');
        assert_eq!(bytes[7], b'-');
        assert_eq!(bytes[10], b' ');
        assert_eq!(bytes[13], b':');
        assert_eq!(bytes[16], b':');
        assert_eq!(bytes[19], b'.');
    }

    #[test]
    fn offline_audit_line_appends_without_truncating() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("server.log");
        std::fs::write(&path, "[2026-11-17 00:00:00.000] [info] server started\n")
            .expect("seed log");
        append_line(&path, "event=client.revoked client_id=cf7940b474310246");
        let text = std::fs::read_to_string(&path).expect("read log");
        assert!(
            text.contains("server started"),
            "the previous session's lines must survive: {text}"
        );
        assert!(
            text.contains("event=client.revoked client_id=cf7940b474310246"),
            "{text}"
        );
        assert!(
            text.lines()
                .last()
                .is_some_and(|line| line.starts_with('[')),
            "{text}"
        );
    }
}
