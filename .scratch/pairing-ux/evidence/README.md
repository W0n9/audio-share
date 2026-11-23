# pairing-ux closeout evidence

Snapshot date: 2026-11-23
Tip: `d28e099` (`integration/pairing-ux`), the merge point for tickets 01–04.
Branch under test: `ticket/05-closeout` (reset to `d28e099`, no product changes).

Status vocabulary follows `docs/agents/evidence-policy.md`. Nothing in this file is a
real-device PASS; the real device is `NOT RUN` throughout.

## Verification snapshot

| # | Command | Status | Result |
|---|---|---|---|
| 1 | `cargo test -p v2-transport -p v2-runtime -p as-cmd` | PASS | 355 passed, 0 failed, 1 ignored (35 test-result groups). `pairing_live`: 8/8 passed, no hang. |
| 2 | `cargo test -p audio-share-server-gui` | PASS | 114 passed, 0 failed, 1 ignored. |
| 3 | `cargo fmt --check` | PASS | exit 0, no output. |
| 4 | `cd gui && npm ci && npm run check && npm run build` | PASS | `svelte-check`: 101 files, 0 errors, 0 warnings. `vite build`: 140 modules transformed, built in 642 ms. |
| 5 | `cd android-client-v2 && ANDROID_HOME="$HOME/Android/Sdk" ./gradlew testReleaseUnitTest -PnoNative=true "-Porg.gradle.java.installations.paths=/home/w0n9/.cache/audio-share-refactor-worktrees/toolchain/jdk-21.0.13+9"` | PASS | 325 tests, 0 failures, 0 errors, 0 skipped; `BUILD SUCCESSFUL in 25s`. |

Notes on the snapshot:

- The ignored Rust test is `pipewire_list_endpoints_and_live_replay` (needs real
  headless PipeWire/wireplumber daemons), not a product failure.
- The ignored GUI test is `update::tests::fetches_the_real_latest_release_over_https`
  (needs network access to `api.github.com`), not a product failure.
- A first combined Rust run exited 0 but was piped through `tail`; its counts were
  discarded and the command was rerun under `pipefail` with a full log. Only the rerun
  is recorded above.
- `pairing_live` (the known flake) passed 8/8 with no timeout in both runs. The flake
  reported by the ticket 01 implementer is **not reproduced** here; that is not proof it
  is fixed.
- The JVM count was aggregated from the JUnit XML reports produced by this run. A later
  `./gradlew clean assembleRelease ...` for the emulator removed those reports, so the
  count above is the authoritative record of this snapshot.
- No `CARGO_TARGET_DIR` was set for any command.

## Per-ticket merge SHAs

| Ticket | Merge/tip SHA | Subject |
|---|---|---|
| 01 Server Core | `34f3857` | Merge ticket 01 (`64e6d97` explicit Restore + delete) |
| 02 Server GUI | `d28e099` | Tip after ticket 02 (`3e6f050`, `cba8a91`, `f64294c`, `d28e099`) |
| 03 Android repair state | `afa4942` | Merge ticket 03 (`4e48ad8` … `0c7cd69`) |
| 04 Android repair flow | `fe38989` | Tip of ticket 04 (`b57d1c9`, `fe38989`) |

## Emulator evidence

Emulator identity, as required by the evidence policy:

- AVD: `emulator:audio-share-api35`
- Android 15, API 35, ABI `x86_64`; emulator 37.2.13
- App: `dev.audioshare.v2` 0.6.0 (release, debug-signed test build)
- Server: `as-cmd` 0.6.0 built from this tip (`d28e099`)
- Build flags: `-PandroidAbi=x86_64 -PtestSupport=true -PdeviceTests=true`
  (`clean assembleRelease assembleReleaseAndroidTest`)
- Real device: **`NOT RUN`**

| Case | Status | What was observed |
|---|---|---|
| `keystore` | EMULATOR | `KEYSTORE_OK hardware_backed=false` (emulator software-backed Keystore). |
| `pairSas` (first pairing) | FAIL (emulator, test assertion) | Pairing completed on the Server: `Paired: sdk_gphone64_x86_64 (b5d79992f385aa4e)`; the phone's stored credential then authenticated (`reload`). The instrumentation still failed: `pairing did not reach COMPLETE; last=<none>`. Observed twice, deterministically. |
| `reload` (after first pairing) | EMULATOR | `RELOAD_OK pin=347c0b05562ace06`. |
| `revoked` | EMULATOR | `ERROR_OK code=204 name=AUTHORIZATION_DENIED`, surfaced close + typed guidance. |
| SAS re-pair after revoke (Server/protocol observation) | EMULATOR | Server printed `This identity was revoked; accepting restores the saved pairing ... (no new key)` and required `Restore <name>? [y/N]`. After `y`: `Paired` and `clients list` showed the **same** `client_id` `b5d79992f385aa4e` and the **same** `created_at` `2026-11-23T08:34:05Z`, state `active`, one record. `reload` then returned `RELOAD_OK`. |
| SAS re-pair contract case (`awaitPairingPhase("COMPLETE")`) | FAIL (emulator, test assertion) | Same `COMPLETE` assertion failure as the first pairing; the pairing and Restore themselves succeeded as recorded above. |
| SAS re-pair with `--accept` and no stdin (negative path) | FAIL (emulator, case result; decline is by design) | The Server asked `Restore ...? [y/N]`, read EOF, and printed `Declined; nothing was stored.` The phone received code 201. Note the decline is the intended explicit-Restore behaviour; the instrumentation case treats any non-`COMPLETE` result as a failure. No `clients list` was taken after this attempt, so the record's state after the decline was not re-read. |
| `pairQr` / QR re-pair | NOT RUN | Not exercised on the emulator. |
| Forget this Server (UI) | NOT RUN | No automated UI path; local credential/key deletion is covered only by JVM tests. |
| Manual UI checklist (first pairing → revoke → QR re-pair → revoke → SAS re-pair → Forget) | NOT RUN | No automated UI/instrumentation path exists for Re-pair, the Restore dialog, or Forget. The existing cases cover Keystore, SAS/QR pairing, stored connect, and typed errors only. |

