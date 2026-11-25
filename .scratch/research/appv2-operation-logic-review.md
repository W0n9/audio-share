# appv2 操作逻辑复审（integration/v0.6.3-gui-network）

## 调研基线

- **分支**：`integration/v0.6.3-gui-network` @ `d6d1879`（2026-11-24）。所有行号与符号均指该分支；引用格式为 `integration/v0.6.3-gui-network:<path>:<line>`。
- **日期**：2026-11-24。
- **范围**：`android-client-v2/app/src/main/java/dev/audioshare/v2/` 的状态机、导航、配对/恢复、播放链路；设计基线为 `.scratch/pairing-ux/spec.md` + issues 01–05、`docs/adr/0006-revocation-and-repairing.md`、`GLOSSARY.md`、根 `README.md`，其中 pairing-ux 文档来自本分支祖先 `ddeb939`（已用 `git merge-base --is-ancestor` 确认）。`.scratch/v0.6.3-gui-network/` 的 Host combobox / 保存回滚 / 确认模态框票据属于 **Server GUI**，不是 appv2；唯一与 appv2 有关的是 #04 让 QR 的 `addr` 使用配置的 Host（`.scratch/v0.6.3-gui-network/issues/04-qr-advertise-configured-host.md:5,24-31`），因此手机端“按地址选 Server”更容易遇到多地址/改地址的情形，见问题 A2。
- **方法**：只读 `git show` / `git grep`；未切分支、未改工作树。未跑测试或真机，本文所有结论均为源代码与设计文档对照，标注 `待确认` 的项未做运行时验证。

## 结论摘要（Top 3）

1. **【代码/状态机】在 PAIRING 页按系统 Back、但还没有开始任何配对交换，会静默杀掉正在播放的会话**：`BackPressed` 无条件走到 `cancelPairing` 并发出 `AppCommand.CancelPairing`，Service 将其映射为 `stopSession(PairingCancel)`；由于 outcome 被 invalidate，用户既没有 Ended 卡片也没有声音，而同页的“Back to Home”按钮却不会停止会话。见 A1。
2. **【代码/状态机】“当前 Server”由用户输入的地址字段猜测，而不是会话真实持有的 pin**：Service 内部有 `sessionServerPin`，但没有通过 `SessionController`/`SessionOutcome` 暴露；UI 用 `selectedServer()`（地址匹配 + `firstOrNull()` 回落）决定修复标记、Forget 归属、Retry 目标，同时 Home/Settings 卡片显示 `storedServers.firstOrNull()`。卡片、按钮与实际会话可能指向不同记录，逐字符保存 `lastAddress` 会放大该问题。见 A2。
3. **【交互/播放链路】没有音频焦点与“拔出耳机”处理，重连也没有尝试次数上限**：全仓库 appv2 源码未出现 `requestAudioFocus` / `AudioManager` / `BECOMING_NOISY`；`ReconnectPolicy` 只限制退避延迟（1s→30s），不限制次数，用户可长期停留在“Reconnecting attempt N”。见 B4、B5（次数上限是否符合 `wire-protocol-v2` §6.3 标 **待确认**）。

---

## A. 代码 bug / 状态机缺陷

### A1. 空配对上下文下按 Back 会终止实时会话，且与“Back to Home”行为不一致

