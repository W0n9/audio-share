# 08: 重构验证与 beta 证据保全

Labels: wayfinder:grilling
Type: grilling
Status: resolved
Blocked by: 02

Preconditions from [06](06-navigation-and-state-restoration.md#answer)：验证矩阵至少覆盖
「旋转/重建」「进程死亡」「Service 已有会话时重绑」三类恢复场景。
Preconditions from [07](07-kernel-audio-refactor-scope.md#answer)：加入
`SessionLifecycleDriver` 的 JVM 时序测试，以及 `EMULATOR` 上的网络回调/rebind 场景。

## Question

给出重构的验证矩阵与证据策略：

1. 哪些项用 JVM、`EMULATOR`、真机？沿用 `docs/agents/evidence-policy.md` 的 `PASS` / `FAIL` / `NOT RUN` / `WAIVED` / `EMULATOR` 词汇。
2. 哪些 beta 证据在重构合入时必须重跑（配对、锁屏播放、断网重连、v1 回归），哪些可以保留并标注「重构前证据」。
3. 如何避免 ticket 25 的验收证据因重构失效；合入顺序与证据时间线如何记录。
4. UI 与架构项的验收场景清单：每个 outcome、每个导航恢复场景至少一个可执行断言。

已知约束（来自 [02](02-beta-freeze-and-merge-point.md#answer)）：post-refactor 证据目录为 `evidence/ticket-08-refactor/`；每条记录绑定 commit SHA 与产物哈希；重构前证据不自动失效，也不自动继承。

产出是验证矩阵与合入门槛，不是测试实现。

## Answer

**结论（用户逐条确认）：JVM / `EMULATOR` / 真机三层分工；25 的 Windows、协议、v1 与
30 分钟真机证据保留为「重构前证据」；真机只跑四项关键路径；证据绑定精确 SHA + 产物哈希；
25 GO 并冻结后采集最终证据；缺项或 `FAIL` 不合入。**

### 1. 验证矩阵（分层）

| 层级 | 覆盖内容 |
|---|---|
| **JVM**（含 Robolectric） | outcome policy/store、reducer 导航、Activity recreate、driver 时序、UI 屏状态、配对/凭据/策略/golden vectors、v1 fixture replay |
| **`EMULATOR`** | 签名 APK 安装/R8、QR+SAS 配对、READY→PLAYING、Stop(701)、锁屏、旋转、进程死亡、rebind/recreate、撤销后 204、API 26 smoke、30 分钟趋势 |
| **真机** | QR+SAS 配对、5–10 分钟可听播放、锁屏 2–5 分钟、临时断网恢复 + Stop |

- JVM 结果记为 `PASS (JVM)`，永不写成设备 `PASS`。
- `EMULATOR` 记录必须含 `emulator:<avd>`、API、ABI、app/Server 版本、build flags，
  并写明真机 `NOT RUN`。
- 30 分钟连续播放不再要求真机复跑：`EMULATOR` 做趋势/泄漏观察，真机做 5–10 分钟可听。

### 2. beta 证据保留 vs 重跑

**保留为重构前证据（不继承、不作为 post-refactor `PASS`）**

- 工单 25 的 Windows GUI/Server 证据与哈希；
- Server 协议、撤销、`as-cmd` 证据；
- v1 冻结 APK 与 v1 fixtures 的既有结果；
- 25 的 30 分钟真机音频/时延证据。

**必须对 post-refactor 产物重跑**

- Android 侧全部配对（QR / SAS / 失败 / 重配对）；
- 播放、锁屏、Stop（应用内与通知）；
- 断网、rebind、recreate、进程死亡；
- UI 状态、导航、旋转恢复；
- API 26 安装与功能 smoke；
- 签名 APK 安装性、R8、资源打包；
- v1 fixture replay（JVM）与 v1/v2 共存安装 smoke（`EMULATOR`）。

Windows 与 Server 不因 Android 重构重跑；若 refactor 分支改动了 Server/Rust 产物，
本表需重新评估（当前 07 已排除 Rust 重构）。

### 3. 真机最小矩阵与豁免

四项：**QR+SAS 配对 / 5–10 分钟可听播放 / 锁屏 2–5 分钟 / 临时断网恢复 + Stop**。

- 默认门槛：四项全 `PASS`。
- 任一 `NOT RUN` 时，仅维护者可显式 `WAIVED`；判决标记「条件通过」，写明风险与复跑计划。
- 不允许把 `NOT RUN` 静默当作通过。

### 4. 证据目录、manifest 与时间线

```
evidence/ticket-08-refactor/
├── manifest.tsv
├── timeline.md
├── jvm/
├── emulator/
└── device/
```

`manifest.tsv` 每行必须含：item、tier、status（`PASS`/`FAIL`/`NOT RUN`/`WAIVED`/`EMULATOR`）、
commit SHA、APK SHA-256、Server 产物 SHA-256、设备/AVD + API + ABI、build flags、
日期、命令、证据路径。缺字段视为记录不完整。

`timeline.md` 记录：25 判定与冻结 tag/commit → 各轴 commit → 每条证据日期 →
合入 commit。目录 append-only，不改写 `evidence/ticket-25/`。

### 5. 合入门槛与精确 SHA 规则

合入 `integration/wire-protocol-v2-beta` 前必须同时满足：

1. JVM 测试全绿；
2. `EMULATOR` 矩阵全部 `PASS`（或维护者 `WAIVED`）；
3. 真机四项按第 3 节；
4. `manifest.tsv` 完整，无未关闭 `FAIL`；
5. 证据绑定**将要合入的 refactor tip SHA + APK/Server 哈希**。

若证据采集后又出现 Android commit：受影响行必须重跑；影响范围无法界定时按全矩阵重跑。
最终证据在 25 GO + `v0.5.0-beta` 冻结之后采集；冻结前只做开发验证，不产生合入用证据。

CI 扩展不在本票决定（归 09）；本票的门槛按手工命令与现有脚本执行。

### 6. UI / 架构验收场景清单

| # | 场景 | 断言 | 层级 |
|---|---|---|---|
| 1 | `Live` 四阶段 | SESSION 显示对应 phase | JVM |
| 2 | `Reconnecting` | 留 SESSION，状态行含 attempt，Stop 可用 | JVM |
| 3 | `Ended(UserStop)` | 留 SESSION，「you stopped」+ Connect again | JVM |
| 4 | `Ended(Surfaced)` | 留 SESSION，错误卡 + Retry / Pair again | JVM |
| 5 | `Live` 时 Connect | 只导航，不重复 `ConnectStored` | JVM |
| 6 | Pairing FAILED | 留 PAIRING，旧 SAS 不残留 | JVM |
| 7 | ABOUT / SETTINGS | 不被会话事件拉走 | JVM |
| 8 | outcome store | 恰好一次 Ended、注册重放、旧代际丢弃 | JVM |
| 9 | 播放中旋转 | SESSION 恢复、播放不断、无第二条连接 | Robolectric + `EMULATOR` |
| 10 | 配对中旋转 | PAIRING + SAS/快照恢复；相机不自动开 | Robolectric + `EMULATOR` |
| 11 | 进程死亡 | 落 HOME、不自动连接、配对不续接 | `EMULATOR` |
| 12 | 活会话 rebind | 服务端连接数不增加 | `EMULATOR` + server log |
| 13 | Back 语义 | PAIRING 取消；SESSION→HOME 继续；HOME finish 发 701 | JVM + `EMULATOR` |
| 14 | 连按两次 Back | 停止会话（现有契约） | `EMULATOR` |
| 15 | 配对完成 | 只自动连接一次 | JVM + `EMULATOR` |
| 16 | 配对失败后重试 | 重新进入 PAIRING，不残留旧状态 | JVM |
| 17 | 重建后的错误卡 | 由 owner replay 恢复，不靠本地持久化 | JVM |

以上每条在 `manifest.tsv` 中有对应行；07 的 driver 时序测试并入 JVM 层。

### 对后续工单的约束

- 09：CI 若要自动化本矩阵，只能强化这些门槛，不能替代真机项。
- 10：UI 验收按第 6 节清单补齐 SETTINGS 的空态/内容断言。
- 12：`AudioShareAppTest`、lifecycle instrumentation、`service-device-test.py` 的
  保留/重写纳入资产处置；本票只定义验收场景。

### 证据

本票为决策票，未改动代码，故无 PASS/FAIL；事实来源为上列文件与既有证据目录。
未建分支、未 commit。
