# M5-06 implementer evidence — 2026-11-19

Branch: `ticket/m5-06-impairment-fuzz` (worktree
`agent-a5e9f53ec2d935590`), based on `integration/m5` @ `c55aad3`
(`BASE_OK`). No production code, proto, docs, or defaults changed.

Ticket claim: the `.scratch/.../m5-06-impairment-matrix-fuzz.md` status edit
was **blocked by worktree isolation** (the harness refuses `Edit` on the
shared checkout). Status left as `ready-for-agent`; the main session/merger
must claim it (same limitation M5-04 recorded).

## Files added (tests only)

- `android-client-v2/native/tests/impairment_matrix.rs` — §11.2 rows 1–5, 8,
  and the client half of row 7.
- `android-client-v2/native/tests/parser_fuzz.rs` — §11.3 client parser fuzz
  and negative cases.
- `crates/v2-transport/tests/impairment_matrix_server.rs` — §11.2 rows 6, 7,
  9, 10 (server host halves).
- `crates/v2-transport/tests/parser_fuzz_server.rs` — §11.3 server parser fuzz
  and §6 registry mappings.

No dependency was added. Evidence lives in this directory (untracked from the
repo; relocated later by the main session).

## TDD status

The `tdd` skill was consulted, but there was **no RED → GREEN slice**: every
behaviour under test (jitter buffer, late drops/concealment, drift estimator/
controller, server pump/pacer/queue, state machine, both parsers) already
exists on `integration/m5`. These are **characterization/conformance tests**
against public seams, with expected values taken from the spec (§6, §7.5, §8,
§11.2, §11.3) and from the injected impairment parameters — not recomputed from
implementation internals — so they are not tautological. No production code
was changed. The only failures during development were mistakes in the new
tests themselves (a shadowed helper name, a nested mutable borrow, and an
incorrect expectation about the client decoder's buffered prefix), fixed
before the final green runs.

## Commands and exact results (all run from the worktree)

1. `cargo test -p v2-transport` → **PASS**, 289 passed, 0 failed, 0 ignored
   across the library plus 19 integration bins. New bins:
   `impairment_matrix_server` 4 passed, `parser_fuzz_server` 9 passed.
   Log: `v2-transport.log`.
2. `cargo test -p audio-share-core` → **PASS**, 118 passed, 0 failed, 1 ignored
   (the ignored test is pre-existing, in one of the capture/wire bins — not
   touched by this ticket). Log: `audio-share-core.log`.
3. `cargo test --manifest-path android-client-v2/native/Cargo.toml` →
   **PASS**, 210 passed, 0 failed, 0 ignored across 9 bins
   (142 lib + 17 `clock_drift_contract` + 7 `clock_drift_session_contract` +
   8 `impairment_matrix` + 10 `jitter_buffer_contract` + 2
   `media_control_closing` + 5 `media_control_contract` + 12
   `media_session_contract` + 7 `parser_fuzz`). Log: `native.log`.
4. Same manifest `--features test-support` → **PASS**, 210 passed, 0 failed,
   0 ignored (same breakdown). Log: `native-test-support.log`.
5. Seed/row-level runs with `--nocapture`:
   - `cargo test --manifest-path android-client-v2/native/Cargo.toml --test
     parser_fuzz -- --nocapture` and `... --test impairment_matrix --
     --nocapture` → **PASS** (7 + 8). Seeds printed:
     `5563352944484679681` (fuzz), `5563352942048509953` (matrix).
     Log: `client-fuzz-matrix.log`.
   - `cargo test -p v2-transport --test parser_fuzz_server -- --nocapture`
     and `... --test impairment_matrix_server -- --nocapture` → **PASS**
     (9 + 4). Seed `5563352942038286337`. Log: `server-fuzz.log`.
6. `rustfmt --edition 2021` on the four new test files — clean.

## Matrix summary

- **PASS (host):** rows 1 (media half), 2, 3, 3b (loss/late separation), 4
  (app-layer dedup), 5, 6 (in-process queue/pacer), 7 (chunker + client
  parser), 8 (pure estimator/controller + simulated buffer), 9 (state machine
  + transport constants), 10 (teardown).
- **NOT RUN:** row 1 control-clean over a real lossy path; row 4 QUIC
  packet-number dedup; row 6 real 300 kbit/s path; row 7 DPLPMTUD re-probe;
  row 8 real device clocks/`AudioTrack`; row 9 real 25 s connection; row 10
  client backoff reconnect.
- Full row-by-row table: `matrix.md`.

## Fuzz summary

70 000 seeded cases total (server 40 000 / client 30 000), no panic, no
non-determinism, all control failures §6 401, all state-machine codes in the
§6 registry, no request silently ignored. Details and seeds: `fuzz.md`.

## Separation requirement (§8)

`cumulative_lost` (network loss) and `cumulative_late_drops` are asserted
separately in every relevant test. `a_far_late_arrival_is_late_dropped_and_not_network_loss`
proves the two move independently: the late arrival increments `late_dropped`
while the earlier gap term returns to zero, and delivered frames do not change.
No assertion treats "packet arrived" as playback evidence; all playback
evidence is the §9 diagnostics fields or the injected-clock drift components.

## Residual risks / coverage boundaries

- The congestion row is an application-layer queue/pacer simulation with an
  injected clock, not a kernel/QUIC path; "capture never blocked" is evidenced
  by `MediaPump::accept` being non-I/O and always returning, not by wall-clock
  measurement.
- The drift row's "buffer bounded" is a host-simulated buffer model, not a
  device audio buffer.
- The client `on_datagram` fuzz keeps generation 1, so its histogram reports
  `stale=0`; stale generation is covered explicitly in the negative tests.
- Real network, real device, DPLPMTUD, and QUIC packet-number dedup remain
  NOT RUN and are not claimed anywhere.
