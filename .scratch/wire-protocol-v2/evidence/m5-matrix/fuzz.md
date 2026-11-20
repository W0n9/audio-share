# M5-06 §11.3 parser fuzz — results

Date: 2026-11-19. Host only. Seeded splitmix64 inline (no cargo-fuzz, no
nightly). Every run replays from its printed seed; `M5_FUZZ_SEED` overrides it.
Logs: `server-fuzz.log`, `client-fuzz-matrix.log`.

## Seeds and case counts

| Side | Target (test name) | Cases | Seed (decimal / hex) | Result |
|---|---|---|---|---|
| Server | `framing::decode_body` — `fuzz_decode_body_never_panics_and_maps_to_401` | 10 000 | 5563352942038286337 / `0x4D3500065E510001` | **PASS** — ok=1379, empty=77, malformed=8514, unset=30; every failure maps to §6 **401** |
| Server | `framing::FrameReader::push` — `fuzz_frame_reader_never_panics_and_maps_to_401` | 10 000 | same | **PASS** — frames=1266, framing_errors=8734; all errors §6 **401**; no panic. The prefix-cap rejection (0 / 65537) is pinned by the explicit negative tests; the fuzz run does not measure buffered size. |
| Server | 24-byte media header `media::MediaHeader::parse` — `fuzz_server_media_header_never_panics_and_is_deterministic` | 10 000 | same | **PASS** — truncated=829, zero_field=0, parsed=9171; only `MisalignedPcm`/`ZeroField` |
| Server | `StateMachine::handle` on mutated valid requests — `fuzz_state_machine_requests_are_deterministic_and_never_silently_ignored` | 10 000 | same | **PASS** — decoded=1332, rejected=8668; all outcome codes in §6 registry; no request silently ignored |
| Client | `media::MediaHeader::parse` — `fuzz_media_header_parser_never_panics_and_is_deterministic` | 10 000 | 5563352944484679681 / `0x4D350006F0220001` | **PASS** — truncated=860, impossible=0, parsed=9140 |
| Client | `MediaSession::on_datagram` (raw + structured hostile datagrams) — `fuzz_on_datagram_never_panics_and_is_deterministic` | 10 000 | same | **PASS** — delivered=1605, duplicate=0, malformed=8121, stale=0, held=266, late=8, ignored=0; counters never decrease; rejects never write PCM |
| Client | `FrameDecoder` + `ClientMachine` on raw/mutated control frames — `fuzz_control_frames_and_client_machine_are_deterministic` | 10 000 | same | **PASS** — decode_errors=9061, decoded_frames=939, fatal=286, stops=93 |

Total: 70 000 seeded cases (40 000 server, 30 000 client).

## Rules asserted

- **No panic / abort.** All cases complete. Inputs are bounded by construction
  (≤ 4 KiB schema bodies, ≤ 70 KiB stream chunks, ≤ 301-byte datagrams or
  ≤ 1 KiB structured payloads), and the prefix-cap rejection that prevents a
  large body allocation is pinned by the explicit 0/65537 negative tests. The
  fuzz run itself does not measure an allocation ceiling.
- **Deterministic.** Each target executes the same seed twice and compares its
  full summary/signature sequence; mismatches fail the test.
- **Deterministic §6 codes.**
  - Every control schema/framing failure is `FrameError::error_code() == 401`
    (`decode_body`, `FrameReader`).
  - State-machine outcomes only ever carry codes from `ErrorCode::ALL`.
  - Explicit negative tests pin: version conflict → **101**; capability
    list conflict → **401**; state violation (`Subscribe` in CONNECTING) →
    **402**; invalid auth proof → **203**; zero/oversized frame length →
    **401**; unset oneof → **401**.
- **Media has no on-wire §6 code.** Rejection is drop + count: `Malformed`,
  `Duplicate`, `StaleGeneration`, `HeldUnknownConfig`, `LateDropped` are
  deterministic outcomes and the corresponding counters move; nothing is
  silently dropped. Unknown generation → bounded hold; after 2 s `tick`
  reports give-up (the session's 403 class).
- **Unknown fields/messages.** A proto3 unknown field alongside a known body is
  ignored (evolution rule); an unknown-field-only body leaves the oneof unset
  and is **401**, never a silent ignore.

## Known coverage boundary

- `on_datagram` fuzz keeps the session generation at 1, so `stale=0` in the
  histogram; the stale-generation path is covered explicitly by
  `negative_unknown_and_stale_generations_state_violations`.
- Transport-level concerns (QUIC packet-number dedup, real datagram-size
  limits, lossy paths) are outside these parsers and are recorded NOT RUN in
  `matrix.md`.