- **【现象】**用户在 SESSION 播放中按 Back 回到 HOME（会话继续，`AppState.kt:333-344`），点 Home 的 `Re-pair` 进入 PAIRING（`AudioShareApp.kt:307-320`；`rePair` 只做导航、清 `pairing`，见 `AppState.kt:783-800`），此时还没扫任何码、也没选 SAS；按系统返回键会直接结束播放，屏幕回到 HOME，状态栏/错误卡不显示任何“已停止”提示。相比之下点击页面内 `Back to Home` 只发 `ShowHome`，不会停止会话（`AudioShareApp.kt:825-832`）。
- **【证据】**
  - `integration/v0.6.3-gui-network:android-client-v2/app/src/main/java/dev/audioshare/v2/AppState.kt:329-347`：`BackPressed` 在 PAIRING 无条件 `cancelPairing(state)`。
  - 同文件 `:534-550`：`cancelPairing` 把 `outcome` 置 null 且**总是**返回 `AppCommand.CancelPairing`。
  - `integration/v0.6.3-gui-network:android-client-v2/app/src/main/java/dev/audioshare/v2/ControlSessionService.kt:218-220`：`ACTION_CANCEL_PAIRING -> stopSession(StopReason.PairingCancel)`；`:492-499` 的 `stopSession` 会 `stopForeground` + `stopSelf`。
  - `integration/v0.6.3-gui-network:android-client-v2/app/src/main/java/dev/audioshare/v2/session/SessionOrchestrator.kt:134-152`：非 `User` 的 teardown 调用 `outcomes.invalidate()`；`integration/v0.6.3-gui-network:android-client-v2/app/src/main/java/dev/audioshare/v2/SessionOutcome.kt:102-115`：`invalidate` 丢弃可重放的 outcome 且不向监听者推送合成事件。
  - 测试仅覆盖“已开始 SAS 后取消”（`.../src/test/java/dev/audioshare/v2/AppStateReducerTest.kt:539-545`），没有覆盖 `pairing == null` 且 `outcome == Live` 的 Back；两条配对启动路径会主动 supersede 当前会话（`AppState.kt:606-610,643-645`），但 `RePairRequested` 不会（`AppState.kt:461-464,783-800`）。
- **【影响】**用户只是“返回上一页”，却丢失正在播放的音频；因为 outcome 被 invalidate，无法从 UI 判断发生了什么，Home 仍显示 PAIRED + Connect（`AudioShareApp.kt:296-305`），再次 Connect 才是唯一恢复路径。
- **【建议方向】**`AppState.BackPressed` 仅在 `state.pairing != null`（确有交换在进行）时才发 `CancelPairing`；否则走 `ShowHome` 的纯导航。另一个更保守的方案：从 Live 会话进入 PAIRING 前给出“开始配对会停止当前播放”的提示；并补一条 JVM 测试覆盖 `pairing=null + Live + BackPressed`。
- **【优先级】**高（用户可见的数据/播放中断，且存在明显的不一致出口）。

### A2. 会话归属用“地址字段猜测”而非真实 pin，UI 与 Service 的当前 Server 可能不一致

- **【现象】**Service 明确知道当前会话连的是哪个 Server（`sessionServerPin`），但该信息没有跨越 Binder 边界；UI 用 `selectedServer()` 猜测：把输入框地址按 `host:port` 严格匹配记录，匹配不到就取 `storedServers.firstOrNull()`。这个猜测同时驱动 Home 的 Connect 是否禁用、204 修复标记贴到哪条记录、Forget 是否清 outcome、Retry 进入哪个重配上下文，以及冷启动自动连接的 `needsRepairing` 判定；而 Home/Settings 的 Server 卡片又显示 `storedServers.firstOrNull()`，与按钮目标可以不是同一条。
- **【证据】**
  - Service 侧真实归属：`integration/v0.6.3-gui-network:android-client-v2/app/src/main/java/dev/audioshare/v2/session/SessionOrchestrator.kt:67-74`（`sessionServerPin`）、`:93-97`（每个 session 设置）、`:167-177`（`stopIfOwnedBy` 用真实 pin 判断）。
  - 接口不暴露：`integration/v0.6.3-gui-network:android-client-v2/app/src/main/java/dev/audioshare/v2/ControlSessionService.kt:29-75`（`SessionController` 只有 outcome/pairing/credential 操作）；`.../dev/audioshare/v2/SessionOutcome.kt:8-17`（`SessionOutcome` 无 Server 身份）。
  - UI 猜测：`.../dev/audioshare/v2/AppState.kt:152-166`（`selectedServer` / `selectedPin`）、`:294-300`（AutoConnect 闸门）、`:813-820`（`projectedRepairState`）、`:731`（`forgettingSessionOwner`）、`:763`（`retry`）；`.../dev/audioshare/v2/MainActivity.kt:152-158`（冷启动读取选中记录的 flag）。
  - 卡片与按钮不同源：`.../dev/audioshare/v2/AudioShareApp.kt:233`（卡片 `firstOrNull()`）对比 `:299-320`（Connect/Re-pair 用 `selectedServer()`）、`:1070`（Settings 显示第一条、`:1116-1123` Forget 也是第一条）；SESSION 显示的是输入框的 `state.address` 而非已连接 Server（`:893-900`）。
  - 放大器：`.../dev/audioshare/v2/MainActivity.kt:195-200` 对每个合法地址输入都 `prefs.saveLastAddress`，部分/临时输入会立刻成为下次冷启动的默认 Server（`AppState.kt:401-415`）。
  - 已知边界：`integration/v0.6.3-gui-network:.scratch/pairing-ux/evidence/README.md:121-123` 明确把“Home/Settings 显示第一条而非 `selectedServer()`”记为多 Server 选择器范围外。当前单 Server 配置下影响有限，但通过 SAS 对不同地址配对可产生多条记录；v0.6.3 的 QR `addr` 改用配置的 Host 后，地址变化/多地址场景更常见（详见调研基线）。
