# 28: Beta final review fixes

**What to build:** Apply the two-axis final review findings on the beta integration branch.
Spec findings are correctness fixes; low-risk Standards findings are cleaned up, and the
deliberate non-fixes are recorded with a rationale.

**Blocked by:** None

**Status:** resolved

## Spec-axis fixes

- [ ] **Concurrent pairing identity.** `ApprovalRequest` is keyed by the per-connection
      `request_id`; two concurrent phones both using `request_id = 1` overwrite each other,
      so one can never be approved and a SAS/device pair can mismatch. Give each pending
      approval a unique local `approval_id` and use it for approve/decline/cancel and all
      admin surfaces. Keep `request_id` for the wire only.
- [ ] **Concurrent pairing cap.** pairing.md/TM-5 requires at most 8 concurrent pairing
      sessions; SAS currently has no nonce and can reach the 16-handshake transport cap.
      Enforce 8 with an RAII permit acquired on admission and released when the pending
      pairing is resolved or dropped; the 9th admission is rate-limited (302).
- [ ] **Offline admin vs running Core.** `as-cmd clients|identity` takes `clients.json.lock`
      per operation, but a running Core does not hold it, so `touch_last_seen` can rewrite
      the whole table and resurrect a just-revoked record; `identity reset` can also run
      while the old identity is serving. Have `V2Runtime` acquire and hold the store lock
      for its lifetime; add a non-blocking `try_acquire` and make offline admin fail with
      a clear "stop the Server first" error when the Core holds it. The GUI revokes
      in-process and is unaffected.
- [ ] **Cancel/deadline cleanup.** `PairingCore::cancel` clears approvals without notifying
      the state machine, so a verified client waits 10 minutes for 206; the deadline path
      does not remove the GUI's pending card. On operator cancel, decide `false` for every
      pending approval (201 + Close); on deadline/connection loss, remove the pending entry
      so admin surfaces stop showing it.
- [ ] **Release-note honesty.** Change the customer release notes claim "Windows is
      smoke-tested only" to the recorded `NOT RUN` status.

## Standards-axis cleanups (low risk)

- [ ] Extract the duplicate display-name normalization from `state_machine.rs` and
      `pairing.rs` into one helper.
- [ ] Deduplicate the identity-key generation block shared by `ServerIdentity::open` and
      `reset`.
- [ ] Unify `as-cmd` `confirm` / `confirm_exact`.
- [ ] Rename the admin-layer `PairingMode` so it no longer collides with `schema::PairingMode`
      and the Android `ClientPairingMode`; collapse the duplicate schema→admin mapping.

## Deliberate non-fixes (record, do not change)

- The Android native crate re-implements binding/SAS/TLV/SPKI; it is a standalone
  cross-compiled workspace and an independent interop oracle, not a shared server crate.
- `as-cmd/src/lib.rs` re-exports `v2_runtime::*` for compatibility; it is not accidental
  indirection.
- UI/pairing mode/phase strings at the JSON/JNI boundary stay strings; typed enums there
  would add conversion layers without a beta need.

## Answer

All Spec-axis fixes and the low-risk Standards cleanups are implemented on
`ticket/28-review-fixes` (commit `4905f56`); evidence is in
`.scratch/wire-protocol-v2/evidence/ticket-28/`.

Key decisions:

- Every pending approval now has a process-local `approval_id`; `request_id`
  never appears in admin surfaces. Removal on deadline/connection loss uses the
  `approval_id`, not `request_id`: the latter is only unique per connection, so
  a global lookup by it would reintroduce the cross-connection collision this
  ticket fixes.
- The eighth-session cap is enforced by an RAII `PairingPermit` returned from
  `admit_request`; the state machine owns it, so approval, decline, deadline,
  and connection loss all release the slot.
- `V2Runtime` holds `StoreLock` for its lifetime, acquired with the new
  non-blocking `StoreLock::try_acquire`. A second Core or offline admin fails
  fast with "stop the Server first" instead of blocking. `touch_last_seen` is
  unchanged; with a single Core owner the external-writer race is gone.
- `PairingCore::cancel` declines every pending exchange (201+Close immediately)
  and removes the pending entries on deadline/close/transport loss.
- Windows Server remains `NOT RUN`; the release note no longer claims a smoke
  test.

Deliberate non-fixes (unchanged, with rationale):

- **Android native crate duplication**: it is a separately cross-compiled
  workspace that also serves as an independent interop oracle. Sharing server
  crates would couple the two ends and weaken that oracle; the duplication is
  intentional.
- **`as-cmd/src/lib.rs` re-exports `v2_runtime::*`**: this is the supported
  compatibility surface used by the existing CLI/public imports, not accidental
  indirection. Removing it would churn callers for no beta benefit.
- **String enums at JSON/JNI boundaries**: typed enums would add
  schema↔UI/JNI conversion layers for beta without changing protocol behavior;
  the boundary strings are stable frontend contract. Deferred until a concrete
  need (e.g. a third mode) appears.
