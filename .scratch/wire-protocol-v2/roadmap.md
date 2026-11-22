# Wire Protocol v2 — 工程路线图

阅读提示：§1–4 混合了不同日期的规划和回填记录。v0.6.0 以 §5 的 2026-11-22 裁决为准；最新整理见 [v0.6.0 发布审阅包](../v0.6.0-review/README.md)。

2026-11-16 基线：M4（13–15）已合入 `phase-2/wire-protocol-v2` @ `1cb4e80`（M4 主体 `0377b59` + 真机修复）；维护者授权 Customer Beta 路径并增加 `EMULATOR` 证据状态。后续 M5、Beta 与发布裁决见 [`map.md`](map.md)。

## 0. 本文的定位与读法

- 本文是**排期/依赖视图**，不是规范、不是 ADR、不是工单。协议行为以 `docs/wire-protocol-v2/{spec,pairing,threat-model}.md`（2026-11-12 accepted）与 `docs/adr/0004-wire-protocol-v2-transport.md` 为准；`.scratch/wire-protocol-v2/spec.md` 留作产品与验收语义的基线。v0.6.0 的例外只适用于本次发布门槛，见 §5。
- 进度与裁决的权威记录是 `map.md`（Notes / Decisions-so-far / Fog）和 `issues/NN-*.md`。工单的创建、`Blocked by`、`Status: claimed/resolved`、frontier 规则见 `docs/agents/issue-tracker.md`。
- 证据状态词的权威定义见 `docs/agents/evidence-policy.md`（`PASS` / `FAIL` / `NOT RUN` / `WAIVED` / `EMULATOR`）。
- 状态词：`DONE` 已完成并有证据 · `NEXT` 下一个可执行范围 · `PLANNED` 已列入但尚未创建工单 · `NOT RUN` 未验证，不得写成 PASS · `WAIVED` 维护者明确豁免，不等于 validated · `EMULATOR` 仅在 Android 模拟器上验证，真机仍为 `NOT RUN`，不得写成真机 PASS。
- 本文**不含排期日期与负责人**：仓库中没有这类信息，由维护者确定后补入。
- 本路线图已由 Git 跟踪；原始证据仍按证据政策默认留在本地。

## 1. 当前位置（基线快照）

| 里程碑 | Tracer-Bullet 步 | 工单 | 结果 |
|---|---|---|---|
| M1 传输原型 | 1 | 01 | `DONE` — `prototype-report.md` rev3 判定 **GO**（9 PASS / 0 FAIL / 2 项 WAIVED），真机 arm64 Android 12 / API 31 上验证 ALPN、SPKI pin、加密控制流授权、DATAGRAM 媒体、前台 Service、干净关闭 |
| M2 规范冻结 | 2 | 02–05 | `DONE` — ADR-0004、normative spec、pairing/key-lifecycle、threat model、schema 与跨语言 golden vectors 均 accepted；工单 05 标记 phase-2 gate closed |
| M3 最小认证控制通道 | 3 | 06–10 | `DONE` — Server v2 transport、认证状态机、Android v2 会话、`phase-3-report.md` 交叉验证、最终两轴 review 修复；工单 09 明确「第 3 步完成，第 4 步解锁」 |
| M3 收尾回归 | 3 | 11–12 | `DONE` — 前台 Service 真机回归、capture absent/unhealthy 的 Subscribe 501、真实配置发布、StreamStarted 去占位化；两轴审查 0 Standards 硬违规 + 2 个 Spec 缺陷真断言 RED→GREEN |

最终验收（`evidence/post-fix-device-acceptance.md`）：workspace 465 PASS / 4 既有 ignored、native 默认与 `test-support` 各 102、JVM 36、默认 `assembleRelease` PASS、真机 READY / rebind / recreate / lock / ACTION_STOP→701 全 PASS；frozen v0.3.4 APK SHA 匹配且 wire probe PASS。

**基线能力边界（`a5de2d9`，M4 之前）**：只有控制通道的 READY 稳态，没有 v2 媒体发送或播放、播放统计、抖动缓冲/漂移修正、生产 pairing（QR/SAS/Keystore）、GUI/CLI 生命周期状态或多 Client 隔离。M4 随后实现单 Client PCM、`AudioTrack` 播放、§9 统计与显式关闭，真机复验见下表；再之后的 Beta、M5 实施与裁决以 `map.md` 的对应记录为准。

## 2. 里程碑与完成判据（M4–M7）

