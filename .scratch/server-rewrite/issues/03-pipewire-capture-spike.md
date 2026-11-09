# 03: Linux PipeWire capture spike

**What to build:** 一个最小 Rust 工具，在 Linux 上枚举 Audio/Sink、按 `object.serial` 与 `priority.session` 选择 Endpoint、采集 F32LE 和至少一种非 F32 格式，并验证 monitor 采集；记录测量结果。决定 `pipewire` crate 是否可直接采用；不达标则验证 `pipewire-sys` direct C API fallback。

**Blocked by:** 01 (New Rust workspace + Wire Protocol harness + v0.3.4 golden fixtures)

**Status:** resolved

- [x] 枚举 Audio/Sink，默认按 `priority.session` 选择。
- [x] 能按 target 采集指定 sink 的 monitor 音频。
- [x] F32LE 与至少一种非 F32（s16、s24 或 s32）可采集并落盘。
- [x] 记录 callback/quantum、CPU 与短时 glitch，并给出 `pipewire` 与 `pipewire-sys` 的结论。
- [x] 反复启动/停止不崩溃。

## Comments

2026-11-09, branch `ticket/03-pipewire-spike`.

### Deliverables
- `tools/pipewire-spike` workspace member: `list`, `capture`, `measure`; pure logic
  (encoding/block_align, default selection, stats, Goertzel) in `src/lib.rs` with
  17 unit tests; Linux PipeWire client in `src/linux.rs`; non-Linux error stub.
- `scripts/pw-test-session.sh`: private headless PipeWire + wireplumber + null
  sink driver. Fixed the malformed `audio.position=FL,FR` (now `[FL,FR]`) and
  added `priority.session` so default selection is testable.
- `tools/pipewire-spike/tests/headless_pipewire.rs`: ignored end-to-end test
  that runs one headless session and verifies list, F32LE/S16LE/S24LE capture
  with a 440 Hz tone (Goertzel), and measure/cycles.
- `docs/server-rewrite/spikes/pipewire.md`: API decision, commands, measured
  numbers, Go/No-Go, caveats.
- `.github/workflows/server-rewrite.yml`: add `libclang-dev` and `pkg-config`
  (the safe `pipewire` crate builds `pipewire-sys` bindings with bindgen).

### API decision
Use the safe `pipewire` 0.10.1 crate with feature `v1_0_0`; no `pipewire-sys`
fallback needed. It provides registry enumeration/default data, monitor capture
via `target.object` + `STREAM_CAPTURE_SINK`, all required formats, negotiated
format read-back, lock-free RT handoff, and clean stop/repeat. `pipewire-sys`
would add unsafe vtable/hook/pod/buffer management for no requirement gain.

### Commands and real results (this host: Ubuntu 24.04.5, Rust 1.98.1, libpipewire 1.0.5, wireplumber 0.4.17)
- `scripts/pw-test-session.sh --tone-wav /tmp/as-tone-03.wav -- ./target/release/pipewire-spike list`
  -> `* serial=37 priority.session=1000 name=TestSink ... display="TestSink"`;
  `total: 1; default: serial=37 (TestSink)`. Serial varies per session.
- F32LE capture (48 kHz, 2 ch, 3 s): negotiated `F32LE rate=48000 channels=2
  block_align=8`, 1,146,880 bytes / 143,360 frames, 70 callbacks. Goertzel on
  the PCM: 440 Hz amplitude 0.2464, 1000 Hz 0.0001.
- S16LE capture (2 s): `S16LE 48000/2 block_align=4`, 385,024 bytes / 96,256
  frames; Goertzel 440 Hz 0.2446, 1000 Hz 0.00001.
- Packed S24LE capture (2 s): `S24LE 48000/2 block_align=6`, 577,536 bytes /
  96,256 frames; Goertzel 440 Hz 0.2446, 1000 Hz 0.00004.
  (S24 is 3 bytes/sample; frames = bytes/6.)
- `measure --seconds 10` (release, F32LE 48 kHz 2 ch): 234 callbacks,
  3,833,856 bytes / 479,232 frames; callback interval min 42.410 / mean 42.666 /
  p50 42.666 / p95 42.787 / max 42.892 ms; quantum 2048 frames (min=mean=max);
  zero_size=0 null_buffer=0 corrupted=0 discont=0 empty=0 ring_overflow=0;
  CPU user 0.040 s + sys 0.042 s = 0.082 s over 10.030 s wall = 0.82%;
  `cycles: 10/10 ok` (0.25 s each, 6 callbacks / 98,304 bytes per cycle).
  Debug build same test: 235 callbacks, 3,850,240 bytes, 3.28% CPU, one
  startup `discont=1`, 10/10 cycles ok.
- `cargo test -p pipewire-spike`: 17 unit tests passed; headless integration
  test passed with `--ignored` (1 passed, 10.11 s).
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D
  warnings`, `cargo test --workspace`: all pass on Linux.
- `cargo check` and `cargo clippy --all-targets` for
  `x86_64-pc-windows-msvc -p pipewire-spike`: pass (stub compiles; not run).

### Caveats
- Only the `support.null-audio-sink` virtual sink was exercised; real ALSA/USB
  monitor ports and hotplug were not. No 8-hour soak is claimed.
- CPU/glitch numbers are from a 10 s run; `measure` reports SPA chunk flags
  (corrupted/discont/empty) plus zero-size/null-buffer/ring-overflow counts.
- The Windows stub is compile-checked only; no Windows runtime here.
- Go for ticket 06, with registry `global_remove`/watchdog for target loss
  (the stream does not report a vanished target promptly).
