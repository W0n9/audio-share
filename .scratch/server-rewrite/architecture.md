# Server rewrite — target architecture (implementation contract)

This note is the shared design contract for implementers. It refines the spec; where it conflicts with a ticket, the spec wins. All paths below are in the repo worktree.

## Workspace layout

```
Cargo.toml                     # workspace, resolver 2, edition 2021
crates/audio-share-core/       # library: protocol, capture, server, config, platform
crates/as-cmd/                 # CLI binary (links core)
gui/                           # Tauri v2 app
  src-tauri/                   # Rust shell, workspace member
  src/                         # Svelte + Vite + TS
  package.json
tools/protocol-harness/        # scripted v0.3.4 client, fixture recorder/replayer, fake server
fixtures/protocol/v0.3.4/      # golden fixtures (linux real, windows synthesized)
scripts/                       # headless PipeWire, fixtures, soak, A/B, packaging helpers
.github/workflows/server-rewrite.yml
docs/server-rewrite/           # architecture, harness, build, release, acceptance, limitations
```

`cargo build` at the root must work before the frontend is built. Tauri's `frontendDist` points at a committed placeholder `gui/dist/` that `npm run build` overwrites; the placeholder is a minimal valid page. Do not make root builds depend on npm.

## Core public API (small, Result-based, no process exit)

```rust
pub struct AudioFormat { pub encoding: Encoding, pub channels: i32, pub sample_rate: i32 }
pub enum Encoding { Invalid=0, PcmFloat=1, Pcm8=2, Pcm16=3, Pcm24=4, Pcm32=5 }
// bits per sample: f32=32, s8=8, s16=16, s24=24, s32=32
// block_align = bits/8 * channels

pub struct Endpoint { pub id: String, pub name: String }

pub struct CaptureConfig {
    pub endpoint_id: String,      // "" or "default" => default endpoint
    pub encoding: EncodingPreference, // Default | F32 | S8 | S16 | S24 | S32
    pub channels: u16,            // 0 => endpoint default
    pub sample_rate: u32,         // 0 => endpoint default
}

pub trait CaptureBackend: Send + Sync + 'static {
    fn endpoints(&self) -> Result<Vec<Endpoint>>;
    fn default_endpoint(&self) -> Result<Option<Endpoint>>;
    fn start(&self, cfg: &CaptureConfig, sink: FrameSink) -> Result<CaptureSession>;
}
pub type FrameSink = tokio::sync::mpsc::Sender<Vec<u8>>;
pub struct CaptureSession { pub format: AudioFormat, /* stop handle */ }
// CaptureSession::stop(self) is idempotent and cancels the capture thread/stream.

pub struct ServerConfig {
    pub host: String,             // default: select_default_address()
    pub port: u16,                // default 65530; 0 allowed for tests (actual port reported)
    // test-only overrides, defaults 3s/5s:
    pub heartbeat_interval: Duration,
    pub heartbeat_timeout: Duration,
}

pub struct ServerHandle { /* cloneable-ish */ }
impl Server {
    pub async fn start(cfg: ServerConfig, capture: Arc<dyn CaptureBackend>, request: CaptureConfig) -> Result<ServerHandle>;
}
impl ServerHandle {
    pub fn local_addr(&self) -> SocketAddr;
    pub fn format(&self) -> AudioFormat;
    pub async fn stop(self) -> Result<()>;   // cancels, closes sessions, stops capture, joins
}
```

Rules:
- Capture is started and format negotiated BEFORE TCP/UDP bind, so GET_FORMAT never races. Format is immutable while running (`Arc<AudioFormat>`), satisfying the shared-state requirement.
- No `exit()`/panic on expected errors. All public fallible APIs return `Result`.
- Stop is async and cancellation-safe; callers may drop the future only after starting it? Prefer `stop(self)` with internal cancellation token and bounded join.
- Core is platform-neutral; platform modules behind `#[cfg(windows)]` / `#[cfg(target_os = "linux")]`. `SyntheticCapture` is always available for tests and `--fake-capture` debug flag.

## Wire protocol (frozen, v0.3.4)

- TCP: `u32 LE cmd` (GetFormat=1, StartPlay=2, Heartbeat=3), then per command:
  - GetFormat response: cmd + `u32 LE size` + protobuf AudioFormat.
  - StartPlay response: cmd + `i32 LE id` (ids start at 1, increment, >0).
  - Heartbeat either direction: cmd only.
