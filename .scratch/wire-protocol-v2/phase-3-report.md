# Phase 3 report — Rust ↔ Android authenticated control channel cross-validation

Ticket: `09-phase3-cross-validation`
Branch: `ticket/09-phase3-cross-validation`
Base commit: `8c219b0` (integration tip after merging `review/wire-protocol-v2-fixes`; work started from `9c706dd`)
Branch head at report time: `7a42315` (plus the docs/evidence commit that contains this file)
Date: 2026-11-12 (first round) / 2026-11-13 (post-fix rerun)
Result: **all ticket-09 items PASS, or NOT RUN on device with host live-QUIC/state-machine coverage stated inline.** No outstanding FAIL.

No media or statistics capability is claimed here. Phase 3 is the control channel only (READY steady state); media/statistics remain phase 4.

## Environment / versions (final, post-fix rerun)

| Item | Value |
|---|---|
| Integration base | `8c219b0` (contains the review fix branch; thread-local auth context fix, store reload, real close codes) |
| Ticket-branch fix | `7a42315 fix(v2-client): surface remote Close code to reconnect policy` |
| Server binary | `target/release/as-cmd`, 6,637,752 bytes, sha256 `0fd79b00ca8cc53d8f08ade3ec6ebd7dd7655d56df35c043847a5d0dec6e542d` |
| Server args | `as-cmd -b 0.0.0.0:65530 --v2-port 65531 --fake-capture -V`, config `AUDIO_SHARE_CONFIG=/tmp/ticket09/data/config.ini` |
| Server identity | freshly generated (fixture server key is PKCS#8 v1, which ring 0.17 rejects); fingerprint `EE5BAE52 0923AB39 71B6A68D C0FD774F A9B27DC3 F1EAC5FB 576985E5 E9F79C94` |
| Fixture client identity | client_id `cf7940b474310246`, compressed pubkey base64url `A9ClKd50dWQawFDHkrQs_weRB8pTENp5U9JZUSFHfA9t` (native key hardcoded from `fixtures/wire-protocol-v2/test-keys.json`) |
| Client APK | `android-client-v2/app/build/outputs/apk/release/app-release.apk`, 2,478,374 bytes, sha256 `7f15a2fc8332c493c85ea21fc20474326b6784ecd1e26b1e6a1b8accf2aad6d6` (built with the review branch plus `7a42315`; no bypass) |
| Native `.so` | unstripped 8,911,744 bytes; in-APK arm64 stripped 2,366,536 bytes |
| Device | Xiaomi M2007J22C (cannon), Android 12 / API 31, arm64, MIUI, no SIM |
| Device path | adb over tailscale, serial `100.117.189.121:5555`, host `100.94.170.124` |
| Build | `android-client-v2`: `ANDROID_HOME=$HOME/Android/Sdk ./gradlew assembleRelease` (gradle passes `--features test-support` to the native crate, per `app/build.gradle.kts`) |
| Evidence root | `.scratch/wire-protocol-v2/evidence/ticket-09/`; post-fix rerun lives in `09-post-fix/` |

## Bypass disclosure and post-fix rerun

The first round (pre-fix, commits `0c503e8`/`4386046`) used a temporary one-line bypass in `android-client-v2/native/src/session.rs`: `.worker_threads(2)` → `.worker_threads(1)`, because the auth signing context was `thread_local!` and could resume on another worker. That bypass was **reverted by the integration merge** (`8c219b0`); the final APK contains the real fix and runs the normal multi-worker runtime. Do not reintroduce `worker_threads(1)`.

Post-fix rerun found one remaining client bug: a remote `Close` reached the native state machine as a terminal stop but the `CloseInfo` was not stored, so Kotlin received `close_code=null` and treated 204 revocation as transport loss (one auto-reconnect). `7a42315` records `CloseInfo` on every terminal close and adds `remote_close_code_is_surfaced`; device evidence below shows `close_code=204` and `close_code=202` with no automatic reconnect.

The pre-fix evidence in `01-e2e-positive/` … `08-replay/` is retained for traceability. Final acceptance evidence is the `09-post-fix/` set.

## Item-by-item results (final)

| # | Requirement | Result | Evidence / note |
|---|---|---|---|
| 1 | E2E positive: connect → AuthInit/Challenge/Proof/Result → Negotiate/NegotiateResponse → READY → user stop | **PASS** (multi-worker, no bypass) | `09-post-fix/ready-multithread.png`, `server-ready-post-fix.log` (`v2 connection accepted`, `control stream open`); host live-QUIC `auth_happy_path_reaches_ready_over_quic` PASS. Frame sequence is not logged by either side; READY + host/replay coverage is the evidence. |
| 2 | User-initiated stop with normal close code | **PASS** (Close 701) | Smoke UI has no stop button. Activity recreation (`settings put system font_scale 1.15` → `onDestroy` → `session.stop()`) produced server `ClientClosed(ErrorCode { code: 701, reason: "client stopping" })` at `16:01:25.419`; `09-post-fix/server-after-stop.log`, `client-after-stop.log`, `after-user-stop.png`. BACK only backgrounds the activity under MIUI; force-stop is an abrupt loss, not a clean stop. |
| 3 | Wrong SPKI pin: handshake-level rejection, no application data | **PASS** | 64-zero pin: `server SPKI does not match pinned identity`; no server acceptance for the pinned attempt; `09-post-fix/wrong-spki-pin-post-fix.png`, `wrong-spki-pin-client.log`. (`deadbeef` is rejected locally as "pin must be 64 hex chars" — also no application data.) |
| 4a | Wrong signature `auth_context` → 203 | **PASS (host)** / NOT RUN on device | `09-post-fix/server-state-machine-codes.log`: `invalid_signature_is_203_and_counts_a_failure` PASS (30/30 state-machine tests). Cannot forge a bad signature from the smoke UI. |
| 4b | Unknown public-key identity → 202 | **PASS** | Device: empty store, server logs `authentication failed ... code=202`; UI `CLOSED`, `session closed (close_code=202); no automatic reconnect`; `09-post-fix/unknown-identity-202-post-fix.png`, `server-unknown-identity-202.log`, `client-unknown-identity-202.log`. A14 (real registry code) confirmed. |
| 5a | Revocation of a live session: terminate ≤5 s | **PASS** | Record deleted at `15:51:38.694Z`; server `Close(204)` at `15:51:39.494` → **0.800 s**. UI `CLOSED`, `close_code=204`, no reconnect. `09-post-fix/revocation-timing.txt`, `revoked-live-6s-post-fix.png`, `server-after-live-revoke-post-fix.log`, `client-after-live-revoke-post-fix.log`. |
| 5b | New connection after revocation → 204 | **PASS** | Store record `state:"revoked"`: server logs `authentication failed ... code=204`; UI `close_code=204`, no reconnect. `09-post-fix/revoked-new-connection-post-fix.png`, `server-revoked-new-connection-post-fix.log`, `client-revoked-new-connection-post-fix.log`. Host: `deleting_the_record_ends_the_live_session_and_rejects_reconnects` + `revoked_identity_is_torn_down_with_204`. |
| 6 | Stray frame before READY → decidable 401/402 | **PASS (host)** / NOT RUN on device | `session_driver.rs` `second_control_stream_is_invalid_state_402`; state-machine 402 tests. No frame-injection surface in the smoke UI. `09-post-fix/host-live-quic-tests.log`. |
| 7 | Malformed frame → 401 + Close | **PASS (host)** / NOT RUN on device | `session_driver.rs` `empty_length_prefix_is_malformed_401`; state-machine 401 tests. No injection surface on device. |
| 8 | TM-5 rate limit, per connection 5 failures | **PASS (host)** / NOT RUN on device | `session_driver.rs` `per_connection_and_per_source_auth_failure_limits`; the smoke client closes after one 202/204, so it cannot accumulate 5 failures on one connection. |
| 9 | TM-5 rate limit, per source 20 failures / 60 s | **PASS (device)** | Pre-fix round: 24 rapid launches with a revoked store produced 20 auth failures in ~15 s, then `source auth-failure budget reached ... code=302` and `budget exhausted ... code=302`; `05-negatives/tm5-rate-limit-server.log`, `tm5-rate-limited.png`, `tm5-rate-limit-client.log`. Guard logic unchanged by the fix branch. |
| 10 | READY idle >60 s, no false death | **PASS** | Post-fix: READY 15:53:28.9 → 15:54:33 (~64 s), active session never ended; six fixture replays ran during the idle window. `09-post-fix/idle-after-70s.png`, `idle-start.png`, `idle-end.txt`, `server-after-idle.log`, `v1-replay/*.log`. |
| 11 | Lock/unlock survival | **PASS** | Screen off 20 s (session 40879 accepted 15:58:42); no new connection after wake, session still live, UI READY. `09-post-fix/lock-before.png`, `ready-after-unlock.png`. Note: `input keyevent 26` toggles power; swipe-up is needed to dismiss the MIUI keyguard. |
| 12 | Network loss → bounded backoff reconnect | **PASS** | On-device `svc wifi disable; sleep 15; svc wifi enable` 15:55:09→15:55:24. Client lost the session, 5 s handshake timeouts, reconnects at 1037/2183/3471/7954 ms (bounded, jittered, 1 s base per §6.3), then QUIC + control stream → READY; old session ended 303 (supersede). `09-post-fix/network-ready-before.png`, `network-ready-after.png`, `server-after-network-loss.log`, `client-after-network-loss.log`. |
| 13 | Resource baseline (device CPU/RSS + server session count) | **PASS** | Idle post-fix device PID 21958: 0% CPU (`dumpsys cpuinfo`), VmHWM 154,932 kB, VmRSS 154,892 kB, 37 threads. Server RSS 9,960 kB; server log 13 accepted / 16 ended lines; no metrics endpoint, active count inferred. `09-post-fix/resource-baseline-post-fix.log`. |
| 14 | `protocol-harness replay` all green | **PASS** | Six fixture sets, exit=0 each: linux/f32-48k-stereo, linux/s16-44k1-stereo, windows/f32-48k-stereo, windows/s16-44k1-stereo, windows/s16-48k-mono, windows/s24-48k-stereo; `09-post-fix/v1-replay/replay-*.log`. |
| 15 | v1 integration tests green | **PASS** | `cargo test --workspace` post-fix: 445 passed, 0 failed, 4 ignored, 39 suites, exit 0. `09-post-fix/cargo-test-workspace.log`. |
| 16 | v0.3.4 `android-app/` build unaffected | **PASS** | Zero diff vs `8c219b0`. Debug build had succeeded earlier on the unchanged tree with a temporary gitignored `android-app/keystore.properties` (removed afterwards); no source change. |
| 17 | Server runs v1+v2 concurrently, isolated | **PASS** | Live v1 replay via 65530 while the v2 device session was READY: `replay: ok (live)`, 1770 datagrams / 2,416,640 bytes, exit 0; v2 stayed READY. `09-post-fix/live-v1-replay-while-v2-ready.log`, `v2-ready-after-v1-live-replay.png`. |
| 18 | Rust ↔ Android replay fixtures on real builds | **PASS** | Rust: `state_machine_replay` 3 + `golden_vectors` 19 = 22 passed; Android native: `cargo test` 97 passed, 0 failed (fixture replay happy/error/limits/pairing). `09-post-fix/server-replay-tests.log`, `android-native-cargo-test.log`. |
| 19 | Phase-3 report with PASS/FAIL/NOT RUN, versions, links, leftovers | **PASS** | this file |

## Host live-QUIC negative suite (post-fix)

`cargo test -p v2-transport --test session_driver -- --nocapture` → 9 passed, 0 failed:

- `auth_happy_path_reaches_ready_over_quic`
- `per_connection_and_per_source_auth_failure_limits`
- `auth_deadline_closes_with_601`
- `empty_length_prefix_is_malformed_401`
- `version_mismatch_is_101`
- `second_control_stream_is_invalid_state_402`
- `new_session_supersedes_the_old_with_303`
- `revoked_identity_is_torn_down_with_204`
- `deleting_the_record_ends_the_live_session_and_rejects_reconnects`

Log: `09-post-fix/host-live-quic-tests.log`. State-machine suite: 30 passed, including 202/203/401/402 cases (`09-post-fix/server-state-machine-codes.log`).

## Defects found during ticket 09 and disposition

1. Thread-local auth context across multi-worker runtime (blocked positive E2E; temporary `worker_threads(1)` bypass used in the pre-fix round). Fixed by the review branch; bypass reverted at `8c219b0`; multi-worker E2E revalidated.
2. Client pending request-id pairing / missing Status treated as success. Fixed by review branch.
3. Client echoed Close frames, violating "Close is one-way and never echoed". Fixed by review branch.
4. Client reconnect policy (500 ms base; 202/203/204/205/303/701 auto-reconnect; close code unread). Fixed by review branch; device shows 1 s base and no reconnect on 202/204.
5. Server Close code 0 not mapped to 401; client deadline/keep-alive semantics; send-path limits. Fixed by review branch; idle/keep-alive revalidated >60 s.
6. Test private key not compile-time gated. Fixed by review branch; APK is built with `test-support` as configured for this smoke app.
7. Authorization store loaded once, no reload, deleted identities not treated as revoked, no production revoke surface (first-round FAIL 5a). Fixed by review branch (A13: reload every 2 s, deleted = revoked); live Close 204 now 0.800 s.
8. Driver auth-failure log hardcoded 203 (A14). Fixed by review branch; device logs now show real 202 and 204.
9. Remote `CloseInfo` not stored, so Kotlin saw `close_code=null` and reconnected once after 204. Found and fixed on this branch in `7a42315` with unit test `remote_close_code_is_surfaced`.

## Leftovers / NOT RUN summary

- Device-level stray-frame injection, malformed-frame injection, wrong-signature proof, and per-connection 5-failure exhaustion: NOT RUN (smoke UI has no injection surface). Covered by host live-QUIC/state-machine tests.
- Per-source TM-5 evidence comes from the pre-fix round; the guard path is unchanged by the fix branch.
- Server session count has no public metric; inferred from acceptance/termination log lines.
- Fixture server PKCS#8 key is v1 and ring 0.17 rejects it (`VersionNotSupported`); the test run used a freshly generated server identity. Regenerate the fixture server key as PKCS#8 v2 (or load with an explicit v1-tolerant path) so the fixture identity is usable as-is.
- `android-app/` debug build needs a local gitignored `keystore.properties`; the app tree is unchanged.
- Media and statistics capabilities were not tested or claimed (phase 4).
- `7a42315` lives on this ticket branch; it should be carried into the integration branch if the ticket branch is not merged as a whole.
