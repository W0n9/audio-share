# Phase 3 report — Rust ↔ Android authenticated control channel cross-validation

Ticket: `09-phase3-cross-validation`
Branch: `ticket/09-phase3-cross-validation`
Base commit: `9c706dd` (integration tip at time of work; `phase-2/wire-protocol-v2` was still at `9c706dd`)
Date: 2026-11-12
Result: **one FAIL (authorization revocation on a live session)**; everything else PASS or NOT RUN with a stated reason. See "Adjudication needed".

No media or statistics capability is claimed here. Phase 3 is the control channel only (READY steady state); media/statistics remain phase 4.

## Environment / versions

| Item | Value |
|---|---|
| Server binary | `target/release/as-cmd`, 6,593,768 bytes, sha256 `a7cbb375a0abd5d090405d9014ea2200e91d93c574d933e88d4d6c227ac4e696` |
| Server args | `as-cmd -b 0.0.0.0:65530 --v2-port 65531 --fake-capture -V`, config `AUDIO_SHARE_CONFIG=/tmp/ticket09/data/config.ini` |
| Server identity | freshly generated (fixture server key is PKCS#8 v1, which ring 0.17 rejects); fingerprint `EE5BAE52 0923AB39 71B6A68D C0FD774F A9B27DC3 F1EAC5FB 576985E5 E9F79C94` |
| Fixture client identity | client_id `cf7940b474310246`, compressed pubkey base64url `A9ClKd50dWQawFDHkrQs_weRB8pTENp5U9JZUSFHfA9t` (native key hardcoded from `fixtures/wire-protocol-v2/test-keys.json`) |
| Client APK | `android-client-v2/app/build/outputs/apk/release/app-release.apk`, 2,467,726 bytes, sha256 `59b4b97a9e5d51ec3062569de19cd99e88e5cdf22c6136ebbadb461749b64bf6` |
| Native `.so` | unstripped 8,883,672 bytes; in-APK arm64 stripped 2,355,888 bytes |
| Device | Xiaomi M2007J22C (cannon), Android 12 / API 31, arm64, MIUI, no SIM |
| Device path | adb over tailscale, serial `100.117.189.121:5555`, host `100.94.170.124` |
| Build | `android-client-v2`: `ANDROID_HOME=$HOME/Android/Sdk ./gradlew assembleRelease` |
| Evidence root | `.scratch/wire-protocol-v2/evidence/ticket-09/` |

### Temporary bypass disclosure (must be re-validated)

`android-client-v2/native/src/session.rs:89-95`: `.worker_threads(2)` → `.worker_threads(1)`.

This is a **temporary ticket-09 bypass** for a known client defect (auth signing context lives in `thread_local!`; a multi-worker runtime can resume the task on another worker, where `SERVER_PUBLIC_KEY` is `None`, producing a 601 close and a permanently AUTHENTICATING UI). The real fix is owned by `review/wire-protocol-v2-fixes`. The APK used for all device evidence contains this bypass. Positive E2E was blocked without it; no other source changes were made.

## Item-by-item results

| # | Requirement | Result | Evidence / note |
|---|---|---|---|
| 1 | E2E positive: connect → AuthInit/Challenge/Proof/Result → Negotiate/NegotiateResponse → READY → user stop | **PASS** (with bypass) | `01-e2e-positive/ready-2026-11-12T23-01.png`, `server-ready.log` (`v2 connection accepted` + `control stream open`), `client-ready.log`; host live-QUIC `auth_happy_path_reaches_ready_over_quic` PASS |
| 2 | User-initiated stop with normal close code | **PASS (Close 701)**, trigger caveat | No stop button in the smoke UI. Activity recreation (`adb shell settings put system font_scale 1.15` → `onDestroy` → `session.stop()`) produced server `ClientClosed(ErrorCode { code: 701 })` for session 45908 at `15:00:51.685` and client `session stopped (701 CLIENT_STOPPING)`. BACK under MIUI only backgrounded the activity (`finishing=false`); force-stop is an abrupt loss, not a clean stop. Evidence: `01-e2e-positive/server-after-stop.log`, `client-after-stop.log`, `user-stop-701-*.log` |
| 3 | Wrong SPKI pin: handshake-level rejection, no application data | **PASS** | Client: `server SPKI does not match pinned identity`; no server `v2 connection accepted` for those attempts; UI IDLE with retry loop (known client reconnect defect). `05-negatives/wrong-spki-pin.png`, `wrong-spki-pin-client.log` |
| 4a | Wrong signature `auth_context` | **PASS (host)** / NOT RUN on device | `crates/v2-transport/tests/state_machine.rs:247 invalid_signature_is_203_and_counts_a_failure`; cannot forge a bad signature from the smoke UI |
| 4b | Unknown public-key identity → 202 | **PASS** | Device: server rejects (control stream open, then auth failure; client closes and reconnects). Server driver log **hardcodes `code=203`** for every auth failure (`crates/v2-transport/src/driver.rs:314-317`), so the exact 202 is proven by `state_machine.rs:213 valid_proof_unknown_identity_is_202_connection_stays` and replay fixture `control/error-202-authorization-unknown.json`, not by the device log. `05-negatives/unknown-identity-202.png`, `unknown-identity-202-client.log` |
| 5a | Revocation of a live session: terminate ≤5 s | **FAIL** | READY at 22:48; fixture record removed from `/tmp/ticket09/data/pairing/clients.json` at 22:48:15; still READY at 22:48:29 (12 s) and the server never ended the session. Root cause: `AuthorizationStore` loads `clients.json` once (`crates/v2-transport/src/auth_store.rs`), there is no reload and no production revoke surface in `as-cmd`; the driver's 2 s revocation poll only tears down `Revoked`, not `Unknown` (deleted record). Violates `pairing.md` §9.2. Evidence: `05-negatives/revocation-live-before.png`, `revocation-live-after-12s.png` |
| 5b | New connection after revocation → 204 | **PASS (restart path)** | Server restarted with the record `state:"revoked"`; device rejection observed in ~100 ms. Exact 204 asserted by live-QUIC `session_driver.rs:454 revoked_identity_is_torn_down_with_204`; device log again shows the hardcoded 203 message. `05-negatives/revoked-new-connection.{png,client.log,server.log}` |
| 6 | Stray frame before READY → decidable 401/402 | **PASS (host)** / NOT RUN on device | `session_driver.rs:377 second_control_stream_is_invalid_state_402`, `state_machine.rs:292 auth_proof_without_challenge_is_402_fatal`; no injection surface in the smoke UI. `05-negatives/host-live-quic-tests.log` |
| 7 | Malformed frame → 401 + Close | **PASS (host)** / NOT RUN on device | `session_driver.rs:428 empty_length_prefix_is_malformed_401`, `state_machine.rs:190 connecting_malformed_frame_is_fatal_401`; no injection surface on device |
| 8 | TM-5 rate limit, per connection 5 failures | **PASS (host)** / NOT RUN on device | `session_driver.rs:557 per_connection_and_per_source_auth_failure_limits`; the smoke client closes after one 202/204, so it cannot accumulate 5 failures on one connection |
| 9 | TM-5 rate limit, per source 20 failures / 60 s | **PASS (device)** | 24 rapid force-stop/start launches with a revoked store produced 20 auth failures in ~15 s, then `source auth-failure budget reached ... code=302` and subsequent `budget exhausted ... code=302`. `05-negatives/tm5-rate-limit-server.log`, `tm5-rate-limited.png`, `tm5-rate-limit-client.log` |
| 10 | READY idle >60 s, no false death | **PASS** | READY 22:39:55 → 22:41:16 (>80 s), server session still open; `02-idle/ready-after-70s.png` |
| 11 | Lock/unlock survival | **PASS** | Screen off 20 s (22:42:06–22:42:33), wake + dismiss keyguard → READY, server session alive; `03-lock/screen-off-20s.png`, `03-lock/ready-after-unlock.png` |
| 12 | Network loss → bounded backoff reconnect | **PASS** | On-device `svc wifi disable; sleep 15; svc wifi enable` (rc 0/0, 22:50:22→22:50:37). Client lost the connection, 5 s handshake timeout → 601, retries at 548/990/1706 ms (bounded, jittered), then QUIC established → READY after Wi-Fi returned; server accepted 59735, old session ended 303 (supersede). Client base delay is 500 ms, below the spec's 1 s (known reconnect defect). `04-network/*` |
| 13 | Resource baseline (device CPU/RSS + server session count) | **PASS** | Device PID 15201: 8.6% CPU (`dumpsys cpuinfo`; 5.9 user + 2.6 kernel), VmHWM 170,580 kB, VmRSS 166,048 kB, 38 threads. Later PID 18220: 4% CPU, VmHWM 149,172 kB, VmRSS 147,832 kB. Server RSS 9,620 kB; 6 accepted / 7 ended log lines, 1 active session inferred (no metrics endpoint). Prototype desktop baseline for comparison: server RSS 8.4 MiB / client 7.5 MiB, CPU ~4.7% / ~6.2%, but that run carried media and the desktop client was native-only. `06-resources/resource-baseline.log`, `ready-session.png` |
| 14 | `protocol-harness replay` all green | **PASS** | Six fixture sets, exit=0 each: linux/f32-48k-stereo, linux/s16-44k1-stereo, windows/f32-48k-stereo, windows/s16-44k1-stereo, windows/s16-48k-mono, windows/s24-48k-stereo; `07-v1-regression/replay-*.log` |
| 15 | v1 integration tests green | **PASS** | `cargo test --workspace`: 435 passed, 0 failed, 3 ignored, 37 suites. First run had one environmental failure (`bind 65531: AddrInUse`) because the live v2 server held the port; rerun after stopping the server was clean. `07-v1-regression/cargo-test-workspace.log` |
| 16 | v0.3.4 `android-app/` build unaffected | **PASS** | Zero diff (`git diff 9c706dd -- android-app/` = 0 lines, 0 files). `sh gradlew assembleDebug` BUILD SUCCESSFUL / exit 0 after supplying a temporary local, gitignored `android-app/keystore.properties` pointing at the debug keystore (removed afterwards; not part of the change). `07-v1-regression/android-app-assembleDebug.log` |
| 17 | Server runs v1+v2 concurrently, isolated | **PASS** | Live v1 replay via 65530 while the v2 device session was READY: `replay: ok (live)`, 1770 datagrams / 2,416,640 bytes; v2 stayed READY before/after. `07-v1-regression/live-v1-replay-while-v2-ready.log`, `v2-ready-{before,after}-v1-replay.png`, `v2-server-log-concurrent.log` |
| 18 | Rust ↔ Android replay fixtures on real builds | **PASS** | Rust: `cargo test -p v2-transport --test state_machine_replay --test golden_vectors` → 3 + 19 = 22 passed. Android native: `cd android-client-v2/native && cargo test` → 83 passed, 0 failed (fixture replay happy/error/limits/pairing). `08-replay/server-replay-tests.log`, `08-replay/android-native-cargo-test.log` |
| 19 | Phase-3 report with PASS/FAIL/NOT RUN, versions, links, leftovers | **PASS** | this file |

## Host live-QUIC negative suite

`cargo test -p v2-transport --test session_driver -- --nocapture` → 8 passed, 0 failed:

- `auth_happy_path_reaches_ready_over_quic`
- `per_connection_and_per_source_auth_failure_limits`
- `auth_deadline_closes_with_601`
- `empty_length_prefix_is_malformed_401`
- `version_mismatch_is_101`
- `second_control_stream_is_invalid_state_402`
- `new_session_supersedes_the_old_with_303`
- `revoked_identity_is_torn_down_with_204`

Log: `05-negatives/host-live-quic-tests.log`.

Device-level evidence for the auth/negotiate frame sequence is indirect: `as-cmd` and the Android client do not log individual control frames. The READY steady state, absence of auth failures on the server, and the host live-QUIC `auth_happy_path_reaches_ready_over_quic` test (plus the fixture replay suites) together cover the Connect → Auth → Negotiate → READY chain.

## Adjudication needed (blocking resolution)

1. **Live revocation ≤5 s is FAIL.** Implement an authorization-state reload/revoke path in the server (reload the persistent store on the revocation poll and treat a previously authorized but now unknown/removed identity as revoked, and/or expose a production revoke surface — the 07 ticket has no runtime revoke API, and direct `clients.json` edits cannot be seen by a running server). Then re-run the device test and confirm the READY session ends within 5 s, plus the 204 pair. Candidate touch points: `crates/v2-transport/src/auth_store.rs` (reload, IO serialization), `crates/v2-transport/src/driver.rs` (poll tick), `crates/v2-transport/src/state_machine.rs:712` (`handle_revocation_check` currently fatal only on `Revoked`). This is not one of the six known review-branch defects, so it needs an explicit owner.
2. **Device log masks the registry code.** `crates/v2-transport/src/driver.rs:314-317` logs `code = 203u32` for every authentication failure. 202/204 device evidence therefore relies on host tests + replay fixtures. Logging the real registry code would make future device acceptance direct.
3. **Known client defects observed but not fixed here** (owned by `review/wire-protocol-v2-fixes`): reconnect policy (§6.3: 500 ms base, 202/203/204/205/303/701 all auto-reconnect, close code unread), Close echo violation, pending request-id/Status handling, and the thread-local auth context. The positive E2E bypass (`worker_threads(1)`) must be reverted and the device E2E re-run after the fix branch merges.
4. Ticket status stays `claimed` because of item 5a; it is not resolved.

## Leftovers / NOT RUN summary

- `android-app/` debug build required a temporary local `keystore.properties` (gitignored) for signing config; the file was removed after the successful build, and there is no source diff.
- Device-level stray-frame injection, malformed-frame injection, wrong-signature auth proof, and per-connection 5-failure exhaustion: NOT RUN (smoke UI has no frame injection). Covered by host live-QUIC/state-machine tests instead.
- Server session count has no public metric; inferred from acceptance/termination log lines.
- Media and statistics capabilities were not tested or claimed (phase 4).
