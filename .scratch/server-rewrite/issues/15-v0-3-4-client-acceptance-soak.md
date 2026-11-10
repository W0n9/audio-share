# 15: v0.3.4 Android Client acceptance + soak

**What to build:** 用真实 v0.3.4 APK 对完整新 Server 做验收：5 种 encoding、单/双声道、44.1/48 kHz、多客户端、重连、heartbeat 超时；8 小时 soak、设备热插拔、睡眠恢复、反复 start/stop；完成 GUI 手工验收清单。

**Blocked by:** 07 (as-cmd parity), 11 (Windows maintenance), 12 (Linux equivalents), 13 (Windows packaging), 14 (Linux packaging)

**Status:** ready-for-human

- [ ] 真实 v0.3.4 APK 在全部组合下无感播放，无解析错位。仅在 Android 真机/模拟器上执行；本环境无手机。见 `docs/server-rewrite/android-v034-acceptance.md`（NOT EXECUTED）。
- [x] 多客户端、重连与 heartbeat 超时行为与 v0.3.4 一致。脚本化 Wire Protocol 矩阵已执行（3 客户端、4 次重连/5 个 START_PLAY id、heartbeat 主动 8 s 存活、静默客户端 ~6001 ms 关闭、SIGINT 后 154 ms 关闭会话）；真实 APK 仍待人工。
- [x] 8 小时 soak 无 crash、无明显 glitch 倒退。已执行 8 小时 release soak（PASS：0 crash、0 heartbeat miss、无 >100 ms gap、max gap 66 ms），见 `docs/server-rewrite/soak.md`。
- [ ] 设备热插拔与睡眠恢复后能恢复采集。headless 热插拔 + 操作员 Stop/Start 已验证；当前 core 没有自动重启 supervisor，睡眠/恢复未执行。见 `docs/server-rewrite/hotplug-sleep.md`。
- [ ] GUI 手工验收清单全部通过。Xvfb/DBus/截图可执行部分已标记；真实桌面托盘、Windows 运行时和手机项未执行，见 `docs/server-rewrite/gui-acceptance-checklist.md`。
- [x] 记录 A/B 结果与已知差异。见 `docs/server-rewrite/ab-linux.md`（旧 s8 退出 1；其余 encoding 的格式/分片一致，PCM 不声明逐字节相同；fix pass 增加了 30 s 的 CPU、PCM 吞吐/漂移和 datagram 到达间隔代理测量）。

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

Final integration merge: `df6ae72` (integration tip `470a95d`, ticket 16
big-bang cutover). That merge deleted the legacy `server-core`/`server-mfc`
C++ trees and updated docs/packaging/CI; it did not change `crates/`,
`tools/`, or `scripts/`, so the matrix/soak binaries and results are
unaffected. `cargo fmt --check`, clippy, and the full workspace tests were
re-run after this merge and pass. A later merge `1afcd36` (integration tip
`3e8074e`, the merger's ticket-15 branch merge) brought only the
`docs/server-rewrite/README.md` index update; fmt/clippy/workspace tests were
re-run again and pass.

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
  `--duration 10s --clients 1 --rotate-mb 0` (18 rotated files, PASS).
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
2. Sleep/resume and real-hardware hotplug (USB/HDMI/default-sink change).
   Automatic capture recovery is not implemented; operator Stop/Start only.
3. Real desktop GUI: GNOME AppIndicator/KDE/XFCE tray right-click, Windows
   tray left-click/menu/toast, Wayland; Windows WASAPI A/B, firewall/UAC,
   WebView2; re-run the Linux deb/AppImage packaging checks if artifacts
   change.
4. The 3-hour update-check interval was not observed in real time. The
   fix-pass restored v0.3.4 timing (first automatic check after one full
   3-hour interval, no startup check) and covers the scheduling decision with
   a unit test; the interval itself remains a real-time manual check.

2026-11-10 — 8-hour release soak PASS.

Command: `scripts/soak.sh --duration 8h --clients 2 --reconnect-every 1800
--sample-seconds 5 --out /home/w0n9/workspaces/audio-share-artifacts/soak-8h`
on the Linux host with release binaries built from
`integration/server-rewrite` (Rust 1.98.1, PipeWire 1.0.5, WirePlumber 0.4.17,
headless null sink with a looped 440 Hz tone).

- Duration 28,803 s (8 h 0 m 3 s); 2/2 clients ok; server exit 0, no crash.
- RSS min / mean / max 11,500 / 15,601.5 / 15,620 KiB over 5,578 samples; CPU
  mean 3.651 %, max 28.5 % (startup sample).
- 22,083,813,376 PCM bytes, 16,174,668 datagrams, 32 sessions, 30 intentional
  reconnects.
- 19,140 heartbeats, 0 misses; datagram gaps >50 / >100 / >250 ms =
  1,336 / 0 / 0; max gap 66 ms (12-minute run: 65 ms).
- Both clients: `alignment_ok: true`, `max_datagram_ok: true`, 440 Hz tone
  detected (amplitude ≈ 0.249), empty `errors`; throughput ratio min 0.9984.
- Evidence: `/home/w0n9/workspaces/audio-share-artifacts/soak-8h/`
  (`soak-summary.json`, `monitor.csv`, `client-{0,1}.json`, `server.log`,
  `server-stream.log`, `tone-48000.wav`).

See `docs/server-rewrite/soak.md`. This closes the soak gate. The real v0.3.4
APK, sleep/resume, real-hardware hotplug, and real desktop GUI gates remain;
`Status` stays `ready-for-human`.

### 2026-11-10 — Windows 11 GUI/runtime evidence

Host: Windows 11 Enterprise 10.0.26300 x64; WebView2 157.0.4341.42; MSVC
14.44; Rust 1.99.1. CI artifacts from run 42710876060 plus a locally built exe
with two runtime fixes. Full table in `docs/server-rewrite/build-windows.md`;
checklist updates in `docs/server-rewrite/gui-acceptance-checklist.md`.

Windows items now observed:

- Window/tray: portable exe, 420×640, tray icon, left-click toggle, native
  menu Show/Start/Stop/Exit (menu actions triggered and observed), close-to-tray
  and close-to-exit, `--hide`, single instance (including a `--hide` second
  instance), WhenAppStart 0/1/2.
- Settings/config: endpoint/encoding/host/port persistence, v0.3.4
  `config.ini` migration 0→2 with notice, Reset, autostart HKCU entry
  (`"<exe>" /hide`, spaces quoted), `server.log`.
- Server: real WASAPI Start/Stop, `protocol-harness probe` 10 s (1,998
  datagrams, 0 heartbeat misses, 440 Hz tone, alignment/MTU OK), Stop closes
  the client session, WLAN `10.8.144.155:65530` client path.
- Windows maintenance: firewall UAC approve/cancel and Sound Panel (ticket 11).
- i18n en-US/zh-CN, About 0.4.0/homepage/license, homepage opens the browser,
  manual update check over HTTPS.

Two Windows-only defects were found and fixed (update-check TLS backend, WASAPI
`Initialize` error mapping); the follow-up CI run 42766244106 (`ebdd468`) is
green on all four jobs and its artifacts include those two fixes, smoke-tested
on Windows 11. A third defect (UDP broadcast stall on a slow Wi-Fi peer) was
found during the real-phone soak and is fixed in a later commit; a fresh CI run
is required to include it in the release artifact.

Still NOT EXECUTED: real v0.3.4 APK matrix/multi-client/60-minute playback,
real phone firewall connectivity, sleep/resume, real-hardware hotplug, Windows
10, missing-WebView2 clean VM, and the real-time 3-hour update interval. The
Linux real desktop tray (GNOME/KDE/XFCE) also remains.

### 2026-11-10 — real v0.3.4 APK on Android 12 (partial)

Phone: Xiaomi M2007J22C, Android 12 / API 31, arm64; released v0.3.4 APK
(`versionCode 3004`, SHA-256 `77cc93d3…a471b4`). The phone connected through
the Windows host's Mobile Hotspot (`5277`, PC `192.168.137.1`, phone
`192.168.137.212`). Server: CI `as-cmd.exe` from run 42766244106 (`ebdd468`)
for 44.1 kHz/60-minute cases and the fixed local GUI for the 48 kHz matrix.

