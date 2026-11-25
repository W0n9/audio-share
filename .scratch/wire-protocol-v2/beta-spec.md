# Wire Protocol v2 — Customer Beta Specification

Status: ready-for-agent
Feature: wire-protocol-v2
Date: 2026-11-16
Type: spec

This is a **scoped beta specification**, not a revision of the accepted design baseline
(`spec.md`) or the normative protocol. Where this document is silent, the baseline,
`docs/wire-protocol-v2/spec.md`, `pairing.md`, `threat-model.md`, and ADR-0004 remain
authoritative.

## Problem Statement

The released v1 Android Client works, but it has no encryption, no authentication, and no
identity — it is only safe on a trusted network. Wire Protocol v2 has a validated QUIC
transport, an authenticated control channel, and single-Client PCM media, but a customer
cannot use it:

- Production pairing does not exist. The Server refuses `PairingRequest`, the authorization
  store is writable only through a test-only path, and `as-cmd` has no pairing command.
- The v2 Android app is a smoke harness. Server address, pin, and key must be injected through
  adb intent extras; there is no address entry, pairing screen, or usable playback UI.
- The Server GUI knows nothing about v2 pairing or paired devices.
- M3-era lifecycle tests (`lock`, `recreate`) have not been re-run since M4 introduced media;
  the `lock` assertion requires `READY` even when the session is legitimately `PLAYING`, and
  `recreate` shows a startup race. Current acceptance evidence therefore points at pre-M4 logs.

External pilot customers need a controlled-trial beta: pair a phone with a Server, play
LAN/VPN audio, survive screen lock and a temporary network drop, and recover from revocation
or a Server identity reset — with known limitations documented. Real-device time is scarce, so
functional work must be provable on the Android emulator without being mistaken for
real-device acceptance.

## Solution

Ship a **customer beta** consisting of:

- a new **v2 Android Client** with a minimal Compose UI and production pairing
  (QR primary, SAS/fingerprint fallback, Android Keystore), published as a signed APK;
- a **Server beta** with a shared Pairing Core, `as-cmd` pairing/administration commands, and a
  Server GUI pairing dialog with a paired-device list and revocation;
- v1 untouched, still enabled by default, clearly labeled as unencrypted and trusted-network
  only;
- one GitHub Release containing the signed APK, Windows portable Server, Linux deb/AppImage,
  and `as-cmd`, with checksums, v1 compatibility notes, and upgrade/rollback steps.

The beta is a **controlled trial**, not a v1 daily-driver replacement. It does not include the
M5 impairment work, multi-Client operation, diagnostics export, F-Droid distribution, or the
v1 independent disable switch. Those remain stable-gate work.

## User Stories

### Pilot customer

1. As a pilot customer, I want to install the beta APK alongside the v1 app, so that I can
   compare them and fall back without uninstalling either.
2. As a pilot customer, I want to see the beta version and its known limitations before
   pairing, so that I know what to expect from a controlled trial.
3. As a pilot customer, I want to enter or select the Server address, so that the app knows
   where to connect.
4. As a pilot customer, I want the last Server address remembered, so that I do not retype it
   every time.
5. As a pilot customer, I want to scan a QR code shown by the Server, so that pairing is one
   action.
6. As a pilot customer without a usable camera, I want to enter an SAS code or compare a
   fingerprint, so that I can still pair.
7. As a pilot customer, I want the pairing invitation to be single-use and short-lived, so
   that a leaked QR or code cannot be replayed.
8. As a pilot customer, I want the app to remember the pairing, so that later connections do
   not require pairing again.
9. As a pilot customer, I want to see `CONNECTING`, `AUTHENTICATING`, `READY`, `PLAYING`, and
   `CLOSED` states, so that I know what the app is doing.
10. As a pilot customer, I want to play the Server's audio, so that the phone acts as a
    speaker.
11. As a pilot customer, I want to stop playback and disconnect, so that I control the
    session.
12. As a pilot customer, I want audio to continue while the screen is locked, so that I can
    pocket the phone.
13. As a pilot customer, I want a temporary network drop to recover automatically, so that I
    do not have to re-pair.
14. As a pilot customer, I want clear error codes and retry guidance, so that I can resolve
    common problems myself.
15. As a pilot customer, I want a re-pair flow after a device revocation or Server identity
    reset, so that I can recover access.
16. As a pilot customer, I want the app to tell me that v1 is unencrypted and trusted-network
    only, so that I choose the right client for the network.
17. As a pilot customer, I want to download updates manually from a release page with
    checksums, so that I can verify what I install.
