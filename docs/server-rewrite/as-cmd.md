# `as-cmd` — command-line interface

`as-cmd` is the headless Server Core entry point. It preserves the upstream
v0.3.4 flags and observable contract while adding a log file, graceful Ctrl-C,
and safe validation. It links `audio-share-core` directly, so it shares the
capture backends and Wire Protocol implementation with the Server GUI.

## Flags, values, defaults

| Short | Long | Value | Default | Notes |
|---|---|---|---|---|
| `-h` | `--help` | none | — | Prints help to stdout, exit 0. `--help` wins over every other flag. |
| `-l` | `--list-endpoint` | none | — | Lists real capture endpoints; marks the default with `*`; prints `total: N`. |
| `-b` | `--bind` | `[host][:port]` | bare flag → default address:65530 | Optional value. Accepts `--bind=value`, `-b value`, and bare `-b`/`--bind`. |
| `-e` | `--endpoint` | `id` or `default` | `default` | Empty or `default` selects the backend default endpoint. |
| | `--encoding` | `default`, `f32`, `s8`, `s16`, `s24`, `s32` | `default` | Unknown values are rejected at parse time. |
| | `--list-encoding` | none | — | Prints the six-line encoding list, exit 0. |
| | `--channels` | non-negative integer | `0` | `0` = endpoint default. Negative/non-numeric values are rejected. |
| | `--sample-rate` | non-negative integer (Hz) | `0` | `0` = endpoint default. Common values: 44100, 48000. |
| `-V` | `--verbose` | none | — | Sets the log level to `trace` (default `info`). |
| `-v` | `--version` | none | — | Prints version to stdout, exit 0. clap's built-in version flag is disabled so `-v` matches v0.3.4. |
| | `--fake-capture` | none | — | Testing aid (not in v0.3.4): uses deterministic synthetic capture instead of the platform backend. |

Default port: **65530**. Default bind host: the first address matching
v0.3.4's private-network masks (`10.x`, `172.16`–`172.31`, `192.168.0.0/16`,
plus the loose matches of the exact mask comparison), otherwise the first
non-loopback IPv4 address. When the machine has no such address, a bare `-b`
fails with a readable error.

Value forms: both `--bind=host:port` and `-b host:port` work. A host without a
port uses 65530. `--bind=:65080`, `--bind=` and bare `-b` use the default
address. Port `0` binds an ephemeral port.

## Dispatch precedence

Matches v0.3.4: help > version > list-endpoint > list-encoding > bind.
`--verbose` only changes logging and does not select an action. Therefore
`-v -l` prints the version, and `-l --list-encoding` lists endpoints.

With no action selected (no arguments, or options such as `--encoding=f32`
without `-b`), the full help is printed to **stderr** and the process exits
**0**, exactly like v0.3.4. (The architecture note that said exit 1 was
superseded by the parity requirement.)

## Exit codes

| Code | Meaning |
|---|---|
| `0` | Help, version, `--list-encoding`, successful `-l`, no-argument help, or a graceful Ctrl-C stop. |
| `1` | Unknown flag, missing/invalid value, bad bind/port, backend/list failure, server bind/capture/start failure. |
| other | Not produced: the rewrite neither segfaults nor calls `exit()` from library code. |

Parse errors print a readable message to stderr. Help and version print to
stdout.

## Examples

```text
$ as-cmd -v
as-cmd
version: 0.4.0
url: https://github.com/mkckr0/audio-share

$ as-cmd --list-encoding
encoding list:
	default		Default encoding
	f32		32 bit floating-point PCM
	s8		8 bit integer PCM
	s16		16 bit integer PCM
	s24		24 bit integer PCM
	s32		32 bit integer PCM

$ as-cmd -l
endpoint list:
	* id: 38   name: Test Sink
total: 1

$ as-cmd -b
$ as-cmd --bind=192.168.1.20
$ as-cmd --bind=192.168.1.20 --encoding=f32 --channels=2 --sample-rate=48000
$ as-cmd -b 192.168.1.20:65080 -e 38 --encoding=s16
```

The help output includes these examples with the machine's default address
interpolated at startup.

## Logging