- **48 kHz stereo**: f32/s8/s16/s24/s32 all played on the phone; `dumpsys
  media.audio_flinger` showed the app's active AudioTrack format 5/2/1/6/3
  respectively at 48 kHz stereo, and every `protocol-harness probe` passed
  (440 Hz, aligned, MTU OK). Evidence:
  `audio-share-artifacts/android/{enc}-48k-probe.json` and
  `{enc}-48k-audioflinger.txt`.
- **44.1 kHz requested**: all five encodings negotiated f32 48 kHz (`S_FALSE`
  closest match) and played; probes passed. The requested rate is not
  available on this Realtek shared-mode endpoint.
- **Mono**: 10 combinations N/A — the driver rejects mono at `Initialize`
  (`AUDCLNT_E_UNSUPPORTED_FORMAT`, readable error); the old v0.3.4 server also
  cannot capture mono on this endpoint.
- **Reconnect / heartbeat**: the phone auto-reconnected after every server
  Stop/Start and created a new AudioTrack. With Wi-Fi disabled for 10 s the
  app showed “Network is unreachable, 会在2秒内重试”; re-enabling rejoined the
  hotspot and resumed playback. The server-side ~6 s heartbeat close is
  covered by the scripted Linux matrix.
- **Multi-client**: one phone plus a concurrent `protocol-harness probe`
  (second Wire Protocol client) ran in every matrix row; a two-Android-client
  run was not available.
- **60-minute playback**: **PASS** on the fixed build. 12 five-minute probes
  (plus a final probe at 15:45): every probe 1,994–2,000 datagrams/6 s, 440 Hz,
  max gap 27 ms, 0 heartbeat misses; phone AudioTrack active at every sample,
  one auto-reconnect (track 129→131), no app FATAL/exception, 682 MB received.
  Evidence: `android/soak-60m-fixed/`.

The first 60-minute attempt (pre-fix `as-cmd`) stalled after ~5 minutes: the
phone showed “已开始” with 0.1 K/s, the next probe received 0 datagrams, but
`GET_FORMAT`/`START_PLAY`/heartbeats still worked and a separate
`wasapi-spike capture` could still capture the tone. Root cause: the broadcast
loop awaited `udp.send_to`; a Wi-Fi peer that stopped draining filled the socket
send buffer, blocking the broadcast loop and then the capture thread's
`blocking_send`. Fix: use `try_send_to` and drop on `WouldBlock` with a
one-time warning, so a slow peer cannot stall capture or other clients.
Post-fix, a 60 s phone Wi-Fi-off plus a non-reading UDP peer still let a PC
probe receive 1,198 datagrams/440 Hz per 6 s; the fixed 60-minute soak is
running as the regression test. Evidence: `android/soak-60m/` (pre-fix stall)
and `android/soak-60m-fixed/` (post-fix).

The first checkbox stays unchecked because mono is N/A on this host and a
two-Android-client run is still missing; the 60-minute and reconnect/heartbeat
items are closed.
