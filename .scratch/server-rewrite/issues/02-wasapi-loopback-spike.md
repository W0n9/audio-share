# 02: Windows WASAPI loopback spike

**What to build:** 一个最小 Rust 工具，在 Windows 上从指定 render Endpoint 以 WASAPI shared-mode loopback 采集系统音频，支持 f32、s8、s16、s24、s32，记录 callback 间隔、CPU 与短时 glitch，并与 1 秒缓冲行为对齐。产出测量报告，决定 `wasapi` crate 是否可直接采用；不达标则验证 `windows-rs` fallback。

**Blocked by:** 01 (New Rust workspace + Wire Protocol harness + v0.3.4 golden fixtures)

**Status:** resolved

> Implemented and **executed on Windows 11 on 2026-11-10** (Realtek render
> endpoint). Enumeration, all five encodings, closest-match fallback,
> 10-minute steady state, and 200 start/stop cycles pass. Mono at the mix rate
> is rejected by this driver at `Initialize` and now returns a readable error;
> the old v0.3.4 CLI also cannot capture mono on this endpoint.

- [x] 能枚举 render Endpoint，显示 IMMDevice ID 与友好名。Windows 11 `list`：1 个活动 render endpoint（Realtek），默认项带 `*`。
- [x] 能采集默认 Endpoint 的系统音频并落盘为 PCM。5 s 采集 240,000 帧 / 1,920,000 字节，f32 2ch 48 kHz，440 Hz 幅度 0.13758，RMS −20.24 dBFS。
- [x] 5 种 encoding 至少各验证一次；不支持的请求能回退到 closest match。f32/s8/s16/s24/s32 全部 `S_OK` 且对齐；s16/44.1 kHz 走 `S_FALSE` 回退到 f32 48 kHz。mono 在 `Initialize` 被驱动拒绝，已改为可读错误。
- [x] 记录 callback 间隔、CPU 与短时 glitch，并给出 `wasapi` 与 `windows-rs` 的结论。600 s：CPU 0.72%，packet 平均 10.000 ms/p99 13.206 ms，0 timestamp error，3 次 index gap（0.3/min）。
- [x] 1 秒缓冲下反复启动/停止不崩溃。`start-stop --iterations 200` 200/200，进程存活；`measure --cycles 10` 10/10，句柄 162/162。

## Comments

### 2026-11-09 — implementation on `ticket/02-wasapi-spike`

Commit: `fbc6de2` (implementation + docs). Files:

- `tools/wasapi-spike/` (`Cargo.toml`, `src/main.rs`, `src/logic.rs`,
  `src/win.rs`); added to root `Cargo.toml`; `Cargo.lock` updated.
- `docs/server-rewrite/spikes/wasapi.md` — purpose, commands, API findings,
  validation results, measurement table marked NOT EXECUTED, GO recommendation.

Commands and results (Ubuntu 24.04, Rust 1.98.1; no Windows runtime/linker):

```
cargo fmt --all -- --check                                          pass
cargo build -p wasapi-spike                                         pass
./target/debug/wasapi-spike list                                    exit 1, "wasapi-spike: Windows only"
cargo test -p wasapi-spike                                          26 passed
cargo test --workspace                                              57 passed, 0 failed
cargo clippy --workspace --all-targets -- -D warnings               pass
cargo check --target x86_64-pc-windows-msvc -p wasapi-spike         pass
cargo clippy --target x86_64-pc-windows-msvc -p wasapi-spike \
    --all-targets -- -D warnings                                    pass
cargo check --workspace --all-targets --target x86_64-pc-windows-msvc  pass
```

Implemented: `list`; `capture --endpoint/--encoding/--channels/--sample-rate/
--seconds/--out` (polling shared-mode loopback, 1 s buffer, device-period
polling, advertised negotiated `AudioFormat` + `block_align`, raw PCM);
`IsFormatSupported` closest-match fallback with readable unsupported-format
errors; `measure --seconds/--cycles` (packet/poll interval min/avg/max/p99,
process CPU, silent/discontinuity/timestamp-error/index-gap counts, frames,
bytes, start/stop cycles, handle count); `start-stop --iterations`. Pins
`wasapi = "0.27.0"` and `windows = "0.62.2"` under
`[target.'cfg(windows)'.dependencies]`. The render-direction guard prevents
the crate from silently creating a microphone capture.

Runtime measurements: **not executed** on this Linux host. The doc's results
table is marked NOT EXECUTED; no numbers were inferred.

Residual: run the Windows matrix (enumeration, five encodings, closest-match
fallbacks with a known tone, 10-minute steady state, 10/200 start/stop cycles,
A/B vs old `as-cmd`) and fill in the measured table before ticket 05 relies on
the recommendation.

### 2026-11-10 — Windows 11 runtime execution (PASS)

Host: Windows 11 Enterprise 10.0.26300 x64; Rust 1.99.1 MSVC; wasapi 0.27.0;
Realtek(R) Audio render endpoint `{0.0.0.00000000}.{e235fde3-...}`; WebView2
157.0.4341.42 as the 440 Hz `AudioContext` tone source. Release
`wasapi-spike.exe`; raw evidence under
`audio-share-artifacts/spike/ticket02/`.

Executed:

- `list`: one active render endpoint, default marked. A display-audio endpoint
  appeared in an earlier enumeration and later reported `DeviceState=4`
  (not present).
- `capture --seconds 5` for default/f32/s8/s16/s24/s32: all `S_OK` and
  block-aligned; 240,000–240,480 frames; 480,000–1,920,000 bytes; 440 Hz
  amplitude ≈0.13758 in every encoding; RMS ≈−20.24 dBFS; 0 timestamp errors
  and 0 index gaps; one start discontinuity.
- `capture --encoding s16 --sample-rate 44100`: `S_FALSE (closest match used)`
  → f32 2ch 48 kHz block_align 8; capture ran normally.
- `capture --channels 1`: `IsFormatSupported` `S_OK`, then `Initialize`
  `AUDCLNT_E_UNSUPPORTED_FORMAT` (0x88890008). Added
  `map_initialize_error` to both the spike and the production backend; the tool
  now exits 1 with a readable “endpoint rejected the negotiated loopback format
  …” message. The old v0.3.4 `as-cmd --channels 1` also cannot capture on this
  endpoint (it hangs after starting the listener; the client sees size 0).
- `capture --endpoint {bogus}`: exit 1 with the OS-localized invalid-parameter
  error.
- `measure --seconds 600 --cycles 10`: 28,799,040 frames / 230,392,320 bytes
  in 59,998 packets; packet interval avg 10.000 ms (p99 13.206, max 27.611);
  poll avg 3.393 ms (p99 3.933); CPU 4.312 s = 0.72 % of one core;
  silent=0, data_discontinuity=4, timestamp_error=0, index gaps 1,200 frames
  across 3 packets; effective rate 47,998.4 frames/s; 10/10 start-stop;
  handles 162/162. The 3 index-gap events are 0.3/min, below the documented
  1 packet/min NO-GO threshold.
- `start-stop --iterations 200` twice: 200/200 both times, process survived;
  handles 107→164 both times (one-time caches, not per-cycle growth).

Conclusion unchanged: **GO with wasapi 0.27.0**. The `windows-rs` fallback was
not needed. See `docs/server-rewrite/spikes/wasapi.md` for the full table.
