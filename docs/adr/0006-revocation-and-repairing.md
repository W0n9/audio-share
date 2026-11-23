# ADR-0006: Revocation is reversible only by an explicit Restore; Re-pairing never silently reactivates a device

Status: accepted (2026-11-23)

## Context

v0.6.0 shipped Revocation and Re-pairing with three inconsistent outcomes. In QR mode the
Client key alias is derived from the Server pin (`ControlSessionService.kt:243`), so a
re-pairing reused the same identity and the Server's `upsert` replaced the revoked record and
wrote it back as `Active` (`auth_store.rs:532-551`, `pairing.rs:1086-1094`) — revocation was
silently undone. In SAS mode a fresh random identity is minted
(`ControlSessionService.kt:273`), so the revoked record stayed and a duplicate active row
appeared. The Server GUI exposed only Revoke: no Delete and no Restore. Meanwhile
`pairing.md` §8.3 already required pairing replacement to be an explicit admin action. The
Client kept showing PAIRED, offered no Forget action, and auto-connected after a cold start
because the outcome store is memory-only. Users could not tell whether a revocation was
final or merely until the next scan.

## Decision

1. **Revocation is reversible, but only through an explicit Restore.** If a pairing presents
   an identity whose stored record is revoked, the Server asks "Restore this device?" before
   persisting. A plain Accept never silently reactivates a revoked record.
2. **Re-pairing preserves identity for the same phone.** QR keeps deriving the key alias from
   the Server pin; SAS reuses the stored record's key alias when the typed address matches a
   stored Server.
3. **A successful pairing replaces records for the same address.** It never replaces a record
   for a different Server pin without the user explicitly pairing that Server.
4. **The Client persists a needs-re-pairing state on the credential**, set by a surfaced close
   that requires re-pairing and cleared by successful authentication or pairing. Home and
   Settings show it. The Client offers Forget Server (credential plus identity key); it never
   deletes a credential silently.
5. **Server surfaces delete only after revocation.** An active device must be revoked first,
   so one click cannot both terminate a session and erase the audit row. `as-cmd clients
   delete` mirrors the GUI.
6. **No Wire Protocol change.** The whole model is local to the Server Core, the Server GUI,
   and the Android Client.

## Considered Options

- **Terminal revocation** — every re-pairing creates a new identity. Stronger in principle,
  but it requires rotating the QR alias derivation, leaves duplicate audit rows forever, and
  has no counterpart in §8.3's explicit-replacement model.
- **Keep silent reactivation** — the operator cannot tell whether revocation was final.
- **Server-initiated restore over the wire** — v2.0 has no Client-to-Server management
  channel; out of scope.

## Consequences

- The Server must correlate an incoming pairing with an existing revoked record at approval,
  and the GUI must carry that record's `client_id` for as long as the invitation is open.
- The Client gains one persisted credential field and a key-deletion path;
  `CredentialStore.delete` finally gets a caller.
- Tests must cover QR/SAS parity: same identity, same `client_id`, one row, a Restore
  confirmation, and no auto-connect while re-pairing is required.
