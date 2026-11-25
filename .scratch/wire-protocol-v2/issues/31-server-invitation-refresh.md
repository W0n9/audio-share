# 31: Server invitation expiry/consumption and in-place QR refresh

**What to build:** The Server GUI must never present a dead QR. When a pairing invitation's nonce
is consumed or its lifetime expires, hide the QR and stop the countdown, explain why, and offer an
in-place refresh. Renew automatically only on natural expiry while no pairing is in flight.

**Blocked by:** None

**Status:** resolved

**Type:** task

## Requirements

- [x] Pairing Core / `PairingAdmin` expose the lifecycle state of the current QR invitation:
      **active**, **consumed**, or **expired**. Consumption is the first well-formed
      `PairingRequest` presenting the nonce, regardless of the pairing's outcome
      (`pairing.md` §5).
- [x] `controller.pairing_status` surfaces that state (and the existing expiry) to the GUI. The
      dialog's `active` flag is true only while the invitation is usable or a live/pending
      pairing exists.
- [x] On **consumed** or **expired**, the Server GUI hides the QR image and stops the countdown,
      shows a localized "this QR is no longer valid" message, and offers a **Refresh QR** button.
- [x] Refresh generates a new nonce/QR **in place**, without requiring Cancel and without
      closing the dialog. Reuse the same `begin(Qr)` path as the dialog's initial QR start so the
      SAS admission window is renewed with it (the Qr branch opens both; `admin.rs:166-177`).
      Do not call `begin_qr` alone. Do not add a second source of pairing state.
- [x] Automatic renewal happens only when all are true: the dialog is open, the invitation
      expired naturally, and there is no live or pending pairing. Guard against repeated renewal
      on status polling. Never auto-renew after a consumed or failed attempt.
- [x] `as-cmd pair` keeps its one-shot behavior. After a failed attempt it prints a clear message
      that the invitation is single-use and the operator must run `as-cmd pair` again; it does
      not silently re-run.
- [x] Tests at public seams: core active→consumed/expired transitions; controller status; GUI
      store/controller refresh behavior, including the no-auto-renew-after-consumption case.
      (Store-specific unit test WAIVED: no frontend test harness; the store delegates to
      `controller.pairing_poll`, and `svelte-check`/build pass.)
- [x] No wire-protocol, accepted-document, ADR, v1, fixture, or frozen-APK changes.
- [x] Manual Linux GUI check PASS (active QR, consumed state, in-place refresh); Windows GUI
      smoke `NOT RUN` (no Windows host). Evidence under `evidence/ticket-31/`.

## Notes

- The nonce table is memory-only and at most 8 outstanding (`pairing.md` §5). A refresh adds one
  nonce; the bound and oldest-eviction rule stay as they are.
- A consumed nonce is already removed by `admit_qr`; the core needs to retain enough state to
  report "consumed" to the local admin surface. This is a local admin/API change, not a protocol
  change.
- The GUI approval-card semantics are unchanged: the Server still requires "Accept device" after
  the Client confirms (ticket 30, D2).

## Answer

2026-11-17 — implemented on `integration/wire-protocol-v2-beta`.

- Pairing Core tracks the current QR invitation and exposes `InvitationState`
  (active/consumed/expired); a consumed invitation stays consumed after its expiry.
- `PairingAdmin`/`CorePairingAdmin` expose `InvitationStateView`; the controller maps it once into
  `PairingInvitationState` (serde lowercase) and `pairing_status` keeps an expired invitation
  visible instead of dropping it.
- `pairing_poll` renews only when the **core** reports natural expiry, with no live/pending
  pairing and no concurrent/blocked renewal. The truncated local countdown and an unknown core
  state (for example after a core restart) never trigger renewal.
- GUI hides the QR/countdown on consumed/expired, shows the localized message, and offers an
  in-place Refresh QR that uses the same `begin(Qr)` path (the SAS admission window is renewed
  too).
- `as-cmd pair` prints the single-use warning at start, on decline, on Ctrl-C, and on timeout; it
  never re-runs automatically.
- Tests: core lifecycle; admin mapping; controller status/poll including consumed-no-renew,
  unknown-core-no-renew, core-active-vs-truncated-deadline, and live-pairing-no-renew.
- Evidence: [`evidence/ticket-31/`](../evidence/ticket-31/README.md). Manual Linux GUI PASS
  (active, consumed, refresh); natural-expiry auto-renew `NOT RUN` manually (controller test
  PASS); Windows `NOT RUN`; store unit test `WAIVED` (no frontend harness).
- No wire protocol, accepted-doc, ADR, v1, fixture, or frozen-APK changes.