依赖以「前一个里程碑的 DoD 全部满足」为准；任何一步的验收数字被实测证伪时，**显式修订预算并附新证据**，不得悄悄放宽测试而保留旧结论（baseline §Measurable Initial Targets）。

| 里程碑 | 步 | 状态 | 范围 | 依赖 | 交付物 | 完成判据（DoD） |
|---|---|---|---|---|---|---|
| **M4 单 Client PCM 媒体** | 4 | `DONE`（13/14/15 全部 `resolved`；2026-11-15 已 fast-forward 合入 `phase-2/wire-protocol-v2`——M4 主体 `0377b59`，真机修复与证据 `1cb4e80`，集成分支现指向 `1cb4e80`；host/JVM/跨端回归全绿；真机 AudioTrack 出声、锁屏连续播放、§9 诊断节奏、Close(701)、339 s 窗口 0.98x 实时均取得证据；60 分钟 soak/双真机/API26 仍 NOT RUN） | Server 侧 capture→DATAGRAM 发送路径；Android 侧接收、jitter-buffer 前的有序交付、`AudioTrack` 播放；`PlaybackDiagnostics` 5 s 周期上报；真实 `StreamStarted`/`StreamEnded` 与显式 Close | M3；`docs/wire-protocol-v2/spec.md` §7、§9、§5.2 | 可跑通的单 Client 端到端音频；Server/Client 两侧媒体与统计日志；既有媒体/控制 golden vectors 的互操作验证 | 24-byte BE 头（§7.1）在两端一致；`max_datagram_size()` 按发送时刻取值、payload frame-aligned、无 IP 分片依赖；§9 全部字段可观测且统计失败不阻塞媒体；`Close` 携带 §6 registry code，错误载荷不含密钥/内部路径/堆栈（§4.2、TM-7）；生产 `StreamStarted` 从不可达变为可达且字段来自真实 stream 状态；v1 fixtures / v0.3.4 APK 无变化 |
| **M5 抖动、漂移与损伤恢复** | 5 | `OPEN` — 工单 [m5-01](issues/m5-01-end-to-end-latency-baseline.md)…[m5-07](issues/m5-07-60min-soak.md)（2026-11-19 开票；用 `M5-NN` 编号避开 16–21 冲突）。L1 阶段 1 已完成：采集→提交 p50 96.8 ms（ring ~46 + 上游 ~55），AudioTrack 128→42 ms 真机听测连续、21 ms 爆音，端到端可听延迟推算 ≈225 ms（未实测出声）。见 [`l1-handoff/l1-results.md`](l1-handoff/l1-results.md) | 有界 jitter buffer（target 60 ms / hard limit 200 ms）、late drop 与 concealment 计数、±100 ppm 漂移测量与修正、paced sends（send-ahead ≤ 100 ms）、故障/重连后恢复 | M4 | §8 全部机制；§11.2 矩阵逐行 PASS/FAIL/NOT RUN 报告；parser fuzz 工装 | 2% loss / 25%·50% reorder / 5% duplicate / 10 ms±5 ms jitter / 300 kbit·queue50 congestion / MTU 1280 / ±100 ppm drift / 25 s idle / connection-kill reconnect 全部有独立证据；network loss 与 `late_dropped` 分开计数；QUIC ACK 或 socket 收包不得当作播放证据；60 分钟无死锁、无假活会话、无不可恢复乱序、无延迟持续增长 |
| **M6 多 Client 与生产化** | 6 | `PLANNED` | 每 Client 独立 session/sequence/buffer/pacing/统计；共享 capture 不被慢 Client 阻塞；生产 pairing（QR、SAS、Keystore）、revocation UX；headless pairing 与 GUI/`as-cmd` 共用 pairing Core；诊断导出（脱敏） | M5；pairing.md §10–11、spec §10 | 双真机并发验收；`as-cmd pair` / `clients` 命令族；GUI/CLI 共同状态视图 | §10 全部限额有测试（32 sessions、16 handshakes、303 supersede、5/20 失败限额、256 datagram 队列并 drop newest 等）；一个慢 Client 不拖慢 capture 与其他 Client；错误签名、未 pin 身份、未授权 Client 分别被拒；测试专用 bypass 不进生产二进制；revocation 立即影响新连接并终止活动会话（已有证据 `phase-3-report.md` item 5a：0.800 s；规则见 pairing.md §9.2） |
| **M7 全量验收、发布与迁移** | 7 | `PLANNED` | 完整 acceptance、v1 回归、release 与 migration 文档、stable 判定 | M6 + 所有 `NOT RUN` 项处置完毕 | acceptance 报告、release notes、migration guide、rollback path、v1 标注/独立禁用开关 | §Later Implementation Acceptance 全绿：Rust↔Android 互测而非各自编码器自测、负向/未知字段/状态违规、确定性损伤注入、parser fuzz、**至少两台真机 Client 并发 60 分钟** + Server soak（泄漏/会话累积）、六套 v1 fixture replay、frozen v0.3.4 APK 验收；v1 在每个阶段可构建可发布 |