- **【影响】**可能出现：卡片显示“PAIRED / Needs re-pairing”的 A 记录，而 Connect/Re-pair/Forget 实际作用于 B 记录；204 的修复标记贴到 B，A 继续显示正常；Forget 非会话记录时 UI 保留 outcome 的判断依据也是这个猜测。多记录场景下错误会成组出现。
- **【建议方向】**在 `SessionOutcome` 或 Binder 接口上附带当前 `sessionServerPin`（会话身份），UI 的修复标记、Forget 归属、Retry、SESSION 地址展示都从该身份派生；“选哪个 Server 连接”与“当前连的是谁”拆成两个状态。短期的低成本缓解：Home/Settings 卡片改用同一个 `selectedServer()`，并把 `saveLastAddress` 从逐字符写入改为在 Connect / 配对成功 / 明确提交时写入。
- **【优先级】**高（结构性状态一致性缺陷；单 Server 时风险低，但涉及安全相关的修复/撤销语义）。

### A3. 205 “无凭据”路径不持久化 needs-repairing，且文案与场景不符

- **【现象】**冷启动自动连接时若根本没有凭据，`connectStoredNow` 调用 `reportStoredConnectFailure(205, ...)` 但不传 pin，因此不会像“密钥丢失”路径那样置位 `needsRepairing`；同时 205 的标题/错误文案是 `SERVER_IDENTITY_MISMATCH`，实际场景是“无保存的配对 / 密钥丢失”。
- **【证据】**
  - `integration/v0.6.3-gui-network:android-client-v2/app/src/main/java/dev/audioshare/v2/ControlSessionService.kt:336-345`（无凭据：`reportStoredConnectFailure(code=205, ...)` 无 pin）对比 `:346-358`（密钥丢失：传 `record.sessionServerPin`）。
  - `.../dev/audioshare/v2/session/SessionOrchestrator.kt:217-228`：只有传 pin 才会 `markNeedsRepairing`；`:253-257` 仅对 `requiresRepairing` 的 code 置位。
  - `.../dev/audioshare/v2/PairingErrorGuidance.kt:44-47`：205 = `SERVER_IDENTITY_MISMATCH`，`requiresRepairing=true`；`.../dev/audioshare/v2/ErrorGuidanceResources.kt:20,45` 使用 `error_205_*` 资源。
  - `integration/v0.6.3-gui-network:.scratch/pairing-ux/spec.md:35-38`（需要重配状态必须持久化、自动连接据此停止）。
- **【影响】**错误卡的主按钮是 `Re-pair`（`AudioShareApp.kt:1294-1306`），但“无凭据”本身已无记录可标记，下次启动仍会尝试一次自动连接并有相当概率再次出现同一张卡；把“密钥丢失”显示成“Server 身份不匹配”会误导用户去怀疑服务器而不是本机数据。
- **【建议方向】**为“无凭据”与“密钥丢失”分别定义 code 或至少分别定义文案（沿用 `ErrorGuidanceBook` 的可扩展结构）；对“密钥丢失”保留置位逻辑，对“无凭据”明确把空列表作为稳定的未配对状态，不重复触发 205。
- **【优先级】**中。

### A4. Forget 的存储失败只在 Service 日志里，UI 已先宣告成功

