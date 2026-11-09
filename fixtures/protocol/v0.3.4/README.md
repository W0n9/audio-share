# Wire Protocol v0.3.4 golden fixtures

These fixture sets pin the frozen v0.3.4 Wire Protocol used by the released
Android Client and the old C++ Server Core. They are the baseline for parity
tests of the Rust Server rewrite.

## Provenance

| Path | Provenance | What it is |
| --- | --- | --- |
| `linux/f32-48k-stereo/` | `recorded-v0.3.4-linux` | Recorded from the real released `as-cmd` 0.3.4 Linux binary in a headless PipeWire session with a generated sine tone playing into a `TestSink` null sink. |
| `linux/s16-44k1-stereo/` | `recorded-v0.3.4-linux` | Same recording setup, with `as-cmd --encoding=s16 --channels=2 --sample-rate=44100`. |
| `windows/f32-48k-stereo/` | `synthesized-v0.3.4-semantics` | Synthesized from the frozen v0.3.4 encoding. **Not recorded on a Windows machine.** |
| `windows/s16-44k1-stereo/` | `synthesized-v0.3.4-semantics` | Synthesized; not recorded on Windows. |
| `windows/s16-48k-mono/` | `synthesized-v0.3.4-semantics` | Synthesized; not recorded on Windows. |
| `windows/s24-48k-stereo/` | `synthesized-v0.3.4-semantics` | Synthesized; not recorded on Windows. |

The Linux PCM is a live capture snapshot: re-recording produces different PCM
bytes, but the same frame structure and control framing. The Windows PCM is a
deterministic integer pattern (a 32-frame sawtooth) used only to exercise
framing and MTU segmentation. No fixture contains timestamps or machine paths.

## Schema (version 1)

Each set is a directory:

```
manifest.json
audio_format.pb                 # exact AudioFormat protobuf payload
control/get_format.request.bin  # 4-byte GET_FORMAT command
control/get_format.response.bin # u32 cmd + u32 size + protobuf
control/start_play.request.bin  # 4-byte START_PLAY command
control/start_play.response.bin # u32 cmd + i32 id
control/heartbeat.server.bin    # 4-byte HEARTBEAT command
control/heartbeat.client.bin    # 4-byte HEARTBEAT response
control/udp_bind.bin            # i32 id, little-endian
pcm/stream.pcm                  # concatenated captured UDP payloads
pcm/segments/NNNN.bin           # individual UDP datagrams in capture order
```

`manifest.json` records:

- `schema_version`, `protocol_version` (`v0.3.4`), `provenance`, `platform`,
  `source`, and free-form `notes`.
- `audio_format`: encoding value and protobuf name, channels, sample rate,
  derived `block_align`, and the exact protobuf bytes (hex + SHA-256).
- `mtu`: `1492` MTU, 20-byte IP header, 8-byte UDP header, 1464-byte maximum
  payload, and the value aligned down to `block_align`.
- `control`: every control payload file, its length, hex, SHA-256, plus
  `start_play_response.id`, heartbeat `count`, and `udp_bind.id`.
- `pcm`: stream file, total bytes and hash, segment count/lengths, and each
  segment's file, offset, length, and SHA-256.
- `capture`: how many heartbeats and UDP datagrams were captured.

## Regenerating

Linux, from the real old `as-cmd` (requires PipeWire, WirePlumber,
`dbus-run-session`, `pw-play`, and no sudo):

```bash
AS_CMD_034=/path/to/as-cmd-0.3.4-linux scripts/record-fixtures.sh
```

The script creates a disposable `XDG_RUNTIME_DIR`, starts PipeWire and
WirePlumber there, creates a `TestSink` `Audio/Sink` null sink, loops a
generated sine WAV into it, and drives `as-cmd` with
`protocol-harness record`. It kills every child process and removes its temp
directory on exit, including on error.

Windows cases are synthesized deterministically:

```bash
cargo run -p protocol-harness -- synthesize --out fixtures/protocol/v0.3.4/windows
```

## Replaying

The canonical command that validates the committed fixtures offline:

```bash
cargo run -p protocol-harness -- replay --fixture fixtures/protocol/v0.3.4/linux/f32-48k-stereo
```

Add `--connect 127.0.0.1:PORT` or `--server-cmd ...` to drive a live server and
compare framing; see `docs/server-rewrite/harness.md`.
