# Server rewrite documentation

Index for the Rust Server that ships as Audio Share 0.4.0. The earlier
implementation (C++ Server Core + Windows-only GUI) was removed in the
big-bang cutover; this directory and the linked references are the current
maintenance documentation.

## Architecture

The repository is one Cargo workspace:

| Path | Purpose |
|---|---|
| `crates/audio-share-core` | Capture abstraction, PipeWire and WASAPI backends, frozen v0.3.4 Wire Protocol, MTU segmentation, settings/config migration, server lifecycle. Public API returns `Result`; library code never calls `exit()`. |
| `crates/as-cmd` | Headless command-line entry point. Thin wrapper over the core; preserves the v0.3.4 flags and adds `server.log`, graceful Ctrl-C, and validated arguments. |
| `gui/` | Tauri v2 + Svelte 5 Server GUI. The Rust controller lives in `gui/src-tauri/src/lib.rs`; Tauri commands/tray are thin adapters. See [`gui/README.md`](../../gui/README.md). |
| `tools/protocol-harness` | Scripted Wire Protocol client, fixture recorder/replayer, and fake v0.3.4 server. |
| `tools/pipewire-spike`, `tools/wasapi-spike` | Audio backend spikes retained as verification tools. |

`as-cmd` and the GUI link the same `audio-share-core`, so capture and protocol
behavior cannot diverge. The only automated integration seam is the Wire
Protocol; unit tests cover framing, MTU, heartbeat, config migration, and
version comparison.

## Protocol and fixtures

- [Wire Protocol (frozen v0.3.4)](../protocol.md) — framing, endianness,
  commands, protobuf, heartbeat, UDP bind, MTU, stop semantics, and the
  startup-format guarantee.
- [Golden fixtures and replay](../../fixtures/protocol/v0.3.4/README.md) —
  recorded Linux and synthesized Windows fixture sets.
- [Protocol harness](harness.md) — `record`, `replay`, `serve-fake`,
  `synthesize`, and fixture schema.

## Components

- [`as-cmd`](as-cmd.md) — flags, dispatch precedence, exit codes, logging,
  config/log resolution, and intentional differences from v0.3.4.
- [Linux support](linux.md) — runtime dependencies, XDG autostart, tray
  behavior, and headless verification.
- [Windows maintenance](windows-maintenance.md) — Firewall Repair and Sound
  Panel behavior, including manual verification steps.

## Capture backends

- [Linux PipeWire capture](capture-linux.md) — `Audio/Sink` enumeration,
  `priority.session` default selection, U8/S16/S24/S32 mapping, realtime ring,
  target-loss behavior, and stop.
- [Windows WASAPI capture](capture-windows.md) — loopback format negotiation,
  closest-match fallback, silent packets, and stop.

## Build and packaging

- [Linux build and packaging](build-linux.md) — deb/AppImage, runtime
  dependencies, clean-container dependency check, known limitations.
- [Windows build and packaging](build-windows.md) — portable
  `AudioShareServer.exe`, WebView2 detection, prerequisites, and artifact
  layout.
- Release procedure: [README](../../README.md) and the workflows in
  `.github/workflows/server-rewrite.yml` and `.github/workflows/release.yml`.

## Verification

- [Linux A/B: v0.3.4 vs 0.4.0](ab-linux.md) — same-machine format, datagram,
  and tone comparison, including the old `s8` defect.
- [Acceptance matrix](acceptance-matrix.md) — `scripts/acceptance-matrix.sh`:
  encoding × channels × sample-rate matrix, multi-client, reconnect,
  heartbeat timeout, and SIGINT scenarios.
- [Soak](soak.md) — `scripts/soak.sh`: duration, release build, log rotation,
  crash detection, and the recorded short release run.
- [Hotplug and sleep](hotplug-sleep.md) — `scripts/hotplug-recovery.sh`:
  headless sink destruction/recreation and operator Stop/Start recovery.
- [Android v0.3.4 acceptance](android-v034-acceptance.md) — real-APK
  acceptance procedure and evidence package.
- [GUI acceptance checklist](gui-acceptance-checklist.md) — manual desktop
  tray/menu/toast, Windows runtime, and Wayland checks.

**Some real release gates remain open.** The 8-hour release soak passed on the
Linux host (see [Soak](soak.md)). The Windows packaging artifacts were built
and checksum-verified by CI, then **executed on Windows 11 on 2026-11-10**
(portable GUI, WebView2-present launch, real WASAPI Start/Stop and A/B,
firewall UAC approve/cancel, Sound Panel, autostart, i18n, update check); see
[Windows build](build-windows.md), [capture](capture-windows.md), and the
[GUI checklist](gui-acceptance-checklist.md). Two Windows runtime defects were
fixed (HTTPS update-check TLS backend and the WASAPI `Initialize` error
mapping), so a fresh CI run is required before the release artifacts are final.
Still not executed: the real v0.3.4 APK on a phone, sleep/resume,
real-hardware hotplug, Windows 10, a missing-WebView2 clean VM, and the real
Linux desktop tray. These are tracked as `ready-for-human` in tickets 13, 15,
and 16.

## Spikes (historical)

- [PipeWire capture spike](spikes/pipewire.md)
- [WASAPI loopback spike](spikes/wasapi.md)

Spike documents record pre-implementation measurements and may refer to the
legacy implementation for comparison.

## Decisions

- [ADR-0001: baseline is upstream v0.3.4](../adr/0001-server-rewrite-baseline.md)
- [ADR-0002: Rust + Tauri v2 + in-process core](../adr/0002-server-rewrite-stack.md)
- [ADR-0003: intentional deviations in 0.4.0](../adr/0003-server-rewrite-deviations.md)