- **【现象】**`ForgetServerConfirmed` 一到达，reducer 就从列表删除记录、清空地址、回到 HOME；Service 侧的 credential 删除若抛异常，只记录 `Log.e` 并继续停止会话/关闭 Service，没有任何事件回传给 UI。
- **【证据】**
  - `integration/v0.6.3-gui-network:android-client-v2/app/src/main/java/dev/audioshare/v2/AppState.kt:722-755`：确认事件无条件删除 UI 记录并回到未配对 HOME。
  - `.../dev/audioshare/v2/ControlSessionService.kt:371-399`：`forgetter.forget` 的异常被吞掉（`:374-381`），后续照常 `stopIfOwnedBy` / `stopSelf`。
  - `.../dev/audioshare/v2/ForgetServer.kt:13-15,23-31`：注释声明 credential 删除失败会“propagates to the caller”，但调用方 Service 没有把这个结果传回 UI。
- **【影响】**极低概率下（app-private SharedPreferences 写失败）出现“界面显示已忘记、凭据仍在磁盘”的错觉；下次冷启动记录又出现，用户对 Forget 的不可逆承诺失去信任。
- **【建议方向】**把 Forget 做成“确认 → Service 删除 → 成功事件后 UI 更新”，或至少在失败时通过 outcome/事件显示错误条。需要权衡 `ForgetServer.kt:13-15` 的“删除结果优先于密钥删除失败”既定语义。
- **【优先级】**低（失败路径罕见，且与 GLOSSARY “Forget Server 不可静默删除”的语义相关：`integration/v0.6.3-gui-network:GLOSSARY.md:103-105`）。

---

## B. 交互设计可优化（非状态机缺陷）

### B1. 配对入口重复且层级混乱，与定稿文案不完全一致

- **【现象】**未配对 HOME 同时提供 `Pair with code` 与 `Scan QR code`（都是完整配对入口）；PAIRING 页又提供 `Scan QR code`、粘贴链接、`Pair with a code` 三个入口；失败态还有 `Try again` / `Scan new code`。定稿文案表要求 Home 已配对次要动作统一为 `Re-pair`，替代 `Pair a new phone` 与独立 `Scan QR instead`，但**未配对** Home 仍保留两个入口。
- **【证据】**`integration/v0.6.3-gui-network:android-client-v2/app/src/main/java/dev/audioshare/v2/AudioShareApp.kt:321-335`、`:440-476`、`:757-782`；`integration/v0.6.3-gui-network:.scratch/pairing-ux/spec.md:103-116`；`integration/v0.6.3-gui-network:android-client-v2/app/src/main/res/values/strings.xml:22-26,161-169`。
- **【影响】**用户不清楚“Home 的 Pair with code”和“PAIRING 页的 Pair with a code”有什么区别；失败恢复时更容易退回通用页而不是沿用重配上下文。
- **【建议方向】**把未配对 Home 收敛为一个主入口（QR scan）+ 一个明确的文字降级入口（配对码）；粘贴链接只在 QR 入口内作为次级选项出现一次，避免在两屏重复。
- **【优先级】**中。

### B2. SESSION 展示的是“输入框地址”，不是当前连接的 Server

- **【现象】**Session 屏幕的 `Server: %1$s` 直接显示 `state.address`，用户可以从 Home 改这个字段而不影响正在运行的会话，显示值因此与真实连接对象分离。
- **【证据】**`integration/v0.6.3-gui-network:android-client-v2/app/src/main/java/dev/audioshare/v2/AudioShareApp.kt:893-900`（`session_server_value`）；`.../dev/audioshare/v2/AppState.kt:281-286`（`AddressChanged` 只改状态，不触碰会话）；A2 中的 `SessionOutcome` 无 pin 证据。
- **【影响】**排障时误导用户以为连到了刚输入的地址。
- **【建议方向】**与 A2 一并解决：展示会话真实 Server（pin 对应的记录/地址），输入字段只作为“下一次连接目标”。
- **【优先级】**中。

### B3. 没有音频焦点 / 拔耳机（BECOMING_NOISY）处理

