# 10: Final integration review fixes

**What to build:** 修复 `/implement-spec` 最终 Standards/Spec 两轴审查在集成分支发现的问题；保持 Tracer-Bullet 第 3 步边界，不扩展到媒体播放、抖动/漂移、生产 pairing 等后续阶段。

**Blocked by:** 06, 07, 08, 09

**Status:** resolved

**Type:** task

## Requirements

- 审查基线为 `main` (`10e5a28`)，被审查 HEAD 为 `phase-2/wire-protocol-v2` (`599edd0`)。
- 独立 worktree/分支：`/home/w0n9/.claude/jobs/f6315943/tmp/wire-v2-final-review`（`review/wire-protocol-v2-final`，基于 `phase-2/wire-protocol-v2` @ `599edd0`）。
- 完整问题清单：`/home/w0n9/.claude/jobs/f6315943/tmp/wire-v2-final-review-findings.md`。
- 逐项修复 Standards 与 Spec 发现，添加能先暴露缺陷的回归测试；规范角色词汇与协议语义不可因文字清理而改变。
- 保持 v1 framing、fixtures、Android 兼容线与发布行为不变。
- 本工单是最终集成的 review-fix 记录，不表示第 4 步媒体交付已完成。

## Definition of done

- 审查发现均有修复、针对性回归测试或明确的范围/不适用说明。
- `cargo test --workspace`、Android native host tests、Kotlin tests、v1 六套 fixture 回放全绿。
- Rust workspace 与 Android native 的 `cargo fmt --check`、Clippy `-D warnings` 全绿。
- Android release 与 v0.3.4 debug 构建绿；需要真机但环境不可执行的项明确记 NOT RUN。
- 修复分支先合回集成分支最新 tip，再由 merger 合入；不得在集成分支外宣称完成。

## Comments

### 2026-11-13 — final review opened

`implement-spec` 的 Standards 与 Spec 并行审查完成；本记录保存可持久化审查交接，完整审查上下文仍指向 job tmp 文件。

### 2026-11-13 — test seams confirmed by the requested fix workflow

本工单是用户显式 `/implement-spec` 流程中对最终 review 的指定修复，而非开放式新接口设计；原 01–09 已确认的要求与「修复所有审查问题」共同授权以下现有公共测试 seam。新增实现测试必须限制在这些 seam，不添加私有实现耦合测试。

1. **Wire Protocol 帧/会话公共边界**：v2-transport live QUIC driver、Android native session/control 公共事件 API；覆盖确定性错误码、401/402/403 终态与重连策略，不代替 Android OS 生命周期验证。
2. **Server 监听/配置公共边界**：as-cmd 参数、Transport socket、live IPv4/IPv6 连接；覆盖地址解析、dual-stack 与 301 可见性，不声称所有网络均可用。
3. **Android 应用生命周期公共边界**：manifest、前台 Service、公开 session facade；覆盖 Activity 重建/锁屏生存与清理，真机不可用则明确 NOT RUN。
4. **身份/授权持久化公共边界**：identity、authorization store 文件 API 与 as-cmd 启动行为；使用隔离临时目录验证首次创建、重载、缺失、损坏、显式重置及权限，不接触用户真实数据。
5. **golden fixtures 与 v1 protocol-harness replay 公共接口**：独立字节标准和冻结 v1 回归；Rust/Kotlin 不得互为编码器。

实现采用逐场景 red → green，每条行为修复先有失败测试。Standards-only 清理在相关行为测试绿后进行，格式修复不创建人造行为测试。机器可读交接：`/home/w0n9/.claude/jobs/f6315943/tmp/wire-v2-final-review-seams.md`。

### 2026-11-13 — final review fixes merged

`review/wire-protocol-v2-final` 已通过 merge commit `83bfd55ed2f9891151148bc7468b205b2553394f` 合入集成分支 `phase-2/wire-protocol-v2`。本次修复覆盖 Android malformed input、codec/control 错误码、前台 Service 生命周期与规范表述，以及 Server capacity/301 可观测拒绝、dual-stack 监听与身份/授权 fail-closed 持久化；未扩展到第 4 步及以后范围。

合并后 integration tree 验证：
- `cargo test --workspace`：41 suites，458 passed，4 ignored。
- `cargo test --manifest-path android-client-v2/native/Cargo.toml`：2 suites，102 passed。
- `cargo fmt --all -- --check`：通过。
- `cargo fmt --all --manifest-path android-client-v2/native/Cargo.toml -- --check`：通过。
- `git diff --check main...HEAD`：仅报告既有 evidence log trailing whitespace、工单末尾空行及 `gradlew.bat` CRLF；未删改原始证据或工具文件。

实现分支交接验证为 Rust workspace/native tests、Kotlin debug/release、v2 release、v0.3.4 debug、六套 v1 replay 与 fmt/clippy 全部通过。Android 真机 device smoke 因 `adb devices` 无设备仍为 NOT RUN。
