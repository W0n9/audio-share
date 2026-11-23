# pairing-ux closeout + code-review fix evidence

Snapshot date: 2026-11-23
Tip under test: `bb5deb0` (`integration/pairing-ux`)

- `2155b64` — Rust core: explicit Restore decision, `PersistMode`, rename.
- `1a70605` — Server GUI: `pairing_restore`, SAS reachable during re-pair,
  one success record, one request-card snippet.
- `bb5deb0` — Android: identity reuse on QR re-pair, persisted RePair intent,
  Forget ownership, shared address/repair rules, handoff-based device case.

Pre-review closeout tip: `818ee7b`; original ticket merge point: `d28e099`.
Status vocabulary follows `docs/agents/evidence-policy.md`. Nothing in this
file is a real-device PASS; the real device is `NOT RUN` throughout.

## Review-fix verification snapshot

| # | Command | Status | Result |
|---|---|---|---|
| 1 | `cargo test -p v2-transport -p v2-runtime -p as-cmd` | PASS | 357 passed, 0 failed, 1 ignored in total. See the `pairing_live` note: the 8 `pairing_live` cases passed 8/8 when rerun alone; the rest passed 349/0/1 across 35 test-result groups. |
| 2 | `cargo test -p audio-share-server-gui` | PASS | 114 passed, 0 failed, 1 ignored. |
| 3 | `cargo fmt --check` | PASS | exit 0, no output. |
| 4 | `cargo clippy --workspace --all-targets -- -D warnings` | PASS | exit 0, `Finished dev profile`. |
| 5 | `cd gui && npm ci && npm run check && npm run build` | PASS | `svelte-check`: 101 files, 0 errors, 0 warnings; `vite build`: 140 modules, built in 753 ms. |
| 6 | `cd android-client-v2 && ANDROID_HOME="$HOME/Android/Sdk" ./gradlew testReleaseUnitTest -PnoNative=true "-Porg.gradle.java.installations.paths=…/jdk-21.0.13+9"` | PASS | 331 tests, 0 failures, 0 errors, 0 skipped; `BUILD SUCCESSFUL in 26s`. |
| 7 | `cd android-client-v2 && ANDROID_HOME="$HOME/Android/Sdk" ./gradlew assembleReleaseAndroidTest -PnoNative=true -PandroidAbi=x86_64 "-Porg.gradle.java.installations.paths=…/jdk-21.0.13+9"` | PASS | `BUILD SUCCESSFUL in 30s`; R8-minified instrumented APK compiles. Execution is `NOT RUN` (no device). |

Notes on the snapshot:

- The ignored Rust test is `pipewire_list_endpoints_and_live_replay` (needs
  real headless PipeWire/wireplumber daemons), not a product failure.
- The ignored GUI test is `update::tests::fetches_the_real_latest_release_over_https`
  (needs network access to `api.github.com`), not a product failure.
- `pairing_live` (the known flake) hung once in the combined run on
  `operator_cancel_rejects_a_verified_pairing_with_201_promptly`, exceeding the
  10-minute timeout. Rerun alone immediately afterwards: 8/8 passed in 0.59 s.
  The combined run above therefore skips those 8 case names and adds the
  standalone 8/8 result. The root cause is still unproven.
- No `CARGO_TARGET_DIR` was set for any command.
- The JVM count was aggregated from the JUnit XML reports produced by this run.

## What the review fixes changed

- **Restore is explicit.** `ApprovalDecision::{Accept, Restore, Decline}` is
  carried through `DriverEvent`/`Event`; `persist_client` takes
  `PersistMode::{New, Restore}`. A plain Accept of a revoked record (or a
  Restore of a missing/active one) fails closed. GUI and `as-cmd` call the
  matching decision.
- **Re-pair opens both paths.** `admin.begin(Qr)` already opens the QR nonce and
  the SAS window; the active QR invitation now keeps a `Use pairing code`
  action, so a revoked row can start SAS re-pair.
