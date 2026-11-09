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
use std::path::Path;
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use audio_share_core::capture::{
    platform_backend, synthetic_backend, CaptureBackend, CaptureConfig, EncodingPreference,
};
use audio_share_core::paths;
use audio_share_core::protocol::{HEARTBEAT_INTERVAL, SESSION_TIMEOUT};
use audio_share_core::server::{default_host, Server, ServerConfig};
use clap::{ArgAction, CommandFactory, FromArgMatches, Parser};
use tracing::Level;
use tracing_subscriber::filter::{filter_fn, LevelFilter};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields, MakeWriter};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

/// v0.3.4 hard-codes port 65530 when the bind value has no port.
const DEFAULT_PORT: u16 = 65_530;

/// Homepage printed by `-v/--version`.
const HOMEPAGE: &str = "https://github.com/mkckr0/audio-share";

/// Binary version (`0.4.0` for the rewrite).
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
}

/// The action selected by the flags, in v0.3.4 dispatch order:
/// version > list-endpoint > list-encoding > bind > help.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Help,
    ListEndpoints,
    ListEncoding,
    StartServer,
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
         as-cmd -l\n  as-cmd --list-encoding"
    )
}

fn select_action(cli: &Cli) -> Action {
    if cli.version {
        // Version is handled inside run; it wins over the other flags.
        Action::Help
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

async fn start_server(cli: Cli, host: String, port: u16) -> Result<(), String> {
    let capture: Arc<dyn CaptureBackend> = if cli.fake_capture {
        Arc::new(synthetic_backend().map_err(|error| error.to_string())?)
    } else {
        platform_backend().map_err(|error| error.to_string())?
    };

    let request = CaptureConfig {
        endpoint_id: cli.endpoint.clone(),
        encoding: encoding_preference(&cli.encoding),
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
            host,
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

    tokio::signal::ctrl_c()
        .await
        .map_err(|error| format!("install Ctrl-C handler: {error}"))?;
    tracing::info!("stopping server");
    handle
        .stop()
        .await
        .map_err(|error| format!("cannot stop the server: {error}"))?;
    tracing::info!("server stopped");
    Ok(())
}

fn encoding_preference(value: &str) -> EncodingPreference {
    match value {
        "f32" => EncodingPreference::F32,
        "s8" => EncodingPreference::S8,
        "s16" => EncodingPreference::S16,
        "s24" => EncodingPreference::S24,
        "s32" => EncodingPreference::S32,
        _ => EncodingPreference::Default,
    }
}

/// Parse v0.3.4 `[host][:port]` bind syntax.
///
/// A bare flag or an empty host selects the default private IPv4 address and,
/// when no port follows, port 65530. Ports are validated instead of wrapping
/// modulo 65536 (an intentional safety fix documented in `as-cmd.md`).
fn parse_bind(raw: &str) -> Result<(String, u16), String> {
    let (host, port_text) = match raw.split_once(':') {
        Some((host, port_text)) => (host, Some(port_text)),
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

fn open_log_file(path: &Path) -> std::io::Result<std::fs::File> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    // Truncate on every launch, matching the MFC GUI's basic_logger_mt usage.
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
        assert_eq!(encoding_preference("default"), EncodingPreference::Default);
        assert_eq!(encoding_preference("f32"), EncodingPreference::F32);
        assert_eq!(encoding_preference("s8"), EncodingPreference::S8);
        assert_eq!(encoding_preference("s16"), EncodingPreference::S16);
        assert_eq!(encoding_preference("s24"), EncodingPreference::S24);
        assert_eq!(encoding_preference("s32"), EncodingPreference::S32);
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
}
