# M5-09：Linux host 漂移窗口证据

## 历史观察与适用范围

B2 脱敏汇总（`client-B2.log` 离群附近约一分钟）：相邻约 5 s 快照内 hard re-anchor 增加 7 次、删除 64,672 帧；补静音增加 91 次、43,680 帧；原始漂移先 −38,082 ppm、后 +336,157 ppm（窗口分别 10,078/10,075 ms，各 100 点）。同轮 Server `media.stats` pending 0 / ≤66 ms，零丢弃。日志没有逐报告的 `(now_ms, capture_frames, playback_frames)`、逐包到达时间或逐次 trim/fill/AudioTrack 接受量，不能原样回放或确定现场根因。

## RED → GREEN

从本工单基线 `96e5f0a` 开始，仅修改测试时运行：

```sh
cargo test --manifest-path android-client-v2/native/Cargo.toml --features test-support --test clock_drift_session_contract -- --test-threads=1
```

- **FAIL**：`catch_up_delivery_does_not_publish_a_playback_clock_outlier` 经真实 `MediaSession` DATAGRAM、播放报告、trim 入账路径。合成双 48 kHz，仅把连续 PCM 的交付在 5 s 内暂存约 251 ms 再批量送达 7 次，触发 7 次 hard trim、合计 64,656 帧；峰值 `+185488 ppm`（10,078 ms / 100 点）。正常不暂存的对照 **PASS**：0 ppm、无 hard trim。同样是满窗离群，不是 B2 现场回放。
- **PASS**：同一命令经修复后，`catch_up_delivery_does_not_publish_a_playback_clock_outlier`、`short_delivery_batches_do_not_starve_the_drift_window`、`one_stalled_delivery_invalidates_old_drift_and_recovers_real_nonzero_drift`、`accepted_silence_does_not_turn_a_capture_stall_into_clock_drift` 均输出 `ok`。已知位置步进的软 240 帧、硬 4,800 帧和接受静音用例仍通过；稳定注入 ±10,000 ppm 仍可原样测出，施加的修正分别为 ±100 ppm。合成读端不是 Kotlin 实现；仍记录 7 次 hard trim，未改变生产触发策略。

未保留的临时探针仅作探索观察，不是可复跑的验收结果：等时钟缩到一次暂存约 +28,164 ppm，关掉 trim 数值不变；仅暂停 reader（不涉及网络迟到或时间戳断点）约 +36,980；只移动报告相位 41 ms 仍约 +28,179。用静音维持提交进度时持续阶跃消失但还有约 −1,704 的短暂估计；试验中仅比较 backlog 跳变也曾留下 −693。这些观察不证明现场原因。

## 批提交压力与旧源码复验

- **PASS**：`cargo test --manifest-path android-client-v2/native/Cargo.toml --features test-support --test clock_drift_session_contract batched_platform_writes_keep_the_window_and_reveal_long_run_drift -- --exact --nocapture`。合成 +75 ppm，固定种子的报告间隔 80–120 ms，理想累计播放向下量化到 2,052 帧（42.75 ms）。2,052 帧是已知容量，**不是**实测的单次 `AudioTrack.write` 粒度。通过实际 `MediaSession` 记账，启动后 1,083 个报告均有满窗；最坏相邻欠进度 1,980 帧（41.25 ms），最大 backlog 步进 2,028 帧（42.25 ms）。原始估计 −1,212…+1,025 ppm，中位数 +108、均值 +76.60；同采样时刻不量化的均值为 +74.60；断言使用已知量化余数推导偏差，容忍量仅为两个整数 ppm 输出各自的舍入误差。这证明该模式下窗口不饥饿、长程方向/量级正确，但 905/1,083 个窗口超出 ±100 ppm；此模型**不支持**逐窗 B 精度，均值不能替代 B 判据，也不能据此推断现场 write 粒度或真机结果。
- **FAIL**（仅在独立工作树临时恢复 `71794d2` 的 `drift.rs`、`media_session.rs`、`session.rs`，保留当前测试）：用上面相同的 `cargo test` 命令，移除 `--nocapture`，分别把测试名替换为 `one_stalled_delivery_invalidates_old_drift_and_recovers_real_nonzero_drift` 和 `catch_up_delivery_does_not_publish_a_playback_clock_outlier`。均成功编译，并分别因 `11,604 ms: Some(-303) != None` 和 `18,323 ms: Some(185488), window (10078, 100)` 失败。运行后用 `git restore --source=HEAD --worktree -- android-client-v2/native/src/drift.rs android-client-v2/native/src/media_session.rs android-client-v2/native/src/session.rs` 恢复生产源。

## 审查回归：共同停滞与诊断覆盖

```sh
cargo test --manifest-path android-client-v2/native/Cargo.toml --features test-support --test clock_drift_contract repeated_shared_stalls_do_not_starve_a_real_drift_estimate -- --exact
cargo test --manifest-path android-client-v2/native/Cargo.toml --features test-support --test clock_drift_contract unequal_progress_during_a_shared_stall_invalidates_the_drift_window -- --exact
cargo test --manifest-path android-client-v2/native/Cargo.toml --features test-support --test clock_drift_session_contract diagnostic_interval_preserves_a_recovered_outlier_and_reports_missing_coverage -- --exact
```

- **FAIL**：保留测试、临时恢复 `b801fe1` 的 `drift.rs`，首条命令编译成功，断言 `periodic shared pauses starved the window at 10000 ms`。
- **PASS**：首条与第二条输出 `ok`。每 5 s 同时暂停 61 ms、注入 +75 ppm，从 10 到 35 s 每次报告都有估计（73…76 ppm）；若两侧同时欠进度但相差 20 ms，已满的窗口立即失效。保留对大相对步进的隔离；仅在欠进度时容许不超过 1 ms 采样分辨率的相对偏差视作共同停顿。
- **PASS**：第三条输出 `ok`。30 ms 的短时捕获迟滞曾报 −175 ppm，到下个诊断快照时已回 0；区间仍导出 −175…0、51/51 有效估计。预热含 100 次报告但尚无有效估计；之后空区间 0/0、无极值、停报 5,000 ms，跨导出后停报间隔 5,100 ms。诊断只写本地日志，不改 Wire Protocol。若真机有效覆盖不足，不能因末值正常或没有极值而判 B 通过。

## 最终验证

| 项目 | 命令 | 结果 |
| --- | --- | --- |
| Native 默认特性 | `cargo test --manifest-path android-client-v2/native/Cargo.toml` | **PASS**：240 项，0 失败 |
| Native `test-support` | `cargo test --manifest-path android-client-v2/native/Cargo.toml --features test-support` | **PASS**：240 项，0 失败 |
| Native Clippy | `cargo clippy --manifest-path android-client-v2/native/Cargo.toml --all-targets --features test-support -- -D warnings` | **PASS**：退出 0 |

JVM **NOT RUN**（无 JVM 改动）。新产物 Windows GUI Server + Android 真机 B **NOT RUN**；不能据 host 结果关闭本工单、认定真机稳定或将 B 合入 beta。若异常再现，最少采集同一测量窗的逐报告时刻与两侧计数、附近 DATAGRAM 到达/时间戳和对应 trim/fill 已接受帧数，以区分现场的停滞、提交突跳或采样相位；不提交原始日志。
