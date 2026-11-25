# 18: UI 外壳：Direction A、主题、Home/About/Settings、en+zh、a11y 基础

**What to build:** 方向 A 的视觉外壳：静态 Material 3 跟随系统 Light/Dark，Home、About、只读 Settings 三屏，全部文案资源化（en + zh-CN），并建立可复用的 a11y 语义基线。

**Blocked by:** 16。

**Status:** resolved

- [x] 主题跟随系统 Light/Dark；不做动态取色、不做用户主题设置。
- [x] Home 覆盖无凭据/已保存 Server/地址错误/相机权限被拒；About 覆盖版本、限制、v1 警告、更新、隐私；Settings 只读（Server 信息、版本、About 链接）并有空态。
- [x] 用户可见文案迁入 `strings.xml` + `values-zh-rCN`；错误指引按 code 映射资源；不提供应用内语言选择器。
- [x] a11y 基线：icon-only `contentDescription`、48dp 触控、200% 字体不裁切、焦点顺序、主题色对比度、RTL padding。
- [x] 通知渠道与正文也本地化；生产包不因 `shrinkResources` 丢 zh-CN。
- [x] Robolectric 测试覆盖三屏状态、资源 key 对齐与语义断言。

## Comments

2026-11-17 — 完成并合入集成分支（merge `d961986`，feature `4961c20`）。
方向 A 外壳（Scaffold + small TopAppBar）、DayNight 主题、Home/About/只读 Settings、
en+zh 各 105 个资源键、错误 code→资源映射、通知本地化、a11y 语义基线。
JVM 210 tests/variant PASS；资源键对齐与 `shrinkResources` 保留 zh-rCN PASS。
证据：`evidence/ticket-18/`。
200% 字体/RTL/Light-Dark 的视觉检查与截图属 `EMULATOR` 项，留给工单 26/27，本票记为 NOT RUN。
