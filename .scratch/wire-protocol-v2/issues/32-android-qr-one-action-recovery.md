# 32: Android QR one-action pairing and invalid-QR recovery

**What to build:** In QR mode, scanning is the Client action: once the native pairing state
reaches `AWAITING_CONFIRMATION` and the binding is verified, the app sends `PairingConfirm`
automatically, with no separate Confirm tap. If the QR is rejected as consumed or expired
(`201`) or the pairing exchange reaches its deadline (`206`), show a blocking state that directs
the user to refresh the QR on the Server (or start a new pairing) and offers an explicit rescan;
never auto-reopen the camera. SAS mode keeps code entry plus explicit Confirm.

**Blocked by:** 30

**Status:** resolved

**Type:** task

## Requirements

- [x] In **QR mode only**, auto-dispatch `ConfirmPairing` exactly once when the pairing snapshot
      reaches `AWAITING_CONFIRMATION`. Guard against duplicate snapshots and Activity
      recreation; do not auto-confirm in SAS mode.
- [x] The Client still verifies the pairing `binding` before sending `PairingConfirm`. This is
      existing native behavior: binding is checked in `control.rs:444-453` before the phase
      becomes `AWAITING_CONFIRMATION` in `control.rs:474-476`. Do not bypass or duplicate it.
- [x] QR pairing UI: replace the standalone Confirm button with a transition state such as
      "Pairing with <fingerprint>…"; then show "Confirmed, waiting for the Server to accept"
      while the Server's approval is outstanding. SAS mode keeps the code input and Confirm.
- [x] On `201` (`PAIRING_REJECTED`: nonce invalid/expired/replayed, or pairing rejected), close
      the scanner and show a blocking message: the QR is no longer valid and the Server must show
      a fresh one. Provide an explicit **Scan again** action; do not auto-reopen the camera.
- [x] On `206` (`PAIRING_PENDING_TIMEOUT`: the 10-minute pairing exchange deadline), show
      "pairing timed out, start a new pairing"; require the same explicit rescan and do not
      auto-reopen the camera.
- [x] If the Server declines after the auto-confirm, surface the existing `201` guidance and keep
      the Server Accept requirement visible in the copy.
- [x] Tests: reducer tests for exactly-once QR auto-confirm and SAS non-regression; error-guidance
      test for `201`/`206`; emulator/manual evidence under `evidence/ticket-32/` (record
      `EMULATOR` when that is what was run).
- [x] No wire-protocol, accepted-document, ADR, v1, fixture, or frozen-APK changes.

## Notes

- `pairing.md` §6.2 (2026-11-17 clarification): QR mode's Client action is the scan; the Server
  still requires local approval. §7.3's two-end confirmation is SAS-only (ticket 30, D1).
- The SAS remains visible as an optional operator aid in QR mode; it is not a completion gate
  (ticket 30, D6).
- The Server GUI work that hides dead QRs and offers refresh is ticket 31.

## Answer

2026-11-17 — implemented on `integration/wire-protocol-v2-beta`.

- `AppReducer` now auto-dispatches `ConfirmPairing` exactly once per QR exchange when the native
  snapshot reaches `AWAITING_CONFIRMATION`; `PairingUi.autoConfirmSent` suppresses duplicate
  snapshots, and the native binding check that gates the phase is untouched. If the Service
  rejects the queued intent and the native phase never advances, the reducer re-arms after
  `QR_AUTO_CONFIRM_RETRY_SNAPSHOTS` (~1 s) so the screen cannot wedge; the Service gate still
  drops the re-armed intent once a confirm has been queued, so at most one reaches native.
- `PairingConfirmGate` in `ControlSessionService` makes the one confirmation durable across
  Activity recreation: a replayed snapshot cannot queue a second confirm (the native confirm
  channel would otherwise close the session with 402), and the gate resets only for a new pairing
  exchange.
- QR UI shows "Pairing with <fingerprint>…" with no Confirm button, then "Confirmed, waiting for
  the Server to accept" while `CONFIRMING`; the SAS stays visible as an optional aid in both QR
  states, and SAS mode keeps the code entry plus explicit Confirm.
- `201`/`206` failures render a blocking pairing card with mode-neutral fresh-invitation guidance
  and an explicit **Scan again** action in QR mode. The scanner stays closed; nothing reopens the
  camera automatically. `SessionError` and the pairing snapshot may arrive in either order
  without losing the blocking state, and a relay clear does not erase it.
- Error guidance copy for `201` names a fresh QR or pairing code plus the Server Accept gate;
  `206` is "Pairing timed out" with the ten-minute deadline explanation.
- Tests: exactly-once auto-confirm, duplicate snapshots and the re-arm bound, SAS non-regression,
  201/206 blocking in both event orders, explicit-rescan-only, guidance copy, and the Service gate
  (118 Android unit tests PASS).
- Evidence: [`evidence/ticket-32/`](../evidence/ticket-32/README.md). Emulator PASS for QR
  one-action pairing and the post-auto-confirm `201` decline state; the `CONFIRMING` label was
  verified in Robolectric (emulator approval window too short to capture). `206` and real-device
  runs are `NOT RUN`; `206` is covered by reducer/guidance tests.
- No wire-protocol, accepted-document, ADR, v1, fixture, or frozen-APK changes.