18. As a pilot customer, I want to report issues through the agreed channel, so that problems
    are recorded and fixed.
19. As a pilot customer, I want no telemetry and no raw audio recording, so that my privacy is
    respected.
20. As a pilot customer running a Windows or Linux Server, I want the same pairing experience,
    so that the OS does not change the workflow.

### Server operator

21. As a Server operator, I want to start pairing from the GUI, so that I do not need a
    terminal for normal operation.
22. As a Server operator, I want to see a QR code containing the address hint, Server identity,
    and a short-lived nonce, so that the phone can scan it.
23. As a Server operator, I want to see the SAS code and identity fingerprint, so that I can
    pair a phone without a camera.
24. As a Server operator, I want the pairing dialog to show expiry and offer cancel, so that a
    stale invitation does not remain open.
25. As a Server operator, I want to see the paired devices, so that I know which phones are
    authorized.
26. As a Server operator, I want to revoke a device, so that a lost or retired phone loses
    access.
27. As a Server operator, I want revocation to terminate the affected active session promptly,
    so that the lost phone stops playing.
28. As a Server operator, I want to reset the Server identity with a confirmation step, so that
    old pairings are invalidated without a hidden recovery backdoor.
29. As a headless operator, I want `as-cmd` commands to pair, list clients, revoke a client, and
    reset identity, so that I can run without the GUI.
30. As a Server operator, I want v1 to remain enabled and clearly labeled, so that I have a
    manual fallback.
31. As a Server operator, I want the beta Server on Windows 11 and Linux, so that I can use my
    platform.
32. As a Server operator, I want logs that never contain private keys, pairing secrets, or raw
    audio, so that I can share them during support.
33. As a Server operator, I want the beta Server to read an existing v1 configuration without
    breaking it, so that upgrading is safe.

### Engineer and agent

34. As an engineer, I want all pairing logic in one protocol-neutral core, so that the CLI and
    GUI cannot diverge.
35. As an engineer, I want the Android UI to depend on a session/pairing interface rather than
    JNI details, so that UI tests do not need a real network or a real Server.
36. As an engineer, I want golden vectors and live-QUIC interop tests, so that Rust and Android
    are tested against each other, not only against their own encoders.
37. As an engineer, I want emulator results recorded as a separate evidence status, so that
    functional progress is not mistaken for real-device acceptance.
38. As an engineer, I want the stale `lock` and `recreate` lifecycle tests corrected and re-run,
    so that beta acceptance evidence reflects M4 and later.
39. As an engineer, I want the beta artifacts signed and reproducible, so that customers can
    verify them.
40. As an engineer, I want the frozen v1 fixtures and the v0.3.4 APK untouched, so that the
    compatibility contract continues to hold.

## Implementation Decisions

### Server: one Pairing Core

- Introduce a protocol-neutral **Pairing Core** shared by `as-cmd` and the Server GUI. It owns:
  beginning a pairing (QR or SAS mode), the single-use short-lived nonce, QR payload
  construction, SAS derivation and presentation, fingerprint presentation, completing a
  pairing and persisting the authorized client, listing clients, revoking a client, and
  resetting the Server identity.
- Replace the current `PairingRequest` refusal with the production admission flow already
  specified in `pairing.md`. Preserve the existing malformed-message precedence, no-detail
  pre-authentication errors, rate limits, and fail-closed behavior.
- Authorized clients are persisted in the existing authorization store with canonical
  `client_id`, 33-byte SEC1-compressed P-256 public keys, `active`/`revoked` state, atomic
  writes, one backup generation, and POSIX permissions (directory 0700, file 0600).
- Revocation takes effect for new connections immediately and terminates affected active
  sessions. Identity reset invalidates all existing pairings and requires re-pairing; there is
  no universal recovery path.
- Pairing mode, nonce, and SAS state are never written to logs.

### Server: CLI and GUI adapters

- `as-cmd` gains a headless pairing path: begin pairing (with QR output where practical and
  SAS/fingerprint always available), list clients, revoke a client, and reset identity with a
  confirmation step.
- The Server GUI gains a pairing dialog that shows the QR code, SAS code, fingerprint, and
  expiry, with cancel; and a paired-device list with revoke. It does not gain a live v2
  session/diagnostic view in beta.
- The pairing dialog never leaves a consumed or expired QR on screen: it hides the stale QR and
  countdown, offers an in-place refresh, and renews automatically only on natural expiry while
  no pairing is in flight. A failed attempt never reuses the same invitation.
