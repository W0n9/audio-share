# v0.6.0 发布边界与放行清单

## 裁决与构建是两件事

[2026-11-22 维护者裁决](../wire-protocol-v2/roadmap.md#5-发布与迁移-gateexperimental--stable)只收窄这一次的 stable 门槛，不修改[协议规范](../../docs/wire-protocol-v2/spec.md)或完成 M6/M7。本次可描述为「经认证与加密的单 Client PCM，含已裁决的 M5 抖动缓冲、丢包掩蔽和漂移处理」；不能承诺多 Client 隔离、完整真实网络损伤恢复、端到端可听时延预算或全量 stable 验收。v2 失败不会回退到 v1、明文或匿名加密。v1 仅适用于受信网络。

| 对象 | 已知状态 |
| --- | --- |
| 代码与复测证据基线 | `ac9482a`；其后至标签的提交只改发布说明、路线图及决策记录。 |
| 本地 annotated tag `v0.6.0` | `675f290`（含发布说明草稿）；`9fece0d` 是其后的[本地标签记录](../wire-protocol-v2/map.md)，不是标签目标。远端状态本次未复查。 |
| 标签内产品版本 | [`VERSION`](../../VERSION) 是 `0.5.0-beta`。Android v2 [`versionName` / `versionCode`](../../android-client-v2/app/build.gradle.kts) 是 `0.2.0-beta` / `2`；两组版本独立，Wire Protocol major 也是独立值。 |
| 已测试产物 | M5-12 是 Windows Server `431997b` 和 Android arm64 **debug** APK `4120cdd`。签名 APK / Server 软件包尚未为 `v0.6.0` 重建并验证，不能把测试产物当发布资产。 |

## 证据账本

以下是已有记录，不是本次文档整理重跑的结果。状态定义见[证据政策](../../docs/agents/evidence-policy.md)。

| 项目 | 结果与边界 |
| --- | --- |
| M4：单 Client PCM、锁屏连续出声、显式关闭 | `PASS`（M4 历史 Android 真机，合并 `1cb4e80`；339 秒窗口约 0.98x 实时）。见[路线图 M4 记录](../wire-protocol-v2/roadmap.md#2-里程碑与完成判据m4m7)，不是 v0.6.0 资产验收。 |
| M5-09：Windows 用例 B | `PASS`（Windows + API 31 真机 48 kHz，按 [2026-11-21 裁决](../wire-protocol-v2/evidence/m5-09/windows-retest.md)的长期漂移、覆盖和有界条件）。[48 kHz 原报告](../wire-protocol-v2/evidence/m5-11/RESULT-WINDOWS-48K.md)的旧逐窗口径仍是 `FAIL`；真机注入 ±100 ppm 未测。 |
| M5-12：控制器饱和复测及 host 回归 | 真机 `PASS`（仅判定窗）：0/180 顶轨，暖机 17/33 另计；host `PASS`（native 260/260 两种配置、Clippy、fmt、JVM 278）。见[脱敏真机结果](../wire-protocol-v2/evidence/m5-12/RESULT-WINDOWS.md)、[host 结果](../wire-protocol-v2/evidence/m5-12/host-drift-saturation.md)和[工单](../wire-protocol-v2/issues/m5-12-drift-correction-saturation.md)。 |
| M5-01 / M5-07：可听基线、单 Client 60 分钟、p95 ≤ 80 ms / p99 ≤ 120 ms | `WAIVED`；缺外部录音设备与值守条件，不是 `PASS`。见 [M5-01](../wire-protocol-v2/issues/m5-01-end-to-end-latency-baseline.md) / [M5-07](../wire-protocol-v2/issues/m5-07-60min-soak.md)。 |
| §11.2 真实损伤：loss、reorder、duplicate、jitter、congestion、MTU、idle、reconnect 等 | `NOT RUN`（M5 真实网络矩阵）；[host 确定性矩阵](../wire-protocol-v2/evidence/m5-matrix/matrix.md)的结果不能替代真机。 |
| Android 硬件/听感覆盖 | API 26 真机为 `WAIVED`（构建与历史模拟器证据不是该机型验收）；Wi-Fi↔蜂窝迁移为原型期 `WAIVED`。M5-12 听感 A/B、真机 ppm 注入、平台实际速率回读为 `NOT RUN`。 |
| M6 / M7 | 不构成本次门槛：双真机并发、60 分钟 Server soak、Windows 长跑、v0.6.0 的 v1 回归为 `NOT RUN`；完整迁移指南与回滚方案未交付。旧 beta 的[真机与 v1 共存记录](../wire-protocol-v2/evidence/ticket-25/ticket-25-result.md)适用于当时的产物，不外推。 |
| 旧 v2 beta 就地升级（205） | 修复与回归 `WAIVED`。记录的绕行是先卸载旧 v2 beta，再安装并重新配对；这是升级提示，不是完整迁移指南。见[工单 34 的 map 裁决](../wire-protocol-v2/map.md#decisions-so-far)。 |

## 对外发布前须处理

- [ ] **先确定正式构建提交与版本策略。** 保留本地标签直到维护者决定其最终用途；本次不改打标签。核对 Server/产品发布号、独立的 Android 应用版本、签名与资产命名。现有 [`Make Release`](../../.github/workflows/release.yml#L152-L208) 是手动触发的 draft 流程：它当前会读出 `0.5.0-beta`，尝试创建 `v0.5.0-beta` pre-release 草稿，并选用过时的 beta 说明（其中写「无抖动缓冲与漂移修正」）。不能用它原样发布 v0.6.0。若只把 `VERSION` 改成 `0.6.0`，stable 分支还会查找目前不存在的 `metadata/en-US/changelogs/600999.txt`；Android 签名 APK 的版本断言与文件名也仍是 beta 硬编码。
- [ ] **用确定的提交出包并验收。** 核对 Windows/Linux Server 与 Android 签名 APK 的二进制版本、签名、ABI、SHA-256、无 `test-support`；在实际候选产物上记录配对、播放、停止、升级和兼容性复验结果。未执行的项目继续记 `NOT RUN`，需要额外豁免时由维护者裁决。
- [ ] **核准对外文案和发布目标。** 审阅[发布说明草稿](../release-notes-v0.6.0.md)，确认单 Client 与所有未验证项的披露；确认远端、tag 目标与 draft 资产后，再决定是否推送与发布。不把本审阅包或原始日志当发布附件。
