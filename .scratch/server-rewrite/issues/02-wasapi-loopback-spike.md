# 02: Windows WASAPI loopback spike

**What to build:** 一个最小 Rust 工具，在 Windows 上从指定 render Endpoint 以 WASAPI shared-mode loopback 采集系统音频，支持 f32、s8、s16、s24、s32，记录 callback 间隔、CPU 与短时 glitch，并与 1 秒缓冲行为对齐。产出测量报告，决定 `wasapi` crate 是否可直接采用；不达标则验证 `windows-rs` fallback。

**Blocked by:** 01 (New Rust workspace + Wire Protocol harness + v0.3.4 golden fixtures)

**Status:** ready-for-human

> Implementation is complete in `tools/wasapi-spike`. Boxes 1–4 are checked for
> implemented capability. Every runtime check still requires a Windows host; no
> Windows runtime was available on this Linux machine, so no measurements were
> executed or fabricated.

- [x] 能枚举 render Endpoint，显示 IMMDevice ID 与友好名。`list` 已实现；Windows 运行验证待执行。
- [x] 能采集默认 Endpoint 的系统音频并落盘为 PCM。`capture` 已实现；Windows 运行验证待执行。
- [x] 5 种 encoding 至少各验证一次；不支持的请求能回退到 closest match。请求与回退逻辑已实现并有纯逻辑测试；真实 Endpoint 验证待执行。
- [x] 记录 callback 间隔、CPU 与短时 glitch，并给出 `wasapi` 与 `windows-rs` 的结论。`measure`/`start-stop` 已实现统计；结论基于 API 覆盖（GO with `wasapi` 0.27.0），实测数据待 Windows 执行。
- [ ] 1 秒缓冲下反复启动/停止不崩溃。**NOT EXECUTED**：需要 Windows 硬件运行 `start-stop`/`measure` 才能确认；工具已实现 1 秒缓冲的反复 start/stop 与句柄计数。

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
