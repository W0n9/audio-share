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
| `crates/as-cmd` | Headless CLI entry point. Thin wrapper over the core; preserves the v0.3.4 flags and adds `server.log`, graceful Ctrl-C, and validated arguments. |
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
- Acceptance and soak evidence (`acceptance.md`, `soak.md`) is produced by the
  ticket 15 release gate and will be linked here when it lands on the
  integration branch. It is not part of this index yet.
- The GUI acceptance checklist is maintained with the rewrite notes; the
  release gate executes it on real desktops.

## Spikes (historical)

- [PipeWire capture spike](spikes/pipewire.md)
- [WASAPI loopback spike](spikes/wasapi.md)

Spike documents record pre-implementation measurements and may refer to the
legacy implementation for comparison.

## Decisions

- [ADR-0001: baseline is upstream v0.3.4](../adr/0001-server-rewrite-baseline.md)
- [ADR-0002: Rust + Tauri v2 + in-process core](../adr/0002-server-rewrite-stack.md)
- [ADR-0003: intentional deviations in 0.4.0](../adr/0003-server-rewrite-deviations.md)
