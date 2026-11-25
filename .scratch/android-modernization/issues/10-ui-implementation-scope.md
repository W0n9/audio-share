# 10: UI 实现范围：屏幕清单、M3 组件与 a11y/i18n

Labels: wayfinder:grilling
Type: grilling
Status: resolved
Blocked by: 01, 04, 05, 06

Preconditions from [06](06-navigation-and-state-restoration.md#answer)：`AppScreen` 增加
`SETTINGS`；屏幕与状态必须符合 06 的 outcome→目的地转移表；SETTINGS 的行项目与空态由本票决定。

## Question

基于「现代 UI 方向」的视觉决策与 01 / 06 的架构，决定 UI 实现的 must-have：

1. **屏幕与状态**：配对（地址 / QR / SAS / 失败 / 重连）、会话（连接中 / 播放 / 已结束）、设置、关于；每屏的 must-have 状态与空态。
2. **M3 组件与主题**：Material 3 组件映射、主题 token、动态取色是否启用、System / Light / Dark。
3. **a11y 与 i18n**：内容描述、触控目标、对比度、中英文案（en-US / zh-CN），以及与现有 `values-zh-rCN` 的关系。
4. **状态恢复验收**：哪些屏幕与状态必须在进程死亡 / Activity 重建后恢复；哪些重新建立。
5. **延后清单**：明确哪些屏幕、动画、功能不在本轮。

新功能不预先禁止，但任何进入本票的功能必须明确写出，并确认不改变 beta 验收范围。产出是屏幕清单、组件映射与验收场景，不是实现。

## Answer

**结论（用户逐条确认）：按方向 A 的屏幕/状态清单实现；静态 M3 主题跟随系统、不做动态取色与
用户主题设置；本轮完成 en + zh-CN 文案资源化；SETTINGS 只做只读信息页；a11y 按固定清单
验收；明确延后项。**

### 1. 屏幕与状态（must-have + 空态）

| 屏幕 | must-have 状态 | 空态 |
|---|---|---|
| **HOME** | 无凭据：地址输入 + Pair 入口；有凭据：Server 卡 + Connect；地址校验错误；相机权限被拒提示 | 无凭据时隐藏 Server 卡与 Connect |
| **PAIRING** | 未开始（QR / 粘贴链接 / SAS 入口）；扫码中；STARTING；AWAITING_CONFIRMATION（SAS 模式需输入，QR 模式一步确认且 SAS 可见）；FAILED（错误码 + 指引 + Try again）；COMPLETE | 未开始时的三入口 |
| **SESSION** | Live 四阶段；Reconnecting(attempt, lastCode) + Stop；Ended(UserStop) + Connect again；Ended(Surfaced) + 错误卡 + Retry / Pair again | 无会话不进入此屏（06：落 HOME） |
| **SETTINGS** | 已配对 Server 只读信息（名称/地址/指纹）、版本、About 链接 | 无配对：说明 + 「Pair a phone」 |
| **ABOUT** | 版本、已知限制、v1 警告、更新说明、隐私 | — |

- 每屏的主操作只有一个；错误与重连内联，不用对话框。
- SETTINGS 不提供开关、删除、重命名；这些都属于新功能，本票不引入。

### 2. M3 组件与主题

- **主题**：`MaterialTheme` + `lightColorScheme()` / `darkColorScheme()`，通过
  `isSystemInDarkTheme()` 跟随系统；**不做动态取色**（API 31+），**不做用户主题设置**。
- 默认 M3 typography；不引入自定义字体或品牌皮肤。
- 组件映射：`Scaffold` + 小型 `TopAppBar`、`Card` / `OutlinedCard`、
  `Button` / `OutlinedButton` / `TextButton`、`OutlinedTextField`、
  `LinearProgressIndicator`（重连/连接中）、`HorizontalDivider`。
- Hero 用静态 `Canvas` 脉冲圈，不做无限动画；不引入自定义转场。
- 图标优先 `material-icons-core`；缺图标用 vector drawable，不引入整套
  `material-icons-extended`。若需新增 `material-icons-core` 依赖，在 09 的版本目录中登记。
- 边到边与 inset：targetSdk 36 阶段（09 phase 3）用 `Scaffold` 的 inset + 安全绘制区，
  不裁切状态栏/导航栏；横幅与主操作不得被手势条遮挡。

### 3. a11y must-have

1. 所有 icon-only 按钮有 `contentDescription`（Settings / About / Camera close 等）。
2. SAS 逐位朗读：`contentDescription = "4 1 8 9 0 2"`，避免被读成一个大数。
3. outcome / error 文本加 `liveRegion = Assertive`，状态变化由 TalkBack 播报。
4. 触控目标 ≥ 48dp；按钮高度 ≥ 48dp。
5. 200% 字体缩放不裁切、不重叠；避免固定高度包裹多行文案。
6. 焦点顺序：标题 → 状态/错误 → 主操作 → 次操作。
7. 只用主题色角色（`onSurfaceVariant`、`errorContainer` 等），不自造低对比度颜色。
8. QR 相机页：权限说明、关闭入口、扫码区域的可理解描述。
9. 使用 `start/end` padding 支持 RTL。
10. 不引入第三方 a11y 扫描依赖；按 08 的场景手工/Compose 断言验收。

### 4. i18n

- 新增 `res/values/strings.xml`（en 默认）与 `res/values-zh-rCN/strings.xml`；所有用户可见
  文案从 Kotlin 硬编码迁到资源（UI、通知、错误指引、空态）。
- 术语沿用 v1：设置 / 关于 / 版本 / 正在连接；新增：配对、配对码、服务器地址、指纹、
  重连、会话。
- 跟随系统语言；**不做应用内语言选择器**。
- 领域层不翻译：`ErrorGuidance` 保留 `code` 与 `retryable` / `requiresRepairing` /
  `automaticRetry` 标志；UI 按 `code` 映射到 `error_<code>_title` /
  `error_<code>_guidance` 资源，不再直接渲染 Kotlin 里的英文标题/指引。
- 重连次数用 `plurals`（attempt 1 vs N）；SAS、地址、版本号、错误码保持语言中立。
- 通知渠道名与正文同样资源化。

### 5. 状态恢复验收（与 06 对齐）

- HOME：`address` 恢复；配置变更不触发 auto-connect。
- PAIRING：`sasInput` 恢复；SAS/指纹/phase 从 Service 回放；相机不自动重开。
- SESSION：outcome 从 Service 回放；有活会话时留在 SESSION；进程死亡落 HOME。
- SETTINGS / ABOUT：屏幕恢复；数据来自 Service / 偏好；不被会话事件拉走。
- 对应 08 场景：#9 旋转播放、#10 旋转配对、#11 进程死亡、#17 错误卡回放。

### 6. 延后清单

- 动态取色、用户主题设置、Settings 开关；
- 暂停/恢复；
- 多 Server 管理（删除/重命名/排序）；
- 横屏/平板专用布局；
- 自定义动画与转场；
- 相册选 QR、手电筒；
- 通知内控制扩展；
- 应用内语言选择器。

以上延后项不进入 fog、不开工单；需要时作为独立需求评估。

### 7. 新功能声明

本票不引入新功能。深色主题是 04 的显示要求，不是用户可配置项；中文是本地化，
不改变 beta 验收范围；SETTINGS 只读。任何开关、删除配对、暂停等都不在本轮。

### 对后续工单的约束

- 12：`AudioShareAppTest` 中依赖硬编码文案的断言必须改为资源/语义断言；截图基线要按
  方向 A 与 zh-CN 重拍；`contentDescription` 与 liveRegion 的断言纳入资产处置。
- 08：场景 #1–#17 的 UI 断言按本票的屏幕/状态清单落位；新增字体缩放与 RTL 检查为
  补充项，不替代真机关键路径。
- 09：若新增 `material-icons-core`，在版本目录与 CI 依赖检查中登记；资源收缩阶段确认
  zh-CN 字符串未被 `shrinkResources` 误删。

### 证据

本票为决策票，未改动代码，故无 PASS/FAIL；事实来源为上列文件（`AudioShareTheme.kt`、
`AudioShareApp.kt`、`PairingErrorGuidance.kt`、v1 `values-zh-rCN`）。
未建分支、未 commit。