- v1 remains enabled by default and byte-compatible. Its “trusted networks only; no
  encryption or authentication” status is stated in `as-cmd --help` and the release notes. The
  independent v1 disable switch is deferred to stable.
- The beta Server version is `0.5.0-beta`. Windows 11 and Linux (Ubuntu 24.04) are supported;
  Linux is the full-acceptance platform, Windows is smoke-tested.

### Android Client: session/pairing interface

- Extend the existing native session + Kotlin service seam with pairing commands and state:
  begin QR/SAS pairing, submit scanned payload or SAS input, complete pairing, store/reload the
  paired credential, connect, stop, and expose state events.
- The Android Client identity remains ECDSA P-256. Private key material lives in Android
  Keystore where available; the paired credential and Server pin persist across app restarts.
- Production builds contain no test-only identity or addressing path; the test-support fixture
  remains behind its explicit build flag.
- Media behavior is unchanged from M4: ordered delivery before playback, `AudioTrack` output,
  foreground media Service, periodic diagnostics, and explicit close. No jitter buffer,
  concealment, or drift correction is added in beta; those are documented limitations.
- Reconnection uses the existing bounded exponential backoff with jitter. A v2 failure never
  falls back to v1 automatically.

### Android Client: minimal Compose UI

- Replace the smoke harness with a minimal Compose UI:
  - **Home**: Server address, pairing/connect entry, current state, error and retry guidance.
  - **Pairing**: QR scanning, SAS entry, fingerprint display, cancel. QR mode is one action:
    after the Client verifies the pairing transcript it sends `PairingConfirm` without a
    separate Confirm tap; the Server still requires "Accept device". The SAS stays visible as
    an optional operator aid and is not a completion gate in QR mode.
  - **Playing**: state, stop action.
  - **About**: beta version, known limitations, v1 warning, update instructions.
- Persist the last Server address and auto-connect when a pairing exists. Do not start on boot.
- Foreground notification identifies the media session and offers stop.
- The UI consumes only the session/pairing interface; it does not call JNI, QUIC, or Keystore
  directly.
- Android app version is `0.2.0-beta` (`versionCode` 2) with `applicationId` `dev.audioshare.v2`,
  so it coexists with the v1 app.

### Release and pilot

- Create a dedicated release keystore. Build and publish signed beta artifacts; keep v1
  artifacts available. Do not publish the beta to F-Droid.
- One GitHub Release contains the signed APK, Windows portable Server, Linux deb/AppImage,
  `as-cmd`, SHA-256 checksums, v1 compatibility notes, and upgrade/rollback steps.
- Updates are manual; there is no in-app auto-update.
- Pilot shape: 1–3 external customers, 2–4 weeks of use, feedback through the agreed ad hoc
  channel with a private internal log. No telemetry.
- Beta exit criteria: two consecutive weeks without P0/P1 pairing or playback defects, and the
  stable-gate items scheduled.
- Target window: 3–5 weeks from 2026-11-16, target 4 weeks (around 2026-12-14). No hard date.

### Evidence policy

- Record `EMULATOR` as a formal evidence status alongside `PASS`/`FAIL`/`NOT RUN`/`WAIVED`.
  Emulator evidence can satisfy functional UI, protocol, lifecycle, and API 26 runtime checks.
- Emulator evidence can never replace: audible quality, end-to-end latency, underrun behavior,
  MIUI/vendor-specific behavior, or the 60-minute two-real-device gate.
- Correct the stale lifecycle tests: the lock case accepts `READY` or `PLAYING` and separately
  asserts the foreground Service and session are alive; the recreate case is investigated and
  fixed for the duplicate `ACTION_START` race. Both are re-run on the real device.

## Testing Decisions

A good test asserts externally observable behavior at an agreed seam, not private methods or
implementation details. Prefer the highest existing seam; do not add per-packet or per-widget
test hooks.

### Seam 1 — Server Pairing Core

- Host-level tests cover the pairing lifecycle: begin QR/SAS, nonce single-use and expiry,
  malformed and replayed requests, successful completion, persistence, list, revoke, and
  identity reset.
- Golden vectors cover the pairing control frames and SAS derivation. The existing vector
  replay must not be rewritten to fit the implementation.
- State-machine tests cover `PairingRequest` in legal and illegal states, rate limits, and
  no-detail error rules.
- Prior art: the existing host live-QUIC and state-machine suites in `v2-transport`.

### Seam 2 — Android Client session/pairing interface

- Native contract tests cover pairing transcript generation/verification, credential reload,
  authorization failure, and revocation handling.