- UDP: client sends `i32 LE id` to server port; server maps id to that UDP source and sends raw PCM datagrams.
- Heartbeat: only peers that completed START_PLAY get a 3s heartbeat loop and a timeout. On each 3s tick: if `now - last_activity > 5s`, close (observably ~6s after START_PLAY); otherwise send heartbeat. Any received heartbeat refreshes activity. A connection that only did GET_FORMAT is never timed out. Client heartbeat before START_PLAY is ignored. Unknown commands and duplicate START_PLAY close without a response.
- MTU: max UDP payload 1492-20-8=1464, align down to block_align; never split a sample. block_align==0 => 1464; block_align>1464 => send whole frame in one datagram (do not panic).
- Multiple sessions: one TCP connection per client; broadcast PCM to every bound peer. On stop, close TCP sessions so clients can retry.
- Explicit LE conversions only; never `transmute`/native-endian.
- Command enum and protobuf field numbers/enum values must not change.

## Capture

### Windows (WASAPI shared-mode loopback)
- Enumerate active render endpoints (`eRender`, active), ID = `IMMDevice::GetId`, name = `PKEY_Device_FriendlyName`.
- Default = `GetDefaultAudioEndpoint(eRender, eConsole)`.
- Start: activate IAudioClient, GetMixFormat, apply requested encoding/channels/rate on a copy, `IsFormatSupported`; S_FALSE => use closest match; unsupported => error with readable message (old code returns).
- Initialize shared mode with LOOPBACK flag, 1s buffer; device period poll loop; handle silence, data discontinuity, device invalidation; stop/reset on shutdown.
- `wasapi` crate 0.27.0 preferred; `windows` 0.62.2 fallback only if spike fails.
- Validation in this environment: `cargo check/clippy --target x86_64-pc-windows-msvc` (no Windows runtime).

### Linux (PipeWire)
- Enumerate `media.class=Audio/Sink` nodes; Endpoint ID = `object.serial`; display name = `node.nick` else `node.description` else `node.name`.
- Default = highest `priority.session` strictly greater than -1 (old code starts at -1; if no sink exceeds it, there is no default). First wins ties.
- Capture via `target.object=<serial>`, `media.category=Capture`, `media.role=Music`, AUTOCONNECT, `stream.capture.sink=true` for monitor; validate the target (invalid targets silently fall back, so verify the selected node).
- Default encoding F32_LE; s8 -> U8; s16/s24/s32 -> S16_LE/S24_LE/S32_LE (S24_LE is packed 3-byte, not a 32-bit container). Negotiated rate/channels read back into AudioFormat. No resampling beyond library negotiation.
- One dedicated PipeWire thread per capture session (or a long-lived loop with a command channel); hand PCM to tokio through a lock-free ring; registry global_remove/watchdog for target loss.
- Validate against headless PipeWire null sink (environment.md).

## Server GUI (Tauri v2 + Svelte)

- Prototype A information architecture: one window; main view = status pill, listen address, Start/Stop primary, current endpoint/encoding summary; settings = secondary view; About = modal or secondary.
- Tray menu: Show, Start, Stop, separator, Exit. Dynamic enable/disable. Windows left click toggles; Linux right-click menu only.
- Close behavior: setting `WhenClose`: 0=Exit, 1=Minimize to tray. New-install default is 1 (Minimize) per spec/prototype A; an existing config that explicitly says 0 keeps Exit.
- Launch behavior: `WhenAppStart`: 0=Do nothing, 1=Start server, 2=Keep last state (start only if `[App] Running=true` was persisted on the previous successful start).
- Single instance: second launch shows/focuses the existing window; `--hide` starts hidden.
- Autostart: config `AutoRun`. Windows writes HKCU Run value `AudioShareServer` = `"<exe>" /hide`; Linux writes `~/.config/autostart/AudioShareServer.desktop` with `Exec="<exe>" --hide` (use `$APPIMAGE` when set). Implement directly for correct path quoting; accept `/hide`, `-hide`, and `--hide`.
- i18n: en-US + zh-CN JSON resources; `[App] language` = `default` | `en-US` | `zh-CN`; applied at startup only. `default` follows system locale.
- Update: GET `https://api.github.com/repos/mkckr0/audio-share/releases/latest`, compare tag to `0.4.0` (strip v, semver), notify + open `html_url`; every 3h while `AutoUpdate`; manual check reports No Update/error.
- Windows-only: Repair Firewall (elevated netsh rules for TCP+UDP inbound for this exe; rule names `Audio Share Server(TCP-In)` / `Audio Share Server(UDP-In)`; readable error on UAC cancel/failure), Sound Panel (`control.exe /name Microsoft.Sound`). Linux shows documentation links instead.
- WebView2: Windows bootstrap before Tauri init checks HKLM/HKCU EdgeUpdate client key; if missing show MessageBox with download URL, then exit 0. Do not crash.
- Start/Stop commands are async; UI never blocks; errors shown as readable messages/toasts.