### Customer Beta 路径（2026-11-16 维护者授权）

Beta 是**受控试用版 Android Client + 对应 Server 包**，不是 M5–M7 的替代或提前完成。它复用 M1–M4 的传输、认证控制通道与单 Client PCM 媒体，补齐生产 pairing、最小 UI、GUI 管理与发布验收。范围与排除项以 [`beta-spec.md`](beta-spec.md) 为准；M5（抖动/漂移/损伤）、M6（多 Client 与诊断导出）、M7（全量 stable 验收）仍是 stable-gate 工作。

| 阶段 | 工单 | Blocked by | 状态 |
|---|---|---|---|
| Server Pairing Core、`as-cmd` 配对/管理、身份重置 | 19 | — | `DONE` — 合并 `9e5f916` |
| Android native 配对、Keystore 凭据、撤销/未知身份 | 20 | 19 | `DONE` — 合并 `65be014`（`EMULATOR` 通过，真机 `NOT RUN`） |
| Server GUI 配对与已配对设备管理 | 22 | 19 | `DONE` — 合并 `2ce53ab`（Linux 手动 PASS，Windows `NOT RUN`） |
| 最小 Compose UI（Home/Pairing/Playing/About） | 21 | 20 | `DONE` — 合并 `d01f3e3`（`EMULATOR` 通过，相机实扫 `NOT RUN`） |
| `lock` / `recreate` 生命周期测试修复 | 23 | — | `DONE` — 合并 `fe8da1c`（仅 `EMULATOR`，真机复跑留给 25） |
| 授权存储原子替换（202 漂移） | 27 | — | `DONE` — 合并 `fdc8b19` |
| Beta 打包与签名（APK + Server `0.5.0-beta`） | 24 | 20, 21, 22 | 本地产物 `DONE` — 合并 `66d168f`；Windows 产物与正式发布 `ready-for-human` |
| 真机 / Windows / v1 回归验收与证据 | 25 | 23, 24 | `ready-for-human` |
| 证据词汇与 roadmap 对齐 | 26 | — | `DONE` — 维护者签核已记录（`a7f135a`） |
| 最终两轴审查修复 | 28 | 24 | `DONE` — 合并 `ce9eba3` |

Beta 明确排除：M5 抖动缓冲/隐藏/漂移/损伤矩阵与 fuzz、多 Client 隔离与 60 分钟双真机、Server GUI 实时 v2 会话/诊断视图、脱敏诊断导出、GUI 身份重置、v1 独立禁用开关、F-Droid 与 in-app 自动更新、开机启动、mDNS/STUN/TURN/relay、Opus/FEC/DTX/PLC。

### M4 工单与测试 seam（2026-11-14 已确认）

13–15 已在 `issues/` 建立，frontier 仍按 tracker 实时计算。维护者确认三个公共 seam：Server capture→认证 QUIC 输出；native 媒体交付/实际播放提交/统计；Android 播放与真实 Service/Binder。测试逐切片 RED→GREEN，不修改 frozen v1 fixtures，不自动提交/push。13/14 已 resolved；15 已完成 Server+Android 合并回归与 `tools/rust-android-interop/` 跨端互测（12 tests，含真实认证 live-QUIC、全新隔离身份），待两轴 review 收尾。证据见 `evidence/ticket-{13,14,15}/README.md`。真机播放与长时间音质仍为 NOT RUN。

