# M5-06 §11.2 impairment conformance matrix — results

Date: 2026-11-19
Branch: `ticket/m5-06-impairment-fuzz` (worktree `agent-a5e9f53ec2d935590`), base `integration/m5` @ `c55aad3`.
All execution is HOST. No real network, no device, no emulator.

Layer vocabulary used below:
- **host app layer** = the Android native `MediaSession` / `JitterBuffer` / pure
  `DriftEstimator`/`DriftController`, driven by an injected session clock.
- **in-process server queue/pacer** = `MediaPump` + `MediaSendQueue` + injected
  `Instant` (no I/O).
- **pure state machine** = `v2_transport::state_machine::StateMachine`.
- **real network / device** = NOT RUN everywhere in this ticket.

Every row is an independent test; a partially host-checkable row is split into
PASS (host part) and NOT RUN (real-network/device part).

| # | Profile | Status | Layer exercised | Test (command: `cargo test ...`) | Evidence pointer |
|---|---|---|---|---|---|
| 1a | Loss 2 % — media gaps → concealment; loss accounting matches injection; late_dropped separate and zero | **PASS** | host app layer | `native --test impairment_matrix row1_two_percent_independent_loss_is_concealed_and_accounted` | `client-fuzz-matrix.log`; `impairment_matrix.rs` |
| 1b | Loss 2 % — control channel stays clean over a lossy path | **NOT RUN** | real network | — | No in-process lossy-QUIC seam; DATAGRAM loss cannot reach the reliable control stream by construction, but that is not executed here |
| 2 | Reorder 25 % — 0 loss; reordered counted; late_dropped 0 | **PASS** | host app layer | `native --test impairment_matrix row2_twenty_five_percent_reorder_loses_nothing` | `client-fuzz-matrix.log`; `impairment_matrix.rs` |
| 3 | Reorder 50 % — 0 loss; reordered counted; late_dropped 0 | **PASS** | host app layer | `native --test impairment_matrix row3_fifty_percent_reorder_loses_nothing` | `client-fuzz-matrix.log`; `impairment_matrix.rs` |
| 3b | §8 separation: far-late arrival → `late_dropped`, not network loss | **PASS** | host app layer | `native --test impairment_matrix a_far_late_arrival_is_late_dropped_and_not_network_loss` | `client-fuzz-matrix.log`; `impairment_matrix.rs` |
| 4a | Duplicate 5 % — 0 app-visible (sequence dedup) | **PASS** | host app layer (application-level sequence dedup) | `native --test impairment_matrix row4_five_percent_duplicates_are_invisible_at_the_app_layer` | `client-fuzz-matrix.log`; `impairment_matrix.rs` |
| 4b | Duplicate 5 % — QUIC packet-number dedup in transport | **NOT RUN** | real QUIC transport | — | No host seam observes QUIC packet-number dedup; the app-layer result above is explicitly not a claim about it |
| 5 | Jitter 10 ms ± 5 ms — no deadlock; A.8 estimate tracks | **PASS** | host app layer | `native --test impairment_matrix row5_ten_plus_minus_five_ms_jitter_tracks_and_never_deadlocks` | `client-fuzz-matrix.log`; `impairment_matrix.rs` |
| 6a | Congestion 300 kbit/s, queue 50 — bounded backlog; backlog drains after the simulated link recovers; no new queue drops afterwards; capture never blocked | **PASS** | in-process queue/pacer with injected clock | `cargo test -p v2-transport --test impairment_matrix_server row6_congestion_bounded_backlog_and_full_rate_recovery` | `server-fuzz.log`; `impairment_matrix_server.rs` |
| 6b | Congestion over a real 300 kbit/s path | **NOT RUN** | real network | — | No network impairment appliance/emulator in this context |
| 7a | Path MTU 1280 — datagrams ≤ limit, frame-aligned, no fragmentation reliance (chunker) | **PASS** | in-process packetizer/pump | `cargo test -p v2-transport --test impairment_matrix_server row7_mtu_1280_chunker_fits_the_limit_without_fragmentation` | `server-fuzz.log`; `impairment_matrix_server.rs` |
| 7b | Path MTU 1280 — client parser accepts path-sized datagrams | **PASS** | host app layer | `native --test impairment_matrix row7_mtu_1280_datagrams_parse_and_deliver` | `client-fuzz-matrix.log`; `impairment_matrix.rs` |
| 7c | Path MTU 1280 — DPLPMTUD re-probe over a real path | **NOT RUN** | real QUIC path | — | No DPLPMTUD probing seam on host |
| 8a | Clock drift ±100 ppm — estimate, rate clamp/slew, bounded buffer, counted correction | **PASS** | host pure estimator/controller + simulated buffer | `native --test impairment_matrix row8_hundred_ppm_drift_engages_rate_match_and_stays_bounded` | `client-fuzz-matrix.log`; `impairment_matrix.rs` |
| 8b | Clock drift ±100 ppm — real capture/audio device clocks and `AudioTrack.setPlaybackRate` | **NOT RUN** | real device | — | No device/emulator; M5-04 recorded the same boundary |
| 9a | Idle 25 s — keep-alive/idle-timeout config check (10 s < 30 s; 25 s covered) + `IdleTimeout` event → 601 (injected clock / state machine) | **PASS** | pure state machine + transport constants | `cargo test -p v2-transport --test impairment_matrix_server row9_idle_timeout_is_601_and_keepalive_covers_25_seconds` | `server-fuzz.log`; `impairment_matrix_server.rs` |
| 9b | Idle 25 s over a real network connection | **NOT RUN** | real network | — | No 25 s live session in this context (no sleeping); only the injected event is executed |
| 10a | Connection kill — teardown (no control frame, CLOSING/CLOSED absorbing) | **PASS** | pure state machine | `cargo test -p v2-transport --test impairment_matrix_server row10_connection_kill_tears_the_session_down` | `server-fuzz.log`; `impairment_matrix_server.rs` |
| 10b | Connection kill — client backoff reconnect (§6.3) | **NOT RUN** | Android/Kotlin policy | — | The only implementation is Kotlin `ReconnectPolicy` in `app/src/main/java`, outside this ticket's allowed test paths; live QUIC kill has no stable in-process seam |
| — | **network loss vs `late_dropped` remain separate counters** | **PASS** | host app layer | row 1 (`cumulative_lost == injection`, `late == 0`) and `a_far_late_arrival_...` (`late == 1`, loss returns to 0) | `impairment_matrix.rs` |

## Notes

- No host simulation above is labelled a real-network or real-device PASS.
- "Packet arrived" is never used as playback evidence: all delivery assertions
  go through the §9 diagnostics counters (`cumulative_lost`,
  `cumulative_late_drops`, `cumulative_reordered`, `concealments`,
  `concealed_frames`) or the injected-clock drift components, never through an
  ACK/arrival count.
- Row 8's "buffer bounded" assertion uses a host-simulated buffer model
  (capture adds 100 ms per 100 ms; playback consumes the rate-corrected
  amount). It is explicitly not a device audio-buffer measurement.