When the server starts (`-b`), `as-cmd` writes the same lines to stdout and to
**`server.log`** in the resolved config directory. The file is truncated on
every server launch (matching the v0.3.4 GUI's log truncation behavior)
and is not touched by `--help`, `--version`, `--list-encoding`, or `-l`, so
listing endpoints cannot truncate a running server's log. The default level is
`info`; `-V/--verbose` raises it to `trace`.

Line format (UTC timestamps, v0.3.4 spdlog shape):

```text
[2026-11-09 15:08:46.055] [info] tcp listen success on 127.0.0.1:65052
```

Startup emits `starting server`, `tcp listen success on <addr>`,
`udp listen success on <addr>`, `server started`, and `capture format`; stop
emits `stopping server` and `server stopped`. Fatal errors are also written to
the file (the stdout sink filters them so they are not duplicated).

Unlike v0.3.4, `as-cmd` now writes `server.log`; the old `as-cmd` only logged to
stdout. This is the intentional improvement required by the rewrite spec.

### Config/log path resolution

The same rules as the Server GUI (`audio_share_core::paths`):

1. `AUDIO_SHARE_CONFIG` — path of the config file; `server.log` goes in that
   file's directory.
2. `config.ini` next to the executable, when that directory is writable.
3. `$XDG_CONFIG_HOME/audio-share-server/` (default
   `~/.config/audio-share-server/`).

`as-cmd` itself does not read `config.ini`; only the log location uses the
resolution. The Server GUI (ticket 09) reuses the same module for persistence.

## Intentional differences from v0.3.4

The flags, defaults, output shapes, and exit-code contract are preserved.
These deviations are deliberate and safe for existing scripts:

- **No PipeWire/audio backend** (`-l`): v0.3.4 segfaulted (exit 139). The
  rewrite prints a readable error and exits 1.
- **Unknown `--encoding`**: v0.3.4 accepted it and failed later, platform
  dependently (Linux exited 1 during capture; Windows kept serving without
  audio). The rewrite rejects it during argument parsing with exit 1.
- **Port validation**: v0.3.4 cast `std::stoi` to `uint16_t`, wrapping
  `99999 → 34463` and `65536 → 0`. The rewrite rejects non-numeric values and
  values above 65535 with a readable error. Port `0` remains valid.
- **`-b value` space form**: v0.3.4's cxxopts implicit value silently ignored a
  space-separated bind value and used the default address. The rewrite accepts
  both `-b value` and `--bind=value`.
- **Empty host**: `--bind=:65080` and `--bind=` now mean "default address:port"
  instead of v0.3.4's `Invalid argument`.
- **Extra colons**: `--bind=host:65083:extra` was accepted by v0.3.4 (`stoi`
  read the leading integer); the rewrite rejects it.
- **Unknown endpoint**: v0.3.4 kept serving forever with an empty format. The
  rewrite returns an actionable error and exits 1 at startup.
- **Negative channels/sample rate**: v0.3.4 crashed (SIGSEGV). The rewrite
  rejects them during parsing.
- **Default host selection is unchanged**: the rewrite applies v0.3.4's
  `(address & mask) == mask` comparison with masks `0x0a000000`, `0xac100000`,
  and `0xc0a80000`. That selection is intentionally loose (for example it can
  also match `11.x`, `173.16`–`173.31`, and `192.169`–`192.175`), exactly like
  the old build, so the default address does not change.
- **Ctrl-C**: v0.3.4 had no signal handler (default termination). The rewrite
  stops capture and the network side and exits 0.
- **`server.log`**: added (see above).
- **Version**: `0.4.0` instead of `0.3.4`.

## Platform notes

- **Linux**: endpoints are PipeWire `Audio/Sink` nodes; the default is the
  highest `priority.session` (first wins ties). `s8` capture now works (U8 →
  `ENCODING_PCM_8BIT`); v0.3.4 exited on that path.
- **Windows**: endpoints are active render (WASAPI) devices; the default is
  `GetDefaultAudioEndpoint(eRender, eConsole)`. In this development
  environment `as-cmd` was validated with
  `cargo check --target x86_64-pc-windows-msvc -p as-cmd` only; real Windows
  endpoint listing and capture remain runtime/manual verification.
- Other platforms fall back to synthetic capture (test builds only).

## Tests

- `crates/as-cmd/tests/cli.rs` — process-level flags, output, and exit codes
  (including the no-daemon `-l` error path).
- `crates/as-cmd/tests/server_process.rs` — live start with synthetic capture,
  v0.3.4 protocol-harness replay, SIGINT exit 0, session close, `server.log`
  creation/truncation/levels.
- `crates/as-cmd/tests/cli_pipewire.rs` — gated headless PipeWire session:
  real `-l`, real-backend protocol replay, graceful Ctrl-C. Skips cleanly when
  PipeWire/`dbus-run-session` are unavailable.
- `crates/audio-share-core/tests/paths.rs` — env override, writable executable
  directory, read-only fallback, XDG default.