| 编号 | 主题 | 里程碑 | Blocked by | Type |
|---|---|---|---|---|
| 13 | Server 媒体发送路径与 pacing（§7.1–7.4） | M4 | 12 | task |
| 14 | Android 媒体接收/播放与 `PlaybackDiagnostics`（§7.5–7.6、§9） | M4 | 13, 05 | task |
| 15 | 流生命周期与显式 Close 端到端（§5、§6） | M4 | 13, 14 | task |
| 16 | ~~jitter buffer、concealment、drift（§8）~~ → 由 **M5-03 / M5-04** 取代 | M5 | — | task |
| 17 | ~~impairment conformance + 负向矩阵 + fuzz（§11.2/§11.3）~~ → 由 **M5-06** 取代 | M5 | — | task |
| M6-01 | 多 Client 隔离与 Server soak（§8、§10） | M6 | M5-06 | task |
| M6-02 | 生产 pairing / Keystore / headless 授权（pairing.md §10–11） | M6 | 12, 04 | task |
| M6-03 | GUI 与 `as-cmd` 共同状态、诊断导出 | M6 | M6-01, M6-02 | task |
| M7-01 | 全量验收、v1 回归、release 与 migration 文档 | M7 | M5-06, M6-03 | task |

> **编号重排（2026-11-20）**：M5 工单用 `M5-NN`，取代 16/17；M6/M7 开票时统一用
> `M6-NN` / `M7-NN`，避开 Beta 工单编号。上表 M6/M7 仅作范围占位，本轮不开票。

## 3. 硬性门槛（不得降级）

| 门槛 | 出处 | 状态 |
|---|---|---|
| Transport Prototype Gate（11 项 hard gate 逐项 PASS/FAIL/NOT RUN） | baseline §216–220；`prototype-report.md` §2 | `PASS with waivers` — GO，但 API 26 真机运行时与 Wi-Fi↔蜂窝迁移是 WAIVED，不得写成 validated |
| Normative Specification Gate（ADR、normative spec、state machine、schema、error registry、threat model、golden vectors、limits） | baseline §222–230 | `DONE` — 2026-11-12 maintainer accepted；决策表 D1–D12 已签署 |
| Later Implementation Acceptance | baseline §232–240；normative spec §11 | `NOT RUN` — 由 M4–M7 逐项积累，M7 汇总 |

固定验收数值：`jitter buffer target 60 ms / hard limit 200 ms`；媒体路径额外时延 `p95 ≤ 80 ms`、`p99 ≤ 120 ms`（约定 normal-LAN 场景）；60 分钟运行不出现死锁、假活、不可恢复乱序或延迟持续增长；loss / reorder / late-drop / concealment / underrun / 端到端可听时延必须分别上报。网络到达指标 ≠ 端到端可听时延指标。

## 4. 遗留与复验清单

| 项目 | 当前状态 / 来源 | 归属 | 是否阻断 stable |
|---|---|---|---|
| API 26 真机运行时 | `WAIVED`（仅 build-level 合规：minSdk 26、API-26 bionic link、无依赖抬高）；`prototype-report.md` §2/§7 | M7 补测或显式记录"未验证" | 是（至少需显式声明） |
| Wi-Fi↔蜂窝迁移与 QUIC path validation | `WAIVED`（环境无此场景）；`Network.bindSocket` 在 MIUI 返回 EPERM，teardown+退避重连这一生产承诺已 PASS；ADR-0004 §4 | M5 的重连矩阵 / M7 复验另一台设备判断 EPERM 是否 MIUI 特有 | 否（迁移不是 v2.0 承诺，普通重连是） |
| 设备级 stray/malformed frame 注入、错误签名 proof、单连接 5 次失败耗尽 | `NOT RUN`（smoke UI 无注入面），host live-QUIC / state-machine 已覆盖；`phase-3-report.md` §91–95 | M4 补注入面 + M7 汇总 | 是 |
| fixture server key 为 PKCS#8 v1，ring 0.17 拒绝 | 已记录，验收使用临时新生成身份；`phase-3-report.md` §96 | M4（重生成 fixture 或显式 v1-tolerant 载入路径） | 是（golden vectors 可复现性） |
| 默认 release 不含 fixture 身份；设备认证复验需显式 `-PtestSupport=true -PdeviceTests=true` | 已记录；`evidence/final-review-1112.md` §188–204 | M4/M7 验收脚本固定该约定 | 是（不得把 Java 编译/host stub 称作设备 PASS） |
| 设备媒体 pacing：audio-seconds 比 wall-clock 约 +14%，原因未查明 | `prototype-report.md` §4/§7 | M4 首个媒体工单优先调查 — **2026-11-15 部分结案**：真机端到端发现并修复「Server 媒体任务丢弃唤醒 chunk」（曾致 0.01x 实时）；修复后 339 s 窗口 0.98x。原型期 +14% 的成因仍未单独复现，**无 underrun 播放**与 p95/p99 时延仍属 M5 §8 | 是（影响时延预算） |
| Windows、API 26 真机、长时间音质 / 60 分钟矩阵、Server soak | `NOT RUN`；`evidence/post-fix-device-acceptance.md` §62–66 | M7 | 是 |
| v1 "trusted networks only" 标注与独立禁用开关、`media.rs` 精简、`send_queue` 恢复 | 工单 12 scope 外、review 其余项另行裁决；Beta 19 负责在 `as-cmd --help` 与发布说明中标注 v1 明文且仅限受信网络 | 独立禁用开关仍为 stable-gate | 是（产品承诺） |
| 多 Client 隔离、60 分钟双 Client、Opus/FEC/DTX/PLC | 原型未验证；v2.0 明确无 FEC/重传，Opus 等为未来 capability | M6 / 未来 capability 单独提案 | 否（除多 Client 与 soak 外） |
| production pairing、QR/SAS UX、Keystore 真实密钥、revocation UX | Beta 19（Server Pairing Core/CLI）、20（Android native/Keystore）、22（GUI）实施中；M6 多 Client 隔离仍为 stable-gate | Beta + M6 | 是 |