- **【现象】**appv2 源码中不存在 `requestAudioFocus`、`AudioManager`、`ACTION_HEADSET_PLUG` 或 `BECOMING_NOISY` 相关处理；播放线程只按 native ring 的数据与内部 underrun 门控运行。
- **【证据】**`git grep -n -E 'requestAudioFocus|AudioManager|BECOMING_NOISY|ACTION_HEADSET_PLUG' integration/v0.6.3-gui-network -- android-client-v2/app/src/main/` 无结果；`integration/v0.6.3-gui-network:android-client-v2/app/src/main/java/dev/audioshare/v2/MediaPlayer.kt:462-483`（纯播放线程启停）、`:703-871`（播放循环只读 native 状态与 ring）；`.../dev/audioshare/v2/session/SessionLifecycleDriver.kt:209-219`（会话创建即 start media）。
- **【影响】**拔出耳机后继续从扬声器外放（隐私/打扰），来电或其他媒体应用无法触发让出焦点；与系统通知栏 Stop 按钮（`ControlSessionService.kt:435-459`）之外没有系统级的播放控制约定。
- **【建议方向】**按 Android 媒体应用惯例接入 `AudioFocusRequest`（AUDIOFOCUS_GAIN）与 `ACTION_BECOMING_NOISY`：短暂失去焦点暂停并停媒体线程、可恢复时重启；永久失去焦点或拔耳机时按 `StopReason.User` 走同一 Stop 语义，保持 UI 卡片一致。
- **【优先级】**中高（真机体验与隐私影响；需真机复验，标 **待确认**）。

### B4. 重连只有退避上限，没有次数上限，UI 无退出引导

- **【现象】**连接失败/传输丢失后进入 bounded backoff，但 `ReconnectPolicy` 只把**延迟**收敛到 30s，重试次数没有封顶；UI 显示 `Reconnecting attempt N`，Stop 按钮可用，但没有“重试次数过多、建议检查网络/重新配对”的终态。
- **【证据】**`integration/v0.6.3-gui-network:android-client-v2/app/src/main/java/dev/audioshare/v2/ClientSession.kt:342-368`（`backoffDelayMs` 上限 30_000，`BACKOFF_INITIAL_MS=1_000`，无 attempt cap）；`.../dev/audioshare/v2/session/SessionLifecycleDriver.kt:286-311`（`scheduleReconnect` 自增 attempt，持续 postDelayed）；`.../dev/audioshare/v2/AudioShareApp.kt:961-986`（attempt 展示）；`.../dev/audioshare/v2/PairingErrorGuidance.kt:88-95`（601/602 可自动重试）。
- **【影响】**网络不可达或服务端已消失时长时间停留在 Reconnecting，用户可能不知道可以 Stop/重新配对。
- **【建议方向】**确认 wire-protocol-v2 §6.3 是否允许无限重试（**待确认**）；若允许，至少在 UI 上给 N 次后提示“检查网络或重新配对”并保留 Stop/Re-pair 入口；若不允许，增加总次数或总时长上限并转为可操作的 `Ended(Surfaced)` 卡片。
- **【优先级】**中。

### B5. 配对进行中离开 PAIRING 页不会取消交换，也没有“配对进行中”提示

- **【现象】**扫码等待服务端批准时点 `Back to Home` 或切到 Settings/About，配对交换仍在 Service 里运行（含前台通知），Home 上没有“正在配对”的状态提示；用户看起来像回到了空闲态。
- **【证据】**`integration/v0.6.3-gui-network:android-client-v2/app/src/main/java/dev/audioshare/v2/AudioShareApp.kt:825-832`（`ShowHome`）；`.../dev/audioshare/v2/AppState.kt:451-459,466-482,521-531`（导航事件都不发 CancelPairing）；`.../dev/audioshare/v2/ControlSessionService.kt:196-205`（配对 action 会 `startForegroundCompat`）；`NavigationRestoration` 只在重绑时把 PAIRING hint 与快照对账（`.../dev/audioshare/v2/Navigation.kt:17-44`）。
- **【影响】**用户可能再开一次新配对（supersede 旧交换）、或疑惑通知为什么还在；和 A1 的“Back 会取消”形成第三种不同语义。
- **【建议方向】**给导航出口统一定义：要么离开 PAIRING 一律 CancelPairing，要么保留交换但在 Home/Settings 显示“配对进行中 + 取消”入口；后一次 `BeginSas`/`BeginQr` 明确提示会取代前一次。
- **【优先级】**中。

### B6. QR 自动确认的“双重保险”增加理解与测试成本

