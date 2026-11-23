# v0.6.0 发布边界与放行清单

> **历史记录（2026-11-23 状态）。** 本表中的标签/版本字段描述的是当时的快照，已不随代码更新。实际对外发布已改走 **v0.6.1**：GUI 与 v2 Android 关于页的 beta 文案、版权方与主页/更新 URL 已更正，产品版本升到 `0.6.1`（Android `versionCode 4`）。当前发布说明见 [`docs/releases/v0.6.1.md`](../../docs/releases/v0.6.1.md)。下表用于追溯 v0.6.0 的裁决与证据边界，不能据此构建或发布。

## 裁决与构建是两件事

[2026-11-22 维护者裁决](../wire-protocol-v2/roadmap.md#5-发布与迁移-gateexperimental--stable)只收窄这一次的 stable 门槛，不修改[协议规范](../../docs/wire-protocol-v2/spec.md)或完成 M6/M7。本次可描述为「经认证与加密的单 Client PCM，含已裁决的 M5 抖动缓冲、丢包掩蔽和漂移处理」；不能承诺多 Client 隔离、完整真实网络损伤恢复、端到端可听时延预算或全量 stable 验收。v2 失败不会回退到 v1、明文或匿名加密。v1 仅适用于受信网络。

| 对象 | 已知状态 |
| --- | --- |
| 代码与复测证据基线 | `ac9482a`；其后至标签的提交只改发布说明、路线图及决策记录。 |
| 本地 annotated tag `v0.6.0` | `675f290`（含发布说明草稿）；`9fece0d` 是其后的[本地标签记录](../wire-protocol-v2/map.md)，不是标签目标。远端状态本次未复查。 |
| 分支内产品版本 | 本分支 [`VERSION`](../../VERSION) 已是 `0.6.0`（Cargo/Tauri 已同步）。Android v2 [`versionName` / `versionCode`](../../android-client-v2/app/build.gradle.kts) 是 `0.6.0` / `3`，发布时按 ABI 各出一个 APK（`arm64-v8a`、`x86_64`）；两组版本仍独立于 Wire Protocol major。 |
| 标签内产品版本 | 本地 `v0.6.0` 标签指向的 `675f290` 仍是 `VERSION=0.5.0-beta`、Android `0.2.0-beta` / `2`，不能作为正式构建提交。 |
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

## 0.6.0 构建试跑记录（2026-11-23）

运行 [45107105796](https://github.com/W0n9/audio-share/actions/runs/45107105796)，提交
`f05452a`，参数 `version=0.6.0`、`create_draft=false`。首次 Android 作业因 Maven
依赖瞬时不可达失败，重跑后全部作业成功；draft 作业按预期跳过。

| 项目 | 状态 |
| --- | --- |
| CI 构建与打包（Server 四类 + 两个 ABI APK） | `PASS` |
| 产物 SHA-256 与 sidecar 一致性（本地下载后复核） | `PASS` |
| APK：`0.6.0` / `versionCode 3`、单 ABI、`CN=Audio Share Beta`、无 test-support | `PASS` |
| deb `0.6.0`、`as-cmd --version` 自报 `0.6.0`、Windows exe 含 `0.6.0` | `PASS` |
| 真机安装播放、Windows 运行、升级路径 | `NOT RUN` |

**Draft（2026-11-23，最终重建）**：文档整理提交 `88f6f07` 已推 fork 分支，
`v0.6.0` annotated 标签（原 `6499c2d` → 新 `812aeb1`）已 force-move 到该提交；旧 draft
及其 14 个附件已删除。在标签 ref 上运行
[45128075877](https://github.com/W0n9/audio-share/actions/runs/45128075877)
（`version=0.6.0`、`create_draft=true`）重新创建了**未发布 draft**：7 个构建文件 +
7 个 `.sha256` sidecar，共 14 个附件；正文由 `docs/releases/v0.6.0.md` 生成，包含下载表、
校验命令与验证边界。所有构建作业成功，标签对象解引用到 `88f6f07`。
此前的 [45114429469](https://github.com/W0n9/audio-share/actions/runs/45114429469)
draft 已被替换，其附件摘要核对记录不再适用于当前 draft。

**仍为 `NOT RUN`**：最终 draft 产物的真机安装播放、Windows 运行与升级路径。
发布前必须补做，或由维护者明确裁决豁免。

## 对外发布前须处理

- [x] **版本策略已定。** 产品与 Android 应用版本均为 `0.6.0`（`versionCode 3`），Rust/Tauri 元数据已通过 `scripts/apply_version.sh` 同步；Android 发布改为按 ABI 各出一个签名 APK。正式构建仍需从包含这些改动的提交进行。
- [ ] **先仅打包并验收。** 在正式提交所在的 ref 手动运行 [`Make Release`](../../.github/workflows/release.yml)，输入 `version=0.6.0`，保持 `create_draft=false`。下载 Actions artifacts，核对 Windows/Linux Server 与**两个** Android 签名 APK 的二进制版本、签名、单 ABI、SHA-256、无 `test-support`；记录配对、播放、停止、升级和兼容性复验。未执行的项目继续记 `NOT RUN`，需要额外豁免时由维护者裁决。
- [ ] **确认标签和对外文案，再创建 draft。** 审阅[对外说明](../../docs/releases/v0.6.0.md)与[证据草稿](../release-notes-v0.6.0.md)。确认远端 `v0.6.0` 标签指向正式构建提交后，选择同一提交重新运行 `Make Release`，设 `create_draft=true`。该次运行会重新构建，不复用先前 artifacts；核对 draft 附件的哈希与验收记录，必要时复验最终资产，再手动发布。不把原始日志当发布附件；工作流不会创建标签或自动公开 Release。
