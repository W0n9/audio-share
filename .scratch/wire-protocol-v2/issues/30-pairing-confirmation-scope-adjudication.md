# 30: Pairing confirmation scope and QR invitation recovery

**What to build:** Record the maintainer adjudication for the v2 pairing confirmation model and
the QR invitation-recovery UX, and land the matching documentation changes. This ticket changes
no protocol and no implementation code; the implementation is split into tickets 31 and 32.

**Blocked by:** None

**Status:** resolved

**Type:** task

## Decisions

- **D1 — §7.3 scope.** The "both ends require an explicit user action" rule in
  `pairing.md` §7.3 applies to **SAS mode only**. QR mode is governed by §6.2: the Client's
  explicit action is the scan, and the Server still requires local approval.
- **D2 — Server approval.** Keep "Accept device" / `as-cmd` confirmation in all modes as the
  authorization gate. No auto-accept.
- **D3 — QR refresh.** Hide a consumed or expired QR immediately and stop its countdown; offer an
  in-place "Refresh QR" action. Auto-renew only on natural expiry while the pairing dialog is
  open and no pairing is in flight. Never auto-renew after a consumed or failed attempt.
- **D4 — Android invalid QR and timeout.** On `201` (nonce invalid/expired/replayed, or pairing
  rejected), close the scanner and show a blocking state directing the user to refresh the QR on
  the Server, with an explicit "Scan again" action. On `206` (the 10-minute pairing exchange
  deadline), show "pairing timed out, start a new pairing"; also require an explicit rescan. Do
  not auto-reopen the camera in either case.
- **D5 — QR one action.** After the Client verifies `binding`, it sends `PairingConfirm` without a
  separate Confirm tap. SAS mode keeps code entry plus explicit Confirm.
- **D6 — SAS in QR mode.** Comparison is not a completion gate. The Server GUI keeps the code in
  the live/pending cards for operator awareness; the Android Client shows the fingerprint during
  the transition instead of blocking on a comparison.
- **D7 — Documentation.** Land the scope clarifications in `pairing.md` (§6.2, §7.3, PR-15), the
  beta-spec implementation decisions, the pairing vocabulary in `GLOSSARY.md`, and the market
  survey in `docs/research/`.

## Checklist

- [x] `docs/wire-protocol-v2/pairing.md` scopes §7.3 to SAS and records PR-15.
- [x] `.scratch/wire-protocol-v2/beta-spec.md` records QR one-action pairing and the
      invitation-refresh behavior.
- [x] `GLOSSARY.md` defines the pairing vocabulary.
- [x] `docs/research/pairing-ux-market-survey.md` is ready to be tracked.
- [x] Implementation tickets 31 (Server) and 32 (Android) are opened.

## Evidence

- Primary text: `docs/wire-protocol-v2/pairing.md` §5, §6.2, §6.3, §7.3;
  `.scratch/wire-protocol-v2/beta-spec.md` user story 5; ticket 29.
- Market survey: `docs/research/pairing-ux-market-survey.md` (some product specifics are marked
  `NOT FOUND` or taken from search-indexed official pages; see its source-strength notes).

## Comments

### 2026-11-17 — maintainer adjudication

The maintainer accepted the recommendations across two grilling rounds: §7.3 is SAS-only; the
Server Accept stays; QR refresh is explicit with expiry-only auto-renew; Android shows a blocking
rescan state; QR becomes one action; SAS comparison is optional in QR mode; docs and glossary are
updated. Implementation is split into tickets 31 and 32.
