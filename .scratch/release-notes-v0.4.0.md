0.4.0 is a big-bang rewrite of the Server in Rust. The C++20 Server Core and the Windows-only MFC GUI are gone: a shared `audio-share-core` crate now powers both the tray-first Tauri v2 GUI and the headless `as-cmd` on Windows and Linux. The Wire Protocol is frozen at v0.3.4, so the released Android Client keeps working unchanged — **no new APK ships with this release; keep using the v0.3.4 release or F-Droid.**

**If you run the v0.3.4 Server, we recommend upgrading.** 0.4.0 brings the Server GUI to Linux, fixes Linux `s8` capture, and removes several startup, TLS, and slow-peer networking defects. It was validated with the real v0.3.4 Android Client on an Android 12 phone, an 8-hour two-client soak, and a Windows 11 runtime checklist.

## Highlights

- **The Server is Rust all the way down.** `as-cmd` and the GUI link the same `audio-share-core`, so capture, framing, MTU segmentation, heartbeat, and lifecycle cannot diverge between the two front ends. The suite runs 281 tests on Linux (4 PipeWire-gated tests ignored) and 273 on Windows, with `cargo fmt --check` and `clippy -D warnings` clean. (#1)
- **One tray-first Server GUI for Windows and Linux.** Start/Stop, tray menu, autostart, launch and close behavior, Settings, About, update checks, and en-US/zh-CN localization. Closing the window now hides it by default on a fresh install.
- **A portable Windows build.** `AudioShareServer.exe` needs no installer and requires the Microsoft Edge WebView2 Runtime, which is detected before Tauri starts. The headless `as-cmd.exe` ships as a zip. Windows Firewall Repair and the system Sound Panel remain in the GUI.
- **First-class Linux packages.** `AudioShareServer` ships as a deb and an AppImage for Ubuntu 24.04+, plus a standalone `as-cmd` tarball. Capture uses PipeWire, packages declare their WebKitGTK/GTK/appindicator/PipeWire dependencies, and config/log files fall back to the XDG directory when `/usr/bin` is not writable.
- **The v0.3.4 protocol is pinned, replayed, and exercised.** Golden fixtures and the `protocol-harness` replay the frozen wire format offline; the acceptance matrix passed 20/20 encoding × channel × rate combinations and 6/6 scripted scenarios. The released v0.3.4 APK played through the real Android Client for 60 minutes on an Android 12 phone with 0 heartbeat misses and no app crash.
- **Long runs stay flat.** The 8-hour release soak carried 2 clients and 22.08 GB of PCM (16.17 million datagrams): 0 heartbeat misses, no datagram gap above 100 ms, and server RSS plateaued at ~15.6 MB with no growth trend.
- **Fixes found during acceptance.** A slow Wi-Fi peer no longer stalls the UDP broadcast loop, Windows polling requests 1 ms timer resolution, update checks use an explicit schannel TLS connector, WASAPI `Initialize` failures produce readable errors, and Linux `s8` silence is written as `0x80`.

## :warning: Breaking changes

- **This is a Server-only release.** The Android Client stays at v0.3.4; no APK is rebuilt or attached. The Wire Protocol and `protos/client.proto` are unchanged.
- **Linux config and log locations can move.** When the executable directory is not writable (deb installs under `/usr/bin`, read-only AppImages), `config.ini` and `server.log` now live in `$XDG_CONFIG_HOME/audio-share-server/` (default `~/.config/audio-share-server/`). A v0.3.4 `config.ini` found next to the executable is migrated; `AUDIO_SHARE_CONFIG` overrides the location.
- **`as-cmd` now writes `server.log`.** The file sits in the resolved config directory, is truncated on each server launch, and receives the same lines as stdout. Failure to open it is a warning, not a startup failure.
- **New installs close to the tray.** `WhenClose` keeps its v0.3.4 encoding (`0` = exit, non-zero = minimize), so existing configs behave as before. A brand-new config with no `WhenClose` key now defaults to minimize-to-tray.
- **Invalid `--encoding` values fail during parsing.** Only `default`, `f32`, `s8`, `s16`, `s24`, and `s32` are accepted; anything else exits 1 with a readable message instead of starting a server that cannot capture.
- **`-b host:port` (space form) now binds the address you pass.** v0.3.4 silently ignored the space-separated value and used the default address; both `-b host:port` and `--bind=host:port` now work. A bare `-b` is unchanged.
- **Capture must succeed before the listener binds.** `GET_FORMAT` can no longer return an empty or torn `AudioFormat`: the negotiated format is known and immutable before the first client connects. An endpoint that cannot start now fails startup (exit 1) instead of serving a server with no audio.
- **`server.log` timestamps are UTC.** Message text, levels, and file placement are unchanged; lines shift by the local UTC offset compared with v0.3.4.

## Upgrading from v0.3.4

- **Windows:** replace the portable `AudioShareServer.exe`; `config.ini` next to it is read as before. The `as-cmd` zip keeps the v0.3.4 archive name and internal layout.
- **Linux:** install the new deb or AppImage. Existing settings are migrated to `~/.config/audio-share-server/config.ini` when the install directory is read-only. A running PipeWire session with a session manager (WirePlumber) is required.
- **Android:** no action. Keep the v0.3.4 APK; it is the acceptance client for this release.

## Known limitations

- **No automatic capture recovery.** If the selected endpoint disappears (unplug, sink change) or the machine sleeps and resumes, capture stops while the process keeps running with its last negotiated format. Recovery is an operator Stop then Start in the GUI, or a restart of `as-cmd`.
- **Some acceptance gates remain manual.** Mono transfer on this Realtek endpoint (the driver rejects it at `Initialize`, as v0.3.4 did), a two-Android-client run, sleep/resume, real-hardware hotplug, Windows 10, a missing-WebView2 clean VM, and the real Linux desktop tray were not exercised.

## What's Changed

* core: add the Rust workspace, frozen protocol, and protocol harness ([fdf1c20](https://github.com/W0n9/audio-share/commit/fdf1c20))
* core: serve the v0.3.4 Wire Protocol with synthetic capture and `as-cmd -b` ([46ede62](https://github.com/W0n9/audio-share/commit/46ede62))
* core: resolve config/log paths with an XDG fallback and `AUDIO_SHARE_CONFIG` ([bf5e072](https://github.com/W0n9/audio-share/commit/bf5e072))
* core: migrate v0.3.4 `config.ini` without losing settings ([bdd113d](https://github.com/W0n9/audio-share/commit/bdd113d))
* capture: add the Windows WASAPI loopback backend ([2f33f91](https://github.com/W0n9/audio-share/commit/2f33f91))
* capture: add the Linux PipeWire backend ([eb1e0ae](https://github.com/W0n9/audio-share/commit/eb1e0ae))
* capture: write unsigned 8-bit silence as `0x80` ([15ad3a1](https://github.com/W0n9/audio-share/commit/15ad3a1))
* capture: request 1 ms timer resolution for Windows polling ([87290af](https://github.com/W0n9/audio-share/commit/87290af))
* as-cmd: restore full v0.3.4 CLI parity ([254c1e9](https://github.com/W0n9/audio-share/commit/254c1e9))
* gui: add the Tauri tray-first Server shell with Start/Stop ([4566b85](https://github.com/W0n9/audio-share/commit/4566b85))
* gui: add lifecycle, i18n, About, and update checks ([410a319](https://github.com/W0n9/audio-share/commit/410a319))
* gui: add Windows Firewall Repair and the system Sound Panel ([2c36fd6](https://github.com/W0n9/audio-share/commit/2c36fd6))
* gui: add Linux documentation links for Windows-only maintenance ([6b0a776](https://github.com/W0n9/audio-share/commit/6b0a776))
* gui: add the WebView2 preflight before Tauri startup ([26ba34f](https://github.com/W0n9/audio-share/commit/26ba34f))
* harness: add the acceptance probe for format, framing, heartbeat, and tone ([c76a381](https://github.com/W0n9/audio-share/commit/c76a381))
* scripts: add acceptance matrix, soak, and hotplug checks ([ca70b7d](https://github.com/W0n9/audio-share/commit/ca70b7d))
* ci: package the portable Windows exe and `as-cmd` archive ([59f9c65](https://github.com/W0n9/audio-share/commit/59f9c65))
* linux: package deb/AppImage with CI and build docs ([3d5bb8b](https://github.com/W0n9/audio-share/commit/3d5bb8b))
* release: assemble the 0.4.0 Server artifacts and draft release ([df02371](https://github.com/W0n9/audio-share/commit/df02371))
* fix(windows): use an explicit TLS backend for update checks and map WASAPI errors ([f2e3c0f](https://github.com/W0n9/audio-share/commit/f2e3c0f))
* fix(server): drop UDP datagrams instead of stalling on a slow peer ([f6f2e28](https://github.com/W0n9/audio-share/commit/f6f2e28))
* chore: remove the legacy C++/MFC Server ([6cd8511](https://github.com/W0n9/audio-share/commit/6cd8511))

**Full Changelog**: https://github.com/W0n9/audio-share/compare/v0.3.4...v0.4.0

### Contributors

[W0n9](https://github.com/W0n9) — rewrite, packaging, and acceptance.