原型 `prototype-report.md` §7 中「normative wire layout / error registry / Protobuf schema 尚不存在」一条**已由 M2 的 03/04/05 关闭**，不再列为遗留。

## 5. 发布与迁移 gate（experimental → stable）

- 当前产品状态（baseline §47–55、ADR-0004 §41）：v1 冻结在 65530；v2 独立 UDP 65531、ALPN `audio-share/v2`、默认与 v1 同时启用但标记 **experimental**；新 Android Client 优先 v2，**v2 失败后绝不自动回退 v1**、明文或匿名加密。
- stable 判定前置：M4–M7 全部 DoD 满足；真实 v2 Android release 存在；migration guide 与 rollback path 已发布；v1 的 "trusted networks only" 标注与独立禁用开关可用；§4 中标记「阻断 stable」的项全部关闭或有维护者书面豁免。

> **2026-11-22 维护者裁决（stable 门槛收窄）**：M5 未完成项（§11.2 真实网络行、60 min soak、
> p95/p99、端到端可听基线）、M6（多 Client 与生产化）、M7（全量验收、v1 回归、migration/
> rollback）以及 §4 中标「阻断 stable」的项，**均不构成本次 stable 的门槛**，转入后续里程碑。
> 这是维护者的书面豁免，不代表这些项已验证：release notes 与相关文档必须继续标
> `WAIVED` / `NOT RUN`，不得写成 `PASS`。本次 stable 的实际能力边界为单 Client
> （M4 + 已裁决的 M5 子集）。
- v1 **不设移除日期**；只有在上述条件成立后才重新讨论其默认值。产品版本、Android app 版本、Wire Protocol major 是三个独立值。
- 发布产物：release notes、migration guide、rollback path、v1/v2 双协议配置说明、脱敏诊断导出说明。不得引入外部 telemetry、原始 PCM 落盘或密钥日志。

## 6. 边界与非目标（v2.0）

v1 framing / commands / UDP / little-endian 行为冻结；0-RTT 全禁；Android baseline API 26（任何抬高 minSdk 的依赖都重开决策）；首个远程角色只有 player；局域网/VPN/用户配置端口转发可达，STUN/TURN、NAT 打洞、relay、账号系统、浏览器/iOS Client 不在首个 v2 release；无应用层重传、无 PCM FEC、无 Opus/DTX/PLC；sample-accurate 跨手机同步不承诺。

## 7. 维护规则

1. 每 resolve 一个工单：先按 wayfinding 把答案与上下文指针写进 `map.md` 的 Decisions-so-far，再更新本文件对应里程碑的状态与证据链接。
2. frontier 仍从 `issues/` 实时计算（open + unblocked + unclaimed，编号最小者优先）；本文的 `NEXT` 只是建议，不替代 claim/resolve 流程。
3. 每次更新必须区分 `PASS` / `FAIL` / `NOT RUN` / `WAIVED` / `EMULATOR`（定义见 `docs/agents/evidence-policy.md`）；不得把 host stub、Java 编译、单端编码器自测冒充真机或互操作 PASS；`EMULATOR` 不得写成真机 PASS。
4. 计划与规范冲突时修改计划；只有维护者裁决 + 新证据才能修改规范数值或验收门槛。
