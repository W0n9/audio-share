# 04: 现代 UI 方向：三版可对比原型

Labels: wayfinder:prototype
Type: prototype
Status: resolved
Blocked by: None (can start immediately)

## Question

用 3 个 radically different 的 Compose 原型回答：v2 客户端在三类状态（首次配对 / 失败恢复、会话播放、设置 / 关于）下应该长什么样？

要求：

- Material 3；主题需覆盖 System / Light / Dark。
- 能在现有 `AppState` / outcome 模型上映射，不要求先完成 01。
- 不提前承诺新功能；原型是 throwaway，但保留为资产并从本工单链接。
- 至少有一个变体展示配对失败、重连中、已结束三种 outcome 的视觉差异。

产出：原型路径 + 视觉方向决策，不是实现代码。

## Answer

**方向选定：A「状态优先」（Status-first）。** 由用户在看过三版模拟器截图后选定；
B「底部导航 + 分步向导」和 C「播放器 + 底部弹层」作为对照保留在原型里，不作为落地方向。

### 视觉方向决策

- **每屏一个 hero、一个主操作。** 会话的四种 outcome 都用同一个 hero 位置表达：
  Connecting / Playing（Live）、Reconnecting（attempt + lastCode 直接写在 hero 下方）、
  Ended（UserStop 与 Surfaced 用不同图标、颜色和文案区分）。用户不需要换屏才知道发生了什么。
- **错误与重连在原位呈现，不弹新屏、不跳转。** 配对失败留在配对屏，用 errorContainer
  + 错误码 + 指引 +「Try again / Back to Home」；重连留在会话屏，Session 语义不变，
  只有一个「Stop session」出口。这与工单 01 的 `SessionOutcome` 形状一一对应。
- **设置与关于从顶栏进入，不设 tab bar。** 四类屏幕仍是 Home / Pairing / Session / About，
  另加一个 Settings 目的地（见下方待决项）；沿用现有 `AppScreen` 枚举即可承载，
  不需要为方向 A 引入 Navigation Compose。
- **配对是单页流程，不是 stepper。** SAS 码、输入框、Server 指纹和 Confirm 在同一张卡内；
  QR 与粘贴链接是 Home/Pairing 上的次操作。
- **Material 3 基线，不加品牌皮肤。** System / Light / Dark 全部覆盖（System 已验证跟随
  设备深色模式）。动态取色是否启用留给工单 10，不在本票承诺。
- **不引入新功能。** 原型只展示 beta 已有的状态（地址输入、已保存 Server、SAS/QR 配对、
  Stop、重连、About）。Settings 页面的**形态**（顶栏进入、单列 list）属于方向 A；
  **内容**（放哪些条目、是否成为第五个目的地）交给工单 06 与 10 决定。
  原型里的 Auto-connect 开关、Session notification 开关与 Theme 行只是占位：
  现有 `UiPreferences` 只持久化 `lastAddress()`；auto-connect 是 `AutoConnectPolicy`
  的启动策略而不是用户开关，通知与主题偏好目前不存在。引入其中任何一项都算新功能，
  本票没有批准。

### 原型资产（throwaway）

- 工程：`.scratch/android-modernization/prototypes/ui-direction/`
  - `README.md`：三版说明、离线构建命令、`am start` 的 variant/state/theme extras。
  - `screenshots/`：API 35 全矩阵（3 变体 × 10 状态 × Light/Dark）+ System 深色 1 张
    + API 26 的三张渲染对照。
- 关键证据：
  - `screenshots/A-pairing-failed-light.png`
  - `screenshots/A-reconnecting-light.png`
  - `screenshots/A-ended-user-light.png` 与 `screenshots/A-ended-error-dark.png`（终局差异）
  - `screenshots/A-settings-light.png`
  - `screenshots/B-playing-system-night.png`（System 主题）
- 证据级别：全部 `EMULATOR`（API 35 主矩阵，API 26 验证 minSdk 26 渲染）；真机 `NOT RUN`。
  本票不改 `android-client-v2`、不建分支、不 commit；原型是未跟踪的 scratch 资产。
- 变体 A/B/C 共用同一份 stub 状态（`SessionOutcome = Idle | Live | Reconnecting | Ended`，
  配对 phase 为 `AWAITING_CONFIRMATION` / `FAILED`），因此三版可直接横向比较。

### 对后续工单的约束（不代替它们决策）

- 工单 06：方向 A 下「失败留在 PAIRING、重连/结束留在 SESSION」是导航验收的核心；
  `AppScreen` 枚举足以承载，但仍需决定 Settings 是否成为第五个目的地。
- 工单 10：屏幕清单以 A 的单列结构为基线；B 的 stepper 与 C 的常驻状态条不进入本轮。
- 新的 fog 已毕业为 [工单 12](12-existing-ui-assets-disposition.md)：现有 Compose 测试、截图
  与 beta UI 证据的保留/重写/冻结处置。

