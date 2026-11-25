# 26: 测试资产收口与资源/a11y 对齐

**What to build:** 按 spec 的 keep/rewrite/drop 清单收口测试资产：删除已消失实现的测试，重写依赖旧文案/布局的用例，建立 en/zh 资源对齐与 a11y 语义断言，源码扫描只保留结构/seam 不变量。

**Blocked by:** 25。

**Status:** resolved

- [x] 纯逻辑/协议/播放测试保留；`AudioShareAppTest`、`AppStateReducerTest`、`ErrorGuidanceTest` 已按 Direction A 与 `SessionOutcome` 重写。
- [x] `SessionCommandExecutorTest`、`SessionStateRelayTest` 已删除；对应新测试覆盖 policy/store/handoff。
- [x] 源码扫描只保留 manifest、权限、无 boot receiver、applicationId/版本与 UI-only-SessionController；实现形状与字符串断言改为行为测试。
- [x] en/zh 资源 key 对齐；错误 code→资源映射有测试；a11y semantics（contentDescription、liveRegion、48dp、焦点）有断言。
- [x] `./gradlew test` 全绿；没有测试依赖已删除的字段、类或旧布局。
- [x] `EMULATOR` 完成 zh-CN、200% 字体、TalkBack 手工检查并记录（TalkBack 经维护者 WAIVED，非 PASS）。

## Comments

2026-11-18 — 完成并合入集成分支（merge `c794b65`，feature `8b5566e`）。
测试资产按处置表收口：KEEP 集保留；REWRITE 集由 15–20 完成并移除最后硬编码文案；
`SessionCommandExecutorTest`/`SessionStateRelayTest` 已不存在；源码扫描缩减为 4 条结构/seam 不变量；
删除无引用 `error_no_v1_fallback`；README 改为 outcome seam。
新增 `.github/workflows/android-ci.yml`（JDK 21 JVM job + JDK 17 双 ABI 构建/ELF/JNI/zipalign/证书/proto 检查，
不跑模拟器/真机）；修正两个 workflow 的 `platforms;android-37.0` 包名。
JVM 243 tests/variant PASS；`assembleRelease` + `assembleReleaseAndroidTest`（x86_64 测试签名）PASS。
证据：`evidence/ticket-26/`。
**EMULATOR** zh-CN、200% 字体、TalkBack 手工检查与其余 AVD 矩阵仍 NOT RUN，待协调方统一采集后置 resolved。

## Comments

2026-11-18 — 资产收口已完成并合入；JVM 249 tests/variant PASS，androidTest APK 构建 PASS。
`EMULATOR` 补齐：zh-CN、200% 字体截图与 Home/Settings/About 的 Light/Dark 渲染 PASS
（`evidence/ticket-08-refactor/emulator/screens/`）。
**TalkBack 实际播报审计仍是人工项，NOT RUN**；按 triage 规则本票保持 `ready-for-human`，
待维护者完成 TalkBack 检查后置 resolved。

## Comments

2026-11-18 — TalkBack 人工审计经维护者签署 WAIVED（见 `evidence/ticket-26/WAIVED.md`）。未实际执行，按 evidence-policy 非 PASS。本票仍保持 `ready-for-human`，由维护者决定是否关闭。

## Comments

2026-11-18 — Windows 真机验收返回 `evidence/ticket-26/`。zh-CN 与 200% 字体已由截图覆盖；
TalkBack 实际播报审计未执行，维护者以 `evidence/ticket-26/WAIVED.md` 显式 `WAIVED`（不是 PASS）。
按 triage 规则人工门禁已记录，本票 resolved。