Interpretation of the `COMPLETE` assertion failure:

- The pairing exchange publishes `COMPLETE`, then the Service hands off to an
  authenticated session. `SessionOrchestrator.startSession()` calls
  `pairing.beginSession()`, which clears the previous snapshot to `null`
  (`SessionOrchestrator.kt:81-89`, `Pairing.kt` `beginSession()`), so a polling test
  cannot reliably observe `COMPLETE`.
- `git diff v0.6.0..d28e099` over `SessionOrchestrator.kt` and `Pairing.kt` shows that
  this `startSession()` → `pairing.beginSession()` clearing path is unchanged since
  v0.6.0; only the outcome sink and credential fields changed. The device test
  (`971a66c`, `12f3e9e`) also predates the feature. The mismatch is therefore unrelated
  to the pairing-ux changes, but it was **not** re-run against a v0.6.0 build. It does
  not indicate that pairing or Restore failed: both were independently confirmed by
  `reload` and by `clients list`.

## `as-cmd clients delete` manual smoke (host)

Run against a temporary config (`AUDIO_SHARE_CONFIG=/tmp/closeout-emu/config.ini`) after
stopping the Server:

| Step | Command | Status | Output |
|---|---|---|---|
| delete active | `as-cmd clients delete b5d79992f385aa4e` | PASS | `client "b5d79992f385aa4e" is active; revoke it before deleting` (exit 1) |
| revoke | `as-cmd clients revoke b5d79992f385aa4e` | PASS | `revoked client ...` |
| delete revoked | `as-cmd clients delete b5d79992f385aa4e` | PASS | `deleted client ...; this identity is refused at authentication` |
| list | `as-cmd clients list` | PASS | `no paired clients` |

## Known gaps / review items

- **Same-address cleanup deletes only the credential, not the identity key.** A
  successful pairing removes other records for the same address via
  `SessionOrchestrator.removeStaleRecordsForAddress`, which calls
  `credentials.delete(...)` only (`SessionOrchestrator.kt:272-290`). The old record's
  Keystore/software identity key is left behind. `Forget Server` does delete the key
  (`ForgetServer.kt:25-27`), so this is a cleanup-only leak.
- **`pairingIntent` is not persisted across Activity rotation.** `UiStatePersistence`
  saves only screen, address, and SAS input (`UiStatePersistence.kt:18-35`). A rotation
  during `PairingIntent.RePair` restores `PairingIntent.First` and loses the re-pair
  context/copy.
- **SESSION "Connect again" is a silent no-op for a needs-repairing record.** The button
  is rendered for `EndReason.UserStop` (`AudioShareApp.kt:918-925`); `connect()` returns
  an unchanged state when the selected record has `needsRepairing` (`AppState.kt:560-566`),
  so the tap does nothing and gives no feedback. The surfaced 204 card does route to
  Re-pair correctly; this is the plain user-stop path.
- **`CredentialRecord.isSameAddressAs` matches `addressHint` as well as `host:port`.**
  `SessionOrchestrator.kt:292-294` treats either a matching `host:port` or a matching
  `address_hint` string as the same address. A stale record whose hint was derived from a
  different host spelling can therefore be deleted by the next pairing.
- **`as-cmd clients delete` has store/admin tests plus this manual smoke, but no
  dedicated CLI test.** `crates/v2-runtime/tests/admin.rs:321-342` covers active refusal,
  revoked deletion, and post-delete authentication failure; `crates/as-cmd/tests/cli.rs`
  covers only `clients list`/`clients revoke`.
- **`pairing_live` flake (reported by the ticket 01 implementer) was not reproduced.**
  Root cause remains unproven; keep the 10-minute timeout and per-binary rerun advice.
- **Svelte/store logic has no test runner.** `gui/package.json` defines only `dev`,
  `build`, `check`, and `tauri`. Store and dialog behaviour is covered by
  `svelte-check` plus the production build, not by unit tests.
- **Device-test `COMPLETE` assertion mismatch.** See the emulator section: the
  instrumentation's `awaitPairingPhase("COMPLETE")` races the session handoff that clears
  the pairing snapshot. Pairing/Restore themselves were confirmed independently; the test
  expectation should be revisited before it is used as a gate.

## Files changed by this ticket

Only `.scratch/pairing-ux/**` (this evidence file, ticket comments, and `spec.md`
status/acceptance wording). No product code, no ticket `Status:` line.
