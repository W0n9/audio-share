# 15: v0.3.4 Android Client acceptance + soak

**What to build:** 用真实 v0.3.4 APK 对完整新 Server 做验收：5 种 encoding、单/双声道、44.1/48 kHz、多客户端、重连、heartbeat 超时；8 小时 soak、设备热插拔、睡眠恢复、反复 start/stop；完成 GUI 手工验收清单。

**Blocked by:** 07 (as-cmd parity), 11 (Windows maintenance), 12 (Linux equivalents), 13 (Windows packaging), 14 (Linux packaging)

**Status:** ready-for-human

- [ ] 真实 v0.3.4 APK 在全部组合下无感播放，无解析错位。仅在 Android 真机/模拟器上执行；本环境无手机。见 `docs/server-rewrite/android-v034-acceptance.md`（NOT EXECUTED）。
- [x] 多客户端、重连与 heartbeat 超时行为与 v0.3.4 一致。脚本化 Wire Protocol 矩阵已执行（3 客户端、4 次重连/5 个 START_PLAY id、heartbeat 主动 8 s 存活、静默客户端 ~6001 ms 关闭、SIGINT 后 154 ms 关闭会话）；真实 APK 仍待人工。
- [ ] 8 小时 soak 无 crash、无明显 glitch 倒退。已执行 12 分钟 release soak（PASS）；8 小时运行 **NOT EXECUTED**，见 `docs/server-rewrite/soak.md`。
- [ ] 设备热插拔与睡眠恢复后能恢复采集。headless 热插拔 + 操作员 Stop/Start 已验证；当前 core 没有自动重启 supervisor，睡眠/恢复未执行。见 `docs/server-rewrite/hotplug-sleep.md`。
- [ ] GUI 手工验收清单全部通过。Xvfb/DBus/截图可执行部分已标记；真实桌面托盘、Windows 运行时和手机项未执行，见 `docs/server-rewrite/gui-acceptance-checklist.md`。
- [x] 记录 A/B 结果与已知差异。见 `docs/server-rewrite/ab-linux.md`（旧 s8 退出 1；其余 encoding 的格式/分片一致，PCM 不声明逐字节相同）。

Additional executed assets:

- [x] `scripts/acceptance-matrix.sh`：20/20 encoding×channels×rate 组合 + 6/6 场景通过。
- [x] `scripts/soak.sh`：可配置时长/release/日志轮转/崩溃检测；12 分钟 release run 通过。
- [x] `scripts/hotplug-recovery.sh`：目标 sink 销毁/重建 + 操作员恢复通过。
- [x] `protocol-harness probe` + `probe_logic`/`probe_cli` tests（8 个测试）。

## Comments

2026-11-09 — branch `ticket/15-acceptance-soak`; integration base `b456c10`
(ticket 14 Linux packaging merge). Implementation commits: `17f200f`
(probe + tests), `8688f6a` (scripts), `3493590` (soak rotation fix),
`f3afe06` (acceptance docs), `f32369e` (this soak record + checklist
update).

Environment: Ubuntu 24.04.5 x86_64, Rust 1.98.1, libpipewire 1.0.5,
WirePlumber 0.4.17, headless `scripts/pw-test-session.sh` null `Audio/Sink`
with a 440 Hz tone, release binaries built from this tree (no Rust runtime
changes after the integration merge). Real hardware, Windows, and an Android
phone are not available here.

Commands/results:

- `cargo test -p protocol-harness --test probe_logic --test probe_cli`:
  8 tests pass (PCM decode for all 5 encodings, Goertzel 440 Hz at 44.1/48 kHz,
  datagram MTU/alignment, tone analysis, live fake-server format/heartbeat/
  timeout/reconnect).
- `scripts/acceptance-matrix.sh --skip-build --out /tmp/t15-matrix-release
  --ab-old .../bin/as-cmd-0.3.4-linux`: 20/20 matrix pass; actual negotiated
  format equals requested in every row (f32/s8/s16/s24/s32 × 1/2 ch ×
  44100/48000; block_align 1/2/3/4/6/8; max datagram 1464; every datagram
  block-aligned; 440 Hz tone present; one heartbeat per 4 s probe).
  Scenarios: 3 simultaneous clients (2556 datagrams total), 4 reconnects with
  ids [1,2,3,4,5], active client 2 heartbeats 3000 ms apart / 0 misses,
  silent client closed at 6001 ms, SIGINT closed the session in 154 ms and the
  server exited 0, 3 start/stop cycles all exit 0. Raw evidence in
  `/tmp/t15-matrix-release/` (`matrix.tsv`, `scenarios.tsv`, `ab.tsv`,
  `cases/*.json`); recorded in `docs/server-rewrite/acceptance-matrix.md`.
- A/B: old 0.3.4 and new Server on the same null sink/tone. f32/s16/s24/s32
  have identical `GET_FORMAT`/`AudioFormat`, block_align, 1464-byte max
  datagram, 440 Hz tone and near-identical datagram distribution; PCM is not
  byte-identical and is not claimed to be. Old s8 exits 1 (U8 defect); new s8
  streams with block_align=2. See `docs/server-rewrite/ab-linux.md`.
- `scripts/soak.sh --skip-build --duration 12m --clients 2
  --reconnect-every 240 --sample-seconds 5 --out /tmp/t15-soak-release2`:
  PASS — actual 722 s, 2/2 clients ok, server exit 0, `client_process_failure:
  false`, RSS min/mean/max 11524/15558.8/15612 KiB, CPU mean/max 3.304/28.5 %, 548,339,712
  PCM bytes, 401,616 datagrams, 6 sessions / 4 reconnects, 470 heartbeats /
  0 misses, gap buckets 50/100/250 ms = 53/0/0, max gap 65 ms, tone present
  for both clients. An earlier run with the same workload
  (`/tmp/t15-soak-release`) reported `client_process_failure: true` due to a
  monitor race at the duration boundary; `soak.sh` now checks the duration
  before client liveness and the rerun records false (both probes were
  `ok: true`). Log rotation was exercised separately with
  `--duration 10s --clients 1 --rotate-mb 0` (18 rotated files, PASS). The
  default 8-hour run is NOT EXECUTED (`docs/server-rewrite/soak.md`).
- `scripts/hotplug-recovery.sh --skip-build --out /tmp/t15-hotplug-release`:
  PASS — sink serial 37 destroyed while capturing (registry id 36), recreated
  as serial 63; the server stayed alive with zero PCM; no automatic recovery;
  operator Stop/Start on the recreated sink restored the tone; server exit 0.
  See `docs/server-rewrite/hotplug-sleep.md`.
- `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --
  -D warnings`, `cargo test --workspace`: pass after the integration merge.

Residual release gates (Status: ready-for-human):

1. Real v0.3.4 APK: full 20-combination matrix, multi-client, reconnect,
   heartbeat, 60-minute playback, evidence package
   (`docs/server-rewrite/android-v034-acceptance.md`). NOT EXECUTED — no
   phone/emulator.
2. 8-hour soak (`scripts/soak.sh --duration 8h`); only 12 minutes executed.
3. Sleep/resume and real-hardware hotplug (USB/HDMI/default-sink change).
   Automatic capture recovery is not implemented; operator Stop/Start only.
4. Real desktop GUI: GNOME AppIndicator/KDE/XFCE tray right-click, Windows
   tray left-click/menu/toast, Wayland; Windows WASAPI A/B, firewall/UAC,
   WebView2; re-run the Linux deb/AppImage packaging checks if artifacts
   change.
5. The 3-hour update-check interval was not observed in real time (startup
   check and interval loop are covered by tests).
