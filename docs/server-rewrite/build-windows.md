# Windows build and packaging (ticket 13)

Status: **packaging code, CI, and documentation implemented; the Windows
artifacts are built, packaged, and executed on native Windows.** The
`windows-packaging` job on `windows-latest` produced a PE32+ x86-64
`AudioShareServer.exe` and the `as-cmd` zip, both checksum sidecars verify, and
the artifacts were executed on Windows 11 on 2026-11-10 (see
[Executed Windows 11 acceptance](#executed-windows-11-acceptance-2026-11-10)).
The two runtime defects found during that execution (HTTPS update-check TLS
backend and the WASAPI `Initialize` error mapping) are fixed and covered by
tests. The follow-up CI run
[42766244106](https://github.com/W0n9/audio-share/actions/runs/42766244106)
(`ebdd468`) is green on all four jobs, and its artifacts include those fixes and
were smoke-tested on Windows 11. A third defect — the UDP broadcast loop
blocking on a slow Wi-Fi peer — was found during the real-phone 60-minute run
and is fixed. The final CI run
[42777988420](https://github.com/W0n9/audio-share/actions/runs/42777988420)
(`9626b10`) is green on all four jobs and its artifacts include all three
fixes. Windows 10 and the missing-WebView2 clean-VM path remain manual gates.

The Windows release is a **portable `AudioShareServer.exe`** with no installer.
The GUI targets Windows 10+ and requires the Microsoft Edge WebView2 Runtime,
which is detected before Tauri starts.

## Artifacts

| Artifact | Source | Contents |
|---|---|---|
| `AudioShareServer.exe` | `gui/` → `npx tauri build --no-bundle` | Portable Server GUI (Tauri), icon embedded |
| `AudioShareServer.exe.sha256` | packaging script | `sha256sum`-format checksum |
| `audio-share-server-cmd-windows.zip` | `cargo build --release -p as-cmd` + packaging script | Top-level `audio-share-server-cmd/` containing `as-cmd.exe`, `LICENSE`, `README.md` |
| `audio-share-server-cmd-windows.zip.sha256` | packaging script | `sha256sum`-format checksum |

The archive keeps the v0.3.4 name and internal layout (`as-cmd.exe` inside
`audio-share-server-cmd/`). Artifact names do not contain the version; version
`0.4.0` is embedded in the binaries. Release wiring is ticket 16.

`tauri.conf.json` keeps:

- `productName: AudioShareServer`
- `mainBinaryName: AudioShareServer`
- `version: 0.4.0`
- `bundle.targets: []` (no MSI/NSIS installer)
- `bundle.icon` including `icons/icon.ico` (embedded in the exe)

`bundle.windows.webviewInstallMode` remains in the config for reference, but it
only affects MSI/NSIS bundles. It does nothing for the portable exe.

## Prerequisites

- Windows 10 or 11, x64.
- Rust stable 1.90 or newer (workspace MSRV) with the default Rust Windows target
  (`rustup default stable-x86_64-pc-windows-msvc`).
- Visual Studio 2022 Build Tools with the **Desktop development with C++**
  workload (the linker and Windows SDK used by the default Rust Windows
  target). The GNU target is not supported: `webview2-com-sys` statically
  links `WebView2LoaderStatic.lib` only for the default target; GNU builds
  would require `WebView2Loader.dll` next to the exe.
- Node.js 22 and npm.
- Microsoft Edge WebView2 Runtime (Evergreen). Windows 11 includes it; most
  Windows 10 machines have it, but it is not guaranteed.

## Build steps

Run from the repository root unless noted. The Cargo workspace puts build
outputs under `<repo>\target\release`, **not** `gui\src-tauri\target\release`.

```bash
cd gui
npm ci
npm run build
npx tauri build --no-bundle
cd ..
cargo build --release -p as-cmd

# Package the flat artifact directory (defaults to <repo>/dist-windows).
bash scripts/package-windows-artifacts.test.sh
bash scripts/package-windows-artifacts.sh
```

Expected outputs:

```text
<repo>/target/release/AudioShareServer.exe
<repo>/target/release/as-cmd.exe
<repo>/dist-windows/AudioShareServer.exe
<repo>/dist-windows/AudioShareServer.exe.sha256
<repo>/dist-windows/audio-share-server-cmd-windows.zip
<repo>/dist-windows/audio-share-server-cmd-windows.zip.sha256
```

Packaging script options (paths are resolved against `--root` when relative):

```text
scripts/package-windows-artifacts.sh [--root DIR] [--target-dir DIR] [--out-dir DIR]
```

Do not use a bare `cargo build --release` for the GUI exe: without
`tauri/custom-protocol` it points at the dev server (`http://localhost:1420`)
instead of the embedded frontend. `npx tauri build` enables the feature itself.
With `bundle.targets: []`, plain `npx tauri build` also produces only the exe,
but `--no-bundle` is explicit.

Verify a downloaded artifact in PowerShell:

```powershell
(Get-FileHash .\AudioShareServer.exe -Algorithm SHA256).Hash.ToLower()
Get-Content .\AudioShareServer.exe.sha256
```

or in Git Bash: `sha256sum -c AudioShareServer.exe.sha256`.

## WebView2 detection and preflight

The preflight runs in `main()` before `tauri::Builder` is created
(`gui/src-tauri/src/webview2_check.rs`). Tauri's own late check can end in an
`expect()` panic; this path exits cleanly instead.

Detection order:

1. `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER` (fixed-version runtime): installed when
   `msedgewebview2.exe` exists in that folder. No registry entry is written for
   a fixed-version runtime.
2. `HKLM\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}`
   value `pv` (machine-wide, 32-bit registry view used by x64 apps).
3. `HKCU\Software\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}`
   value `pv` (per-user install).

`pv` that is missing, empty/blank, or `0.0.0.0` means **not installed**. Any
other value is accepted. The pure decision function (`decide`) is unit-tested
with injected registry and environment values on every host.

When the runtime is missing, the app shows a native `MessageBoxW` titled
“Audio Share Server” explaining that the WebView2 Runtime is required:

- **Yes** opens
  <https://developer.microsoft.com/en-us/microsoft-edge/webview2/> in the
  default browser.
- **Yes or No** then exits the process with status **0** (no Tauri error, no
  panic). The user installs the Evergreen Runtime and starts the app again.

Windows 11 is expected to pass without a prompt. Windows 10 must be tested both
with the runtime present and absent (see checklist).

## CI

`.github/workflows/server-rewrite.yml` job **Windows portable packaging**
(`windows-packaging`, `windows-latest`) runs on pushes to
`integration/server-rewrite` and `ticket/**`, on pull requests touching
`gui/**`, `crates/**`, `scripts/**`, the workflow, and on manual dispatch:

1. Rust stable (Windows target), Node 22, npm cache, Rust target cache.
2. `npm ci`, `npm run build`, `npx tauri build --no-bundle`.
3. `cargo build --release -p as-cmd`.
4. Asserts `<workspace>/target/release/AudioShareServer.exe` and
   `as-cmd.exe` exist (workspace target path guard).
5. Runs `scripts/package-windows-artifacts.test.sh`.
6. Runs `scripts/package-windows-artifacts.sh`.
7. Uploads GitHub Actions artifacts:
   - `AudioShareServer-windows`: `AudioShareServer.exe` + `.sha256`
   - `audio-share-server-cmd-windows`: the zip + `.sha256`

The parallel `Rust (windows-latest)` job keeps running fmt, clippy, and the
workspace tests (including the WebView2 decision tests) on a native Windows
host.

### Executed CI build (2026-11-09)

Run [42638565632](https://github.com/W0n9/audio-share/actions/runs/42638565632)
on commit `6f1f429` completed green for all four jobs: `Rust (ubuntu-24.04)`,
`Rust (windows-latest)`, `Linux deb/AppImage packaging`, and `Windows portable
packaging`. The two Windows artifacts were downloaded on the Linux host and
verified:

| Artifact | Contents | Verification |
|---|---|---|
| `AudioShareServer-windows` | `AudioShareServer.exe` (11,918,848 B) + `.sha256` | PE32+ GUI x86-64; SHA-256 `27a57a65…75e2b` matches |
| `audio-share-server-cmd-windows` | `audio-share-server-cmd-windows.zip` (686,080 B) + `.sha256` | Contains `audio-share-server-cmd/{as-cmd.exe,LICENSE,README.md}`; SHA-256 `eb745b78…ea60` matches |

This proves the native Windows build, packaging, and artifact upload paths.
It does **not** execute the GUI, WASAPI capture, or `as-cmd.exe`; those remain
in the manual checklist below.

The first push of this branch exposed pre-existing CI defects, fixed in
`95521b7`, `e3c584e`, and `6f1f429`: the Ubuntu `rust` job lacked the
Tauri/WebKitGTK build dependencies needed to compile the GUI crate; the
synthetic-capture stop test assumed the frame channel was empty immediately
after `stop()`; and the listener-release test assumed an immediate connect
refusal on an ephemeral port.

## Executed Windows 11 acceptance (2026-11-10)

Host: Windows 11 Enterprise 10.0.26300, x64; WebView2 Runtime 157.0.4341.42;
Visual Studio 2022 Build Tools 17.x (MSVC 14.44.35207); Rust 1.99.1. The
initial acceptance used the CI artifacts from run
[42710876060](https://github.com/W0n9/audio-share/actions/runs/42710876060)
(`bad71d3`) and a locally built exe containing the first two fixes found during
this run (HTTPS update-check TLS backend and the WASAPI `Initialize` error
mapping). Later runs and their artifacts are listed below.

| Artifact | Size | SHA-256 |
|---|---:|---|
| CI `AudioShareServer.exe` (run 42710876060, pre-fix baseline) | 11,918,848 B | `21c5b26478a0a4ad12241e60eec724b4613fbc07ad2f85c0fb479d1d2851faa6` |
| CI `audio-share-server-cmd-windows.zip` (run 42710876060) | 686,081 B | `6f8f864779d0290ab720af5c3f1855075765b7652bdd0fe04533ad665af4bd42` |
| CI `AudioShareServer.exe` (run 42766244106, fixes included) | 11,973,120 B | `93a456642ad52db1c934b0f818b441cfe25a232650d1065ebff7e2e8abab2e0c` |
| CI `audio-share-server-cmd-windows.zip` (run 42766244106) | 686,340 B | `b200f5fd8d8d4f84387028e586b717db72f6097249fdc8931a1cff1282c01839` |
| CI `AudioShareServer.exe` (run 42777988420, all three fixes) | 11,966,464 B | `a3ee857ac315b7e6ec3b79ad1248317f75b1cbe6b20a6ed220100d0ec77b2cbf` |
| CI `audio-share-server-cmd-windows.zip` (run 42777988420) | 688,182 B | `216c95808d79a55c3309cf4b58004db4534165398083a19beb9378fa3a7993ef` |
| locally built fixed `AudioShareServer.exe` | 11,970,048 B | `f2d14db7cd66e52f821d04307e6bd2085530b70a332f50d1019e714576683925` |

All downloaded CI `.sha256` sidecars matched with `Get-FileHash`; the zip
contains `audio-share-server-cmd/{as-cmd.exe,LICENSE,README.md}`.

| Check | Result |
|---|---|
| Portable exe, no installer/MSI; runs directly and from a path containing spaces | PASS |
| Launch opens the 420×640 window (630×960 physical at 150% scaling), no console flash; process image `AudioShareServer.exe` | PASS |
| Tray icon present in the overflow; left-click toggles the window; right-click menu shows Show / Start / Stop / Exit | PASS |
| Tray menu actions Show / Start / Stop / Exit | PASS (triggered through the native menu command IDs; each action observed) |
| Close with `WhenClose=1` keeps the process running and hides the window; tray Show restores it | PASS |
| Close with `WhenClose=0` exits the process (~1 s) | PASS |
| `--hide` starts with no visible window and a tray icon; tray Show restores it | PASS |
| Single instance: second launch exits 0 and focuses/re-shows the existing window; a second `--hide` launch exits 0 without showing the hidden window | PASS |
| WhenAppStart: `0` stays stopped; `1` starts the server; `2` follows the previous `Running` value | PASS (all three) |
| Start/Stop uses real WASAPI; listen address updates; Stop closes the client session | PASS — probe 10 s: 1,998 datagrams, alignment and MTU OK, 0 heartbeat misses, 440 Hz tone; Stop closed the TCP session (`serverClosed: true` after 3.1 s) |
| Settings list real endpoints; Host/Port/Endpoint/Encoding persist; settings lock while running | PASS |
| Existing v0.3.4 `config.ini` migrates (language, host, port, endpoint, encoding, app settings) with a visible notice and `configVersion=2` | PASS — migrated 0 → 2, values preserved |
| Reset restores server defaults and writes `config.ini` | PASS |
| Autostart on writes HKCU `Run\AudioShareServer` = `"<exe>" /hide`; off removes it; spaces in the path are quoted | PASS |
| Manual update check over HTTPS | PASS — “up to date” (fixed; see below); the real HTTPS ignored test passes |
| Repair Firewall (UAC approve) creates TCP/UDP inbound rules scoped to the exe, Domain+Private, allow | PASS — verified with `netsh advfirewall firewall show rule ... verbose` |
| Repair Firewall (UAC cancel) shows a readable error and leaves the GUI responsive | PASS — “Firewall repair was cancelled because administrator permission was not granted.” |
| A client can connect through the repaired rules | PASS on the server side — local client over the WLAN IP `10.8.144.155:65530` received 120 aligned datagrams; the real Android Client was later executed on one phone (ticket 15) |
| Sound Panel opens `control.exe /name Microsoft.Sound`; GUI stays responsive | PASS — `#32770` dialog titled “声音” observed and closed |
| en-US and zh-CN render; language persists and applies after restart | PASS — system default zh-CN, switched to en-US, restarted, switched back |
| About shows 0.4.0, homepage, Apache-2.0; homepage opens the default browser | PASS |
| `server.log` is created/truncated next to the executable and records startup/start/stop/update errors | PASS |
| `as-cmd.exe` from the zip: `--version` 0.4.0, `-l`, `--list-encoding` | PASS |
| Real WASAPI A/B vs v0.3.4 `as-cmd.exe` (default/f32/s8/s16/s24/s32) | PASS — see [capture-windows.md](capture-windows.md) |

Remaining open items:

- Windows 10 (only Windows 11 Enterprise 10.0.26300 was available).
- Missing-WebView2 clean-VM test (the runtime is present; the decision logic is
  unit-tested), per-user WebView2 install, and fixed-version
  `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER`.
- Reboot/login autostart (the HKCU entry was verified, not a real sign-in).
- Two-Android-client run and the mono combinations (mono is N/A on this Realtek
  endpoint). The single-phone matrix, reconnect, heartbeat, and a passing
  60-minute playback were executed on 2026-11-10 through the Windows Mobile
  Hotspot; see [Android acceptance](android-v034-acceptance.md).
- Sleep/resume and real-hardware hotplug; exclusive-mode endpoint; endpoint
  switching (only one active render endpoint).

### Defects found and fixed during this run

1. **Update check failed on Windows**: “Unknown Scheme: cannot make HTTPS
   request because no TLS backend is configured”. ureq 2's `native-tls`
   feature only provides an adapter and is not a default backend; the Windows
   agent now passes an explicit `native_tls::TlsConnector` (schannel). Unit
   test + the real `api.github.com` ignored test pass.
2. **Mono capture error was a raw HRESULT**: `IsFormatSupported` returns
   `S_OK` for mono at the mix rate, but `Initialize` rejects it with
   `AUDCLNT_E_UNSUPPORTED_FORMAT` (0x88890008). The production backend and the
   spike now map it to a readable “endpoint rejected the negotiated loopback
   format … try another encoding, channel count, or sample rate” error.
3. **UDP broadcast loop could stall on one slow Wi-Fi peer**: the loop awaited
   `udp.send_to`. When the phone stopped draining (power save/link stall), the
   socket send buffer filled; the broadcast loop blocked, the bounded frame
   channel filled, and the capture thread blocked in `blocking_send`. The
   control plane (TCP heartbeats) stayed alive, so the phone showed “playing”
   with no audio. The loop now uses `try_send_to` and drops on `WouldBlock`
   (one warning per session), so one slow peer cannot stall capture or other
   clients. Post-fix, a 60 s phone Wi-Fi-off plus a non-reading UDP peer still
   let a PC probe receive 1,198 datagrams/440 Hz per 6 s; the fixed 60-minute
   phone soak is the regression test.

All three fixes are covered by tests, and the final `Rust (windows-latest)`
job in run 42777988420 is green. The final artifacts were smoke-tested on
Windows 11: `as-cmd --version` 0.4.0, `-l`, `--list-encoding`, and the
readable mono `AUDCLNT_E_UNSUPPORTED_FORMAT` error. The final `as-cmd.exe`
(SHA-256 `8d984573600def6ead0c0effcfcb3e5750adb6472091a36de661e42393ea831c`)
was also played by the real v0.3.4 Android Client for ~11 minutes (six probes
at two-minute intervals: 1,997–2,000 datagrams per 6 s, 440 Hz, max gap 21 ms,
0 heartbeat misses, phone AudioTrack active throughout, 0 app crashes). The
GUI HTTPS update check was smoke-tested earlier (run 42766244106). The older
run 42710876060 artifacts are retained as the pre-fix baseline.

## Manual acceptance checklist (Windows 10 and Windows 11)

Run each item on **both** Windows 10 x64 and Windows 11 x64. Record OS build,
artifact SHA-256, and pass/fail. Do not mark the ticket resolved without these
results.

### Artifact and launch

- [x] CI `windows-packaging` job is green and its artifacts download
      (verified from the Linux host, run 42638565632; see above).
- [ ] `AudioShareServer.exe.sha256` matches the downloaded exe
      (`sha256sum -c` or `Get-FileHash`).
- [ ] No installer/MSI/NSIS artifact exists; the exe runs directly from a
      normal folder (and from a path containing spaces).
- [ ] Launching the exe opens the 420×640 window (no console flash); the tray
      icon appears.
- [ ] `AudioShareServer.exe` is the process image name (Task Manager → Details).

### WebView2

- [ ] **Runtime present:** app starts normally on a machine with the Evergreen
      Runtime (Windows 11 default).
- [ ] **Runtime missing:** on a clean Windows 10 VM without WebView2, launch
      shows the native Audio Share Server dialog with the download link.
- [ ] Clicking **Yes** opens the Microsoft WebView2 page in the default
      browser; the app then exits with code 0.
- [ ] Clicking **No** exits with code 0. No Tauri panic dialog, no
      “WebView2 runtime not installed” crash text.
- [ ] After installing the Evergreen Runtime, launching the same exe succeeds.
- [ ] (If available) a per-user install (HKCU `pv`) and a fixed-version folder
      set via `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER` are both detected.

### Server GUI behavior

- [ ] Start shows Running and a listen address; Stop returns to Stopped.
- [ ] Settings lists real Windows audio endpoints; encoding/host/port selectable.
- [ ] `config.ini` is created next to the exe; an existing v0.3.4 config is
      migrated (language, host, port, endpoint, encoding, app settings).
- [ ] Tray menu Show / Start / Stop / Exit works; left-click toggles the window;
      close button follows `WhenClose` (minimize to tray or exit).
- [ ] Single instance: a second launch focuses the existing window.
- [ ] Autostart writes/removes the HKCU Run entry (`"<exe>" /hide`) and survives
      a reboot.
- [ ] Repair Firewall prompts for UAC and creates both inbound rules; a phone
      can connect with the firewall enabled. Cancelling UAC shows a readable error.
- [ ] Sound Panel opens `control.exe /name Microsoft.Sound`.
- [ ] Update check reports No Update or finds a newer release; disabling
      auto-update stops the periodic check; About shows 0.4.0, homepage, license.
- [ ] en-US and zh-CN render after restart; system-locale default works.
- [ ] A v0.3.4 Android Client connects and plays audio (protocol acceptance).

### as-cmd archive

- [ ] Extract `audio-share-server-cmd-windows.zip`; it contains
      `audio-share-server-cmd/as-cmd.exe`, `LICENSE`, `README.md`.
- [ ] Zip SHA-256 matches the `.sha256` file.
- [ ] `as-cmd.exe --version` prints the expected name/version/homepage.
- [ ] `as-cmd.exe --list-encoding` and `as-cmd.exe -l` work; `-b` serves audio
      to the Android Client.

## Known limitations and residual verification

- Not executed on Windows from this Linux host. The native Windows CI runner
  builds and packages the artifacts (see above); `cargo
  check/clippy --target x86_64-pc-windows-msvc` also type-checks the code
  locally (the two `GNU compiler is not supported for this target` warnings
  come from `tauri-build` resource embedding and disappear in a native
  Windows build).
- No code signing: SmartScreen may warn on first run; this matches v0.3.4 and
  the 0.4.0 scope.
- No installer and no Tauri updater: users replace the portable exe manually.
- Toast notifications are not fully supported for uninstalled portable apps;
  the in-app update banner is the primary UX.
- Real WebView2 detection, the missing-runtime dialog, and Windows 10/11 GUI
  acceptance remain **manual Windows verification** for ticket 13.