- **【现象】**UI reducer 在 QR 的 `AWAITING_CONFIRMATION` 自动发一次 `ConfirmPairing`，若 Service 拒绝则累计 40 个快照后重发；Service 侧又有独立的 `PairingConfirmGate` 做每次交换一次的门控。两层 guards 解决了 Activity 重建与拒绝重发两个不同问题，但使“确认到底发了几次”难以从单一层推断。
- **【证据】**`integration/v0.6.3-gui-network:android-client-v2/app/src/main/java/dev/audioshare/v2/AppState.kt:47-60,278,688-707`; `.../dev/audioshare/v2/ControlSessionService.kt:536-556`; `.../dev/audioshare/v2/session/SessionOrchestrator.kt:72,179-182`; 对应测试 `.../src/test/java/dev/audioshare/v2/AppStateReducerTest.kt:394-413`、`.../src/test/java/dev/audioshare/v2/PairingConfirmGateTest.kt`、`.../src/test/java/dev/audioshare/v2/session/SessionOrchestratorTest.kt:145-172`。
- **【影响】**不是用户可见 bug；但重复状态会在排查“QR 卡在 AWAITING”时增加歧义（A1、A2 的修复也会触及该区域）。
- **【建议方向】**长期可把“重发条件”下沉到 Service 的单一 gate（带 attempt 计数与拒绝反馈），UI 只渲染快照；短期不必动，优先级低于 A1/A2。
- **【优先级】**低。

---

## C. 设计已覆盖但未验证 / 实现与设计偏差

### C1. Forget 入口只在 Settings，设计文档允许 Server 卡片也有

- **【现象】**`pairing-ux/spec.md` 的“忘记此 Server”目标流程写“Server 卡片或 Settings 提供”，但 appv2 的 Home Server 卡片只有 PAIRED / Needs re-pairing 徽标与 Connect/Re-pair，Forget 只出现在 Settings 页面。
- **【证据】**`integration/v0.6.3-gui-network:.scratch/pairing-ux/spec.md:69-72`；`integration/v0.6.3-gui-network:android-client-v2/app/src/main/java/dev/audioshare/v2/AudioShareApp.kt:235-272,307-320,1116-1123`；ticket 03 的验收只写“Home 的 Server 卡片显示 Needs re-pairing”（`.scratch/pairing-ux/issues/03-android-repair-state.md:15-16`）。
- **【影响】**用户从“Needs re-pairing”卡片想彻底断开时，找不到 Forget，必须知道去 Settings。
- **【建议方向】**若维持单入口，更新设计文档并考虑在 Needs re-pairing 卡片的次级动作中给“Forget this Server”；否则在卡片补入口，复用已有确认对话框（`AudioShareApp.kt:124-156`）。
- **【优先级】**低—中。

### C2. 201/206 阻断态对 SAS 的恢复动作仍指向通用配对页

- **【现象】**`PairingUi.isBlockingInvitationFailure` 不区分模式，但只有 QR 渲染“邀请失效 → Scan new code”的主按钮；SAS 走 `Try again → ShowPairing`，回到通用入口列表，虽然提示文案已在卡片里出现。
- **【证据】**`integration/v0.6.3-gui-network:android-client-v2/app/src/main/java/dev/audioshare/v2/AppState.kt:71-81`；`.../dev/audioshare/v2/AudioShareApp.kt:740-782`；`integration/v0.6.3-gui-network:.scratch/pairing-ux/spec.md:74-77`（失败恢复要求“服务端刷新主导，手机阻断态”）与 ticket 04 验收（`:16`）；`.../res/values/strings.xml:189-192`。
- **【影响】**SAS 用户比 QR 用户少一步明确引导（先让服务端刷新/新建配对），更容易重复一次失败的尝试。
- **【建议方向】**SAS 阻断态也提供专用恢复动作：“请在服务端重新开始配对”，并直接回到当前 `PairingIntent` 上下文（RetPair 不能退化成 First）。
- **【优先级】**低。

### C3. SAS 重配的“同一身份”依赖地址命中，服务端地址变更时会新增记录

