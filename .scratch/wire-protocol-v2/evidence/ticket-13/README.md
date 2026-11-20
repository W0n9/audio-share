# Ticket 13 — Server PCM media path: RED → GREEN evidence

Worktree: `.claude/worktrees/m4-server-pcm`, branch `m4-server-pcm`, base `a5de2d9`.

Run date: 2026-11-14 (host tests only; no device work in this ticket).

## Real failing assertions before the implementation

| Command | Result |
|---|---|
| `cargo test -p v2-transport --test media_packetizer` | `error[E0432]: unresolved import v2_transport::media` — no media module existed (`packetizer-red.log`) |
| `cargo test -p v2-transport --test media_queue` | `error[E0432]: no MediaSendQueue / PacingViolation in media` (`queue-red.log`) |
| `cargo test -p v2-transport --test state_machine_playing` | 19 compile errors: no `ServerState::Playing`, no `StreamLifecycle`, no `last_playback_diagnostics` (`state-machine-red.log`) |

## GREEN

| Command | Result |
|---|---|
| `cargo test -p v2-transport --test media_packetizer` | 9 passed (`packetizer-green.log`) |
| `cargo test -p v2-transport --test media_queue` | 8 passed (`queue-green.log`) |
| `cargo test -p v2-transport --lib media_pump` | 9 passed (unit tests inside `src/media_pump.rs`) |
| `cargo test -p v2-transport --test state_machine_playing` | 14 passed (`state-machine-green.log`) |
| `cargo test -p audio-share-core --test capture_media_tap` | 9 passed (`tap-green.log`) |
| `cargo test -p as-cmd --test v2_media` | 2 passed — live QUIC: `Subscribe` → `StreamStarted` → real DATAGRAMs → `Unsubscribe` (`StreamEnded` first) (`live-media-green.log`) |

## Notable findings

- **`StreamEnded` must not clear the subscription** (this ticket's Android-visible
  contract): the baseline Client machine cleared `subscribed` on `StreamEnded`,
  which made a capture restart unreachable. Spec §5.2 keeps the subscription;
  only a user `Unsubscribe` clears it. Fixed in the Android native crate under
  ticket 14 (`stream_ended_retains_subscription_for_the_next_instance`).
- **First media datagram anchor**: a Subscriber that joins a stream already
  running receives `StreamStarted.initial_timestamp = next_frame` (here 448),
  not `0`. `0` is only for a fresh instance (spec §4.4). The live test asserts
  the first datagram's timestamp is at or after the announced anchor.
- **Media generation watch initial-value trap**: `tokio::sync::watch` reports
  `has_changed()` on a fresh receiver; treating that as a generation change made
  the media task call `set_generation(1)` on generation 1 and close the session
  with 602. Fixed by marking the initial value seen at task start and by using
  `send_if_modified` so identical generations never notify (§7.2).
- **Per-send sizing**: the pump keeps unencoded PCM and packets it at
  `max_datagram_size() − 24` inside the send path, so a path-limit reduction
  applies to the very next datagram (§7.3/D2) — no encoded datagram can become
  too large.
- **Capture failure vs end**: the v1 `broadcast_loop` now reports why it ended
  (shutdown → 502, capture channel closed → 501) to the media tap; ADR-0003's
  "no automatic recovery" is unchanged.

## Not covered / explicitly out of scope

- Jitter buffer, concealment, drift correction, impairment matrix, fuzz — M5.
- Multi-Client isolation/soak, production pairing, GUI state — M6.
- Windows capture, API 26 device, 60-minute audio quality — M7.
- The `--fake-capture` pacing remainder noted in the roadmap (+14 %) is not
  re-measured here; the pump's pacing is verified on the media clock with the
  synthetic capture only.
