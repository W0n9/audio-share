# 13: 重构分支 handoff：冻结 tip 与 worktree

**What to build:** 在 Beta 冻结后，从冻结 tip 建立 `refactor/android-modernization` 集成分支与各轴 worktree，记录基线 SHA 与产物哈希，让后续所有实现票有一个干净、可回滚的起点。

**Blocked by:** [wire-protocol-v2 工单 25](../../wire-protocol-v2/issues/25-beta-acceptance-real-device-windows-v1.md) 转 GO 并打 annotated `v0.5.0-beta` tag（外部条件）。

2026-11-17 核验：工单 25 已 **resolved / GO（fresh-install pilot）**，HEAD `8858c91`；
`v0.5.0-beta` tag 已创建（annotated，2026-11-17 18:28:18 +0800，指向 `8858c91`）。
冻结条件已满足。

**Status:** resolved

- [x] [wire-protocol-v2 工单 25](../../wire-protocol-v2/issues/25-beta-acceptance-real-device-windows-v1.md) 的 GO 判定与 `v0.5.0-beta` tag 已记录；分支从冻结 tip `8858c91` 切出（tag 之后 beta 无 Android 提交，无需 merge forward）。
- [x] `refactor/android-modernization` 与按轴 worktree 建立，每轴使用独立 `CARGO_TARGET_DIR`（见 `evidence/ticket-13/worktrees.tsv`）。
- [x] `GLOSSARY.md` 与 session-outcome ADR 作为重构分支首个 commit `39cbda2` 提交；ADR 0005 的 Ended 屏幕语义按 06 修订。
- [x] `VERSION` / `versionName` / `versionCode` 未改；beta 分支不挂重构 worktree。
- [x] 基线 commit SHA、APK/Server 产物哈希与冻结时间写入 `evidence/ticket-13/handoff.md`。

## Comments

2026-11-17 — handoff 完成。集成分支 `refactor/android-modernization`（worktree
`~/.cache/audio-share-refactor-worktrees/modernization`）首个 commit `39cbda2`。
基线 JVM `./gradlew test -PnoNative=true` 全绿（JDK 17 / Gradle 8.9 / AGP 8.7.3）。
本地历史分支 `refactor` 重命名为 `refactor/legacy-docs` 以释放命名空间。