## Config (`config.ini` next to executable)

Resolution order (Linux packages/AppImage cannot write next to the executable):
1. `AUDIO_SHARE_CONFIG` env var (absolute path), if set.
2. `config.ini` next to the executable if that directory is writable (Windows portable, CLI tarball; preserves v0.3.4 behavior).
3. Otherwise `$XDG_CONFIG_HOME/audio-share-server/config.ini` (default `~/.config/audio-share-server/config.ini`).
`server.log` follows the same directory. If a legacy exe-dir config exists but is read-only, migrate its values into the writable location and report it.

Legacy sections/keys (confirmed in MFC source; keep exact casing):
- `[App] configVersion`, `language`, `Running` (last running state, bool)
- `[Network] host`, `port` (strings)
- `[Capture] endpoint` (string, default "default"), `encoding` (integer 0..6: 0=default, 1=invalid, 2=f32, 3=s8, 4=s16, 5=s24, 6=s32)
- `[App Settings] AutoRun`, `WhenAppStart` (0 none, 1 start, 2 keep last state), `WhenClose` (0 exit, 1 minimize), `AutoUpdate`
- MFC writes host/port/endpoint/encoding and `[App] Running=true` on successful start, `Running=false` on stop. Reset reloads config values.
- New config keeps these keys and integer encoding for downgrade compatibility; also writes a new `configVersion=2`.

Behavior:
- New configVersion = 2. Missing/1 => migrate all recognized keys and write v2; report `Migrated { from: 1 }`.
- 2 => load. >2 => load recognized keys without overwriting, report `FutureVersion` (backup file kept).
- Parse failure => back up `config.ini.bak`, use defaults, report `RecoveredFromBackup`.
- Writes are atomic: temp file in same directory + fsync + replace (Unix rename; Windows `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`).
- Defaults: host=default address, port=65530, endpoint="default", encoding="default" (0), autostart=false, launch=0, close=1 (minimize; existing explicit 0 stays exit), autoupdate=false, language="default".

## as-cmd

- Flags exactly: `-h/--help`, `-l/--list-endpoint`, `-b/--bind [[host][:port]]` (no value => default address), `-e/--endpoint`, `--encoding`, `--list-encoding`, `--channels`, `--sample-rate`, `-V/--verbose`, `-v/--version`.
- Default port 65530. Default bind host = first private IPv4 (10/8, 172.16/12, 192.168/16) else first non-loopback IPv4.
- `-l` lists endpoints with `*` for default and `total: N`; `--list-encoding` lists default/f32/s8/s16/s24/s32.
- Help includes examples; no args prints help to stderr and exits 1; parse/start errors exit 1 with readable stderr; help/version exit 0.
- Logging: `server.log` next to executable (and stdout for CLI); `-V` sets trace.
- `-b` runs until SIGINT/Ctrl-C, then stops and exits 0. (Add graceful signal handling.)

## Packaging / CI

- Windows: `tauri build --no-bundle`, artifact renamed `AudioShareServer.exe`; WebView2 runtime required but detected.
- Linux: `tauri build --bundles deb,appimage`; deb declares `libwebkit2gtk-4.1-0, libgtk-3-0, libayatana-appindicator3-1, libpipewire-0.3-0`; AppImage documented.
- CI: windows-latest + ubuntu-24.04; fmt, clippy, tests, frontend build, artifact upload. No vcpkg/MSVC/MFC.
- Version 0.4.0; artifact names `AudioShareServer.exe`, `audio-share-server-cmd-windows.zip`, `audio-share-server-cmd-linux.tar.gz`, plus deb/AppImage.

## Test seams

- Only automated integration seam: Wire Protocol scripted client over real localhost sockets with synthetic capture. Unit tests for framing/MTU/heartbeat/config/version.
- No GUI automation; manual checklist in docs.
- Long-running/real-device checks: scripts + documented results; do not fake execution.