- JVM tests cover the Service/state relay, including pairing states and state replay after
  Activity recreation.
- Instrumentation covers pairing, connect, playback, lock, reconnect, stop, and revoke on the
  emulator, driven by the existing device-test harness.
- Production builds are tested with the test-support flag off; test identities exist only in
  explicitly flagged builds.

### Cross-end interop (existing harness)

- Extend the Rust↔Android interop harness with pairing and authorization cases: QR transcript,
  SAS transcript, wrong fingerprint, unknown identity, revoked client, identity change, and
  live-QUIC pairing completion against a real Android receiver/state machine.
- Do not test only one end’s encoder. Golden vectors remain the independent oracle.

### UI and GUI

- Android UI state transitions are tested through the session/pairing interface and a state
  reducer; key Compose screens get UI tests for address entry, pairing choice, error display,
  and stop. No pixel-level assertions.
- The Tauri controller is tested against a fake Pairing Core; the Svelte frontend is checked
  with `svelte-check` and the production build. Pairing dialog behavior is also exercised in
  the release acceptance pass.

### Acceptance evidence

- **Emulator**: full functional suite, including API 26 runtime, with results recorded as
  `EMULATOR`.
- **One real arm64/API 31 device**, one session: install the signed beta; pair by QR; pair by
  SAS; `READY`→`PLAYING`; 30 minutes continuous playback; lock/wake continuity; temporary
  network drop and recovery; revoke terminates the active session; re-pair; v1 APK coexistence
  smoke.
- **Windows**: Server GUI pairing plus a 5-minute playback smoke from the real phone, or an
  explicit `NOT RUN` when no Windows host is available.
- v1 fixtures, replay, and the frozen v0.3.4 APK remain part of regression.

## Out of Scope

- Full M5 jitter buffer, concealment, drift correction, impairment matrix, fuzzing, and the
  60-minute no-regression run. Known media limitations are documented for the pilot.
- Multi-Client isolation, concurrent phones, Server soak, and the two-real-device 60-minute
  gate.
- Server GUI live v2 session/diagnostic view.
- Redacted diagnostics export and any external telemetry.
- GUI identity reset; beta exposes identity reset through `as-cmd` only.
- The independent v1 disable switch; v1 is labeled but not switchable in beta.
- F-Droid distribution and in-app auto-update.
- Boot start or background restart after reboot.
- mDNS discovery, STUN/TURN, NAT hole punching, relays, accounts, browser/iOS clients.
- Opus, FEC, DTX, codec-specific PLC, and application-layer retransmission.
- Revision of the accepted design baseline, normative protocol, ADR-0001, or ADR-0004.
- Changing or rebuilding the frozen v0.3.4 APK or v1 golden fixtures.

## Further Notes

- **Risks**: production pairing is the largest unknown — no production code exists today. The
  real-device schedule is the external bottleneck; the single acceptance session must be
  booked. M5 media quality is unmeasured, so the pilot may expose glitches that are accepted
  as documented limitations. Windows smoke may end as `NOT RUN`. Pilot feedback is ad hoc, so
  the internal defect log is the source of truth for exit criteria.
- **Stable blockers not in beta**: the frozen `media.sequence-wrap` timestamp contradiction,
  the fixture server key’s PKCS#8 v1 incompatibility, v1’s independent disable switch,
  diagnostics export, multi-Client isolation, and the full acceptance matrix remain stable-gate
  work.
- **Evidence honesty**: do not record host stubs, Java compilation, emulator runs, or
  single-end encoder tests as real-device or interop PASS. Use `EMULATOR` where that is what
  was run.
- **Compatibility**: v1 framing, commands, UDP behavior, and little-endian layout stay frozen.
  v2 failure never silently downgrades to v1, plaintext, or anonymous encryption.
- **Repository conventions**: `.scratch/` stays untracked; specs, roadmap, evidence, and
  release notes are not committed unless the maintainer asks. Do not modify `roadmap.md` or
  the accepted protocol documents without maintainer adjudication.
- **Confirmation-model adjudication (2026-11-17)**: `pairing.md` §7.3's two-end confirmation is
  SAS-only; QR pairing is one action on the Client, with the Server's Accept retained as the
  authorization gate. See `docs/research/pairing-ux-market-survey.md` and ticket 30.
- **Vocabulary**: Beta, Experimental, and Stable are defined in `GLOSSARY.md`. Beta is a
  controlled-trial Client release; Experimental is the Server-side protocol status before the
  stable gate; Stable requires the full acceptance gate, migration guide, and rollback path.