- **【现象】**ADR-0006 §2 的目标是“同一手机重配保持同一身份”；QR 按 pin 命中已存记录（`PairingRepair.kt:28-31`），SAS 按**用户输入的地址**命中（`PairingRepair.kt:12-18`）。如果 Server 地址变了（v0.6.3 之后 QR 广告地址来自配置 Host，变更更可能），用户用新地址 + SAS 重配会生成新 alias，服务端出现第二条设备记录，而不是 Restore 旧记录。
- **【证据】**`integration/v0.6.3-gui-network:docs/adr/0006-revocation-and-repairing.md:24-26`；`.../dev/audioshare/v2/PairingRepair.kt:12-18,20-31`；`.../dev/audioshare/v2/ControlSessionService.kt:301-306,268-273`；`integration/v0.6.3-gui-network:.scratch/pairing-ux/spec.md:35-36,64-67`。
- **【影响】**撤销后的 SAS 重配在地址变更场景下违背“不产生第二条记录”的设计；用户需要在服务端手动 Delete 旧记录。
- **【建议方向】**为 SAS 也提供基于已存记录（例如从 `PairingIntent.RePair` 携带的 pin）的别名复用路径，而不是只靠地址；若产品上接受“地址变更=新 Server”，需在 ADR/文案中明确。具体取舍 **待确认**。
- **【优先级】**低—中。

### C4. 同地址清理只删凭据、不删身份密钥（已知 gap）

- **【现象】**配对成功时 `removeStaleRecordsForAddress` 只调用 `credentials.delete`，旧记录的 Keystore/软件密钥文件保留，而 Forget 路径会删除密钥。
- **【证据】**`integration/v0.6.3-gui-network:android-client-v2/app/src/main/java/dev/audioshare/v2/session/SessionOrchestrator.kt:276-299`；`integration/v0.6.3-gui-network:.scratch/pairing-ux/evidence/README.md:99-105,116-120`（明确标为已知 gap，非本轮 review finding）。
- **影响**：本地密钥材料残留，属于清理泄露而非功能故障。
- **建议**：与 Forget 共用“删除凭据 + 密钥”的单一函数，并保持“删除失败不破坏新凭据”的现有语义（`:277-279`）。
- **优先级**：低。

### C5. 设计已实现但设备侧验证不足（不是 bug，是验收债）

- **【现象】**配对恢复语义（Restore 需显式确认、Re-pair 保留身份、needs-repairing 闸门、Forget 二次确认、201/206 阻断态）在代码与 JVM 测试中均有对应实现，但完整的手机端操作清单在审核修复后从未在模拟器/真机复跑。
- **【证据】**`integration/v0.6.3-gui-network:.scratch/pairing-ux/issues/05-closeout.md:14,26-29`；`.../.scratch/pairing-ux/spec.md:137-143`；`.../.scratch/pairing-ux/evidence/README.md:82-97`（修复后 real device、emulator、pairQr、QR 重配、Forget UI 全部 `NOT RUN`）；ticket 03/04 的 review fixes 与 332 JVM tests PASS（`.scratch/pairing-ux/issues/03-android-repair-state.md:29-40`、`:04-android-repair-flow.md:28-46`）。
- **【影响】**本文 A1/A2/A3 所涉路径大多能用 JVM reducer/orchestrator 测试覆盖，但真实交互路径（系统 Back、通知栏 Stop、音频焦点、二维码扫码重配、Forget UI）没有设备证据，不能把设计 PASS 当成操作逻辑 PASS。
- **【建议方向】**按证据政策补一轮真机清单：首次配对 → SESSION 播放 → Back 到 Home → Re-pair 页 Back（验证 A1）→ 撤销 → QR/SAS 重配 Restore → Forget；逐项记 `PASS`/`FAIL`/`NOT RUN`。
- **【优先级】**中（尤其 A1 相关的 Back 语义极易在真机上被用户触发）。

---

## 附：边界说明

- 本报告不评价 Server GUI 的 Host combobox、保存回滚、撤销确认框等修复；它们是 v0.6.3 的目标，但不属于 appv2（`.scratch/v0.6.3-gui-network/spec.md:11-27,40-49`）。其中“QR `addr` 使用配置 Host”（issue 04）会改变手机端扫描到的地址，从而与 A2 的地址匹配逻辑相互作用，已在该条中说明。
- 未运行任何命令改变仓库状态；除本文件外没有写入或改动的路径。