- **Identity is preserved.** QR re-pair reuses a stored `keyAlias` when the
  payload pin matches, including records originally created by SAS.
- **Rotation keeps the context.** `UiStatePersistence` saves the RePair pin; the
  restored `PairingIntent.RePair` keeps the cause/instruction.
- **Forget is scoped.** `SessionOrchestrator.stopIfOwnedBy` stops only the
  forgotten Server's session; another Server's live session keeps running.
- **One address rule.** `ServerAddress` (strict `host:port`) is used by
  `PairingRepair`, `SessionOrchestrator`, and `AppState`; the free-form
  `addressHint` string no longer matches.
- **One repair-state rule.** `repairStateForOutcome` is shared by the Service
  and the UI projection.
- **No dead buttons.** `Connect`/`Connect again` for a `needsRepairing` record
  opens the RePair context instead of no-oping.
- **Device case fixed.** `pairSas` awaits the one-shot credential handoff
  instead of polling a `COMPLETE` snapshot the handoff clears.

## Per-ticket merge SHAs (historical closeout)

| Ticket | Merge/tip SHA | Subject |
|---|---|---|
| 01 Server Core | `34f3857` | Merge ticket 01 (`64e6d97` explicit Restore + delete) |
| 02 Server GUI | `d28e099` | Tip after ticket 02 (`3e6f050`, `cba8a91`, `f64294c`, `d28e099`) |
| 03 Android repair state | `afa4942` | Merge ticket 03 (`4e48ad8` … `0c7cd69`) |
| 04 Android repair flow | `fe38989` | Tip of ticket 04 (`b57d1c9`, `fe38989`) |

## Device evidence after the review fixes

- Real device: **`NOT RUN`** (no device scheduled).
- Emulator: **`NOT RUN`** in the review environment. `adb devices` listed no
  attached device and no emulator was running; the review-fix tip was not
  exercised on a device. Do not treat the earlier `d28e099` emulator results as
  evidence for `bb5deb0`. The release androidTest APK builds (row 7), so the
  instrumented case is only blocked on hardware, not on compilation.
- `pairQr`, QR re-pair, Forget (UI), and the full manual checklist (first
  pairing → revoke → QR re-pair → revoke → SAS re-pair → Forget): **`NOT RUN`**
  for the same reason.

Historical emulator evidence for the pre-review tip `d28e099` (Keystore, SAS
pairing, 204 revocation, SAS restore with the same `client_id`/`created_at`) is
preserved in the ticket comments and earlier revisions of this file. It does not
cover the review-fix changes.

## Known gaps (remaining)

- **Same-address cleanup deletes only the credential, not the identity key.**
  `SessionOrchestrator.removeStaleRecordsForAddress` calls `credentials.delete`
  only; the old record's Keystore/software identity key is left behind. Forget
  Server does delete the key, so this is a cleanup-only leak. Pre-existing and
  outside the 19 review findings.
- **`as-cmd clients delete` has no dedicated CLI test.** Store/admin tests and
  the earlier manual smoke cover it.
- **`pairing_live` flake.** Observed once in the review run; passed alone
  immediately after. Keep the 10-minute timeout and per-binary rerun advice.
- **Svelte/store logic has no test runner.** `gui/package.json` defines only
  `dev`, `build`, `check`, and `tauri`; behaviour is covered by `svelte-check`
  plus the production build.
- **Device coverage.** See the device section: every post-review device case and
  the real device are `NOT RUN`.

## Deferred after review

- None of the 19 review findings were deferred.
- The identity-key cleanup leak above was not one of the findings; it is left
  for a follow-up rather than widened into this pass.

## Files changed by this pass

Product code: `crates/v2-transport/**`, `crates/v2-runtime/**`,
`crates/as-cmd/src/main.rs`, `gui/src/**`, `gui/src-tauri/src/**`,
`android-client-v2/app/src/main/**`, `android-client-v2/app/src/test/**`,
`android-client-v2/app/src/androidTest/**`. Docs: this file and `spec.md`
status/acceptance wording. No ticket `Status:` line was changed.
