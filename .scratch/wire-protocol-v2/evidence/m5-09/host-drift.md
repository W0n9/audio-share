# M5-09：Linux host 漂移窗口证据

## 历史观察与适用范围

B2 脱敏汇总（`client-B2.log` 离群附近约一分钟）：相邻约 5 s 快照内 hard re-anchor 增加 7 次、删除 64,672 帧；补静音增加 91 次、43,680 帧；原始漂移先 −38,082 ppm、后 +336,157 ppm（窗口分别 10,078/10,075 ms，各 100 点）。同轮 Server `media.stats` pending 0 / ≤66 ms，零丢弃。日志没有逐报告的 `(now_ms, capture_frames, playback_frames)`、逐包到达时间或逐次 trim/fill/AudioTrack 接受量，不能原样回放或确定现场根因。

## RED → GREEN

从本工单基线 `96e5f0a` 开始，仅修改测试时运行：

```sh
cargo test --manifest-path android-client-v2/native/Cargo.toml --features test-support --test clock_drift_session_contract -- --test-threads=1
```

- **FAIL**：`catch_up_delivery_does_not_publish_a_playback_clock_outlier` 经真实 `MediaSession` DATAGRAM、播放报告、trim 入账路径。合成双 48 kHz，仅把连续 PCM 的交付在 5 s 内暂存约 251 ms 再批量送达 7 次，触发 7 次 hard trim、合计 64,656 帧；峰值 `+185488 ppm`（10,078 ms / 100 点）。正常不暂存的对照 **PASS**：0 ppm、无 hard trim。同样是满窗离群，不是 B2 现场回放。
- **FAIL**：缩减至一次暂存；等时钟产生 `+28164 ppm`。仅关掉 trim 的测试探针也为 `+28164 ppm`；仅暂停 reader 而保持网络正常（无迟到或发送时间戳断点）为 `+36980 ppm`；仅错开报告相位 41 ms 为 `+28179 ppm`，仍是满窗。仅用接受的静音维持播放提交进度时，持续阶跃消失，但仍有短暂 `−1704 ppm`。这些探针只用于区分这个合成输入的原因。
- **FAIL**（仅比较 backlog 跳变的探针）：两侧同时欠进度时仍短暂报 `−693 ppm`。
- **PASS**：同时检查各边相对名义进度的欠量后，本次合成离群消失。60 ms 是观测连续性的容忍量，独立于 ring 策略，不是 ppm 限制。正常周期性 20 ms 批交付不饿死估计窗口。已知位置步进的软 240 帧、硬 4,800 帧和接受静音对照仍通过；单次断续使旧 +75 ppm 失效、修正平滑回零，完整新窗口恢复 +75 ppm。稳定注入 ±10,000 ppm 仍可原样测出，施加的修正分别为 ±100 ppm。补静音对照复跑后只发布 0 ppm（停滞窗口不发布估计）。合成 7 次 hard trim 仍是 7 次，未改变其触发策略。重建样本的 5 s 本地诊断见 [真机复测清单](windows-retest.md)。

## 批提交压力与旧源码复验

- **PASS**：`cargo test --manifest-path android-client-v2/native/Cargo.toml --features test-support --test clock_drift_session_contract batched_platform_writes_keep_the_window_and_reveal_long_run_drift -- --exact --nocapture`。合成 +75 ppm，固定种子的报告间隔 80–120 ms，理想累计播放向下量化到 2,052 帧（42.75 ms）。2,052 帧是已知容量，**不是**实测的单次 `AudioTrack.write` 粒度。通过实际 `MediaSession` 记账，启动后 1,083 个报告均有满窗；最坏相邻欠进度 1,980 帧（41.25 ms），最大 backlog 步进 2,028 帧（42.25 ms）。原始估计 −1,212…+1,025 ppm，中位数 +108、均值 +76.60；同采样时刻不量化的均值为 +74.60。这证明该模式下窗口不饥饿、长程方向/量级正确，但 905/1,083 个窗口超出 ±100 ppm；此模型**不支持**逐窗 B 精度，均值不能替代 B 判据，也不能据此推断现场 write 粒度或真机结果。
- **FAIL**（仅在独立工作树临时恢复 `71794d2` 的 `drift.rs`、`media_session.rs`、`session.rs`，保留当前测试）：用上面相同的 `cargo test` 命令，移除 `--nocapture`，分别把测试名替换为 `one_stalled_delivery_invalidates_old_drift_and_recovers_real_nonzero_drift` 和 `catch_up_delivery_does_not_publish_a_playback_clock_outlier`。均成功编译，并分别因 `11,604 ms: Some(-303) != None` 和 `18,323 ms: Some(185488), window (10078, 100)` 失败。运行后用 `git restore --source=HEAD --worktree -- android-client-v2/native/src/drift.rs android-client-v2/native/src/media_session.rs android-client-v2/native/src/session.rs` 恢复生产源。

## 最终验证

| 项目 | 命令 | 结果 |
| --- | --- | --- |
| Native 默认特性 | `cargo test --manifest-path android-client-v2/native/Cargo.toml` | **PASS**：237 项，0 失败 |
| Native `test-support` | `cargo test --manifest-path android-client-v2/native/Cargo.toml --features test-support` | **PASS**：237 项，0 失败 |
| Native Clippy | `cargo clippy --manifest-path android-client-v2/native/Cargo.toml --all-targets --features test-support -- -D warnings` | **PASS**：退出 0 |

JVM **NOT RUN**（无 JVM 改动）。新产物 Windows GUI Server + Android 真机 B **NOT RUN**；不能据 host 结果关闭本工单、认定真机稳定或将 B 合入 beta。若异常再现，最少采集同一测量窗的逐报告时刻与两侧计数、附近 DATAGRAM 到达/时间戳和对应 trim/fill 已接受帧数，以区分现场的停滞、提交突跳或采样相位；不提交原始日志。
