# 02: Beta 冻结点与重构分支的合并机制

Labels: wayfinder:grilling
Type: grilling
Status: resolved
Blocked by: None (can start immediately)

## Question

决定重构与 beta 并行的精确机制，使重构不污染 beta 验收证据。

需要决定：

1. **分支点**：从当前 `integration/wire-protocol-v2-beta` HEAD 切出，还是从 beta 验收提交 / `0.5.0-beta` tag 切出？
2. **冻结点**：ticket 25（真机 / Windows / v1 回归验收）resolved、`0.5.0-beta` 发布、还是 stable gate 开始？冻结期间允许哪些重构提交存在？
3. **合并方向**：重构分支合入 `integration/wire-protocol-v2-beta` 还是 `main`；使用 merge commit 还是 squash。
4. **证据保全**：哪些 beta 证据在合入时必须重跑；如何标记「证据来自重构前」；如何避免重构让 ticket 25 的证据失效。
5. **Worktree 与命名**：worktree 路径、分支命名；是否允许多个轴各开一个 worktree 并行。

已知约束：当前分支是 `integration/wire-protocol-v2-beta`；ticket 25 仍是 `ready-for-human`；beta-spec 明确排除 F-Droid、自动更新等 stable-gate 工作。

## Answer

### 分支点

- 分支点取 beta 分支的当前 tip `d72b050`：handoff 时从该 SHA 切出 `refactor/android-modernization`，并在重构分支首个 commit 记录该 SHA。不等 tag：工单 29 只改 Server/GUI，APK 哈希未变（`6da98e4a…`），Android 证据已经锚定。
- 纪律：tag 之前若仍有 **Android 代码**改动落到 beta 分支，必须 merge forward 到重构分支，并把 Android 侧证据视为重置；Server 侧修复不触发此规则。

### 冻结点与冻结期间

- 冻结 = 工单 25 以 go 收口 + 在 beta 验收提交上打 **annotated 本地 tag `v0.5.0-beta`**。本地 tag 即证据锚点；push / GitHub Release 仍需维护者单独授权，不因本票自动执行。
- 冻结后 beta 分支只接受 release-blocking 修复；每个修复立即 merge forward 到 `refactor/android-modernization`。**单向流动：重构分支永不倒灌 beta。**
- beta 证据目录 append-only：不重写、不搬移、不重新打 tag。

### 合并方向与策略

- 各轴 `refactor/android-<axis>` → `refactor/android-modernization`：`--no-ff` merge commit（沿用仓库的 `merge(ticket-NN)` 惯例）。禁止 squash：重构的价值在于可 bisect、可单独 revert 的切片。
- post-refactor 证据（[08](08-refactor-verification-and-beta-evidence.md)）通过后，`refactor/android-modernization` → `integration/wire-protocol-v2-beta`：一次 `--no-ff` merge commit。
- `integration/...` → `main` 属于 v2 发布流程（stable gate），不在本图范围，不在本票决定。

### 证据保全原则

1. beta 证据绑定「commit SHA + APK/Server 哈希」，不绑定分支 tip。
2. 重构不会让重构前证据自动失效，也不会自动继承；每个 `PASS` 必须对具体 commit 重新记录。
3. post-refactor 记录写入新目录（建议 `evidence/ticket-08-refactor/`），与 `ticket-25/` 并存，各自标明所属 commit 与证据时间点。
4. 冻结的 v0.3.4 APK 与 v1 fixtures 任何轴都不得修改。

重跑矩阵（JVM / `EMULATOR` / 真机）与合入门槛由 [08](08-refactor-verification-and-beta-evidence.md) 决定；CI 扩展由 [09](09-build-ci-baseline-profile.md) 决定。

### Worktree、命名与隔离

- 集成分支：`refactor/android-modernization`；每轴：`refactor/android-<axis>`（如 `refactor/android-session`、`refactor/android-ui`、`refactor/android-build`）。
- worktree 路径：`~/.cache/audio-share-refactor-worktrees/<axis>`（沿用 beta worktree 先例）。
- 每个 worktree 使用**独立 `CARGO_TARGET_DIR`**（建议 `~/.cache/audio-share-refactor-worktrees/<axis>/cargo-target`），不共享：共享 target 会在分支间反复全量重编并抢文件锁；磁盘成本约 1–2 GB/轴。`~/.gradle` 全局共享（内容寻址 + 锁），每个 worktree 的 `.gradle` 项目目录天然隔离。
- 并行上限 2–3 轴；native 构建是瓶颈。
- 任何轴都不得执行 `scripts/apply_version.sh`、不得修改 `VERSION` / `versionCode`（`release.yml` 硬校验 `versionName`，且版本号属于 beta 发布线）。
- `native/Cargo.lock` 与 `tools/rust-android-interop/Cargo.lock` 的变更只在 `refactor/android-modernization` 上解决，不手改到 beta。
- beta 分支不挂任何重构 worktree。

### 文档落点

`GLOSSARY.md` 与 `docs/adr/0005-session-outcome-ownership.md` 随 `refactor/android-modernization` 的**首个 commit** 提交，不提交到 beta 分支；重构分支存在前保持未提交。

### 证据状态

`NOT RUN` —— 本票只产出决策；未创建分支、worktree 或 tag，未提交任何代码。执行动作留待 handoff。

### Handoff（未执行）

```sh
git worktree add -b refactor/android-modernization \
  ~/.cache/audio-share-refactor-worktrees/modernization d72b050
```

- 集成分支的 worktree 目录名为 `modernization`，不套用 `<axis>` 规则；各轴仍为 `~/.cache/audio-share-refactor-worktrees/<axis>`。
- `GLOSSARY.md` 与 `docs/adr/0005-session-outcome-ownership.md` 目前只在 beta 工作区未提交。从 `d72b050` 切出的 worktree **不会**带上它们：handoff 时先把两份文件复制到重构 worktree 提交，再在 beta 工作区 `git checkout GLOSSARY.md` 并删除 ADR 文件。
- 在该还原完成前，**不要在 beta 分支执行 `git add -A`**，否则会把这两份文档误提交进 beta。
- 若 tag 之前 beta 再出现 Android 改动，按「merge forward + Android 证据重置」规则处理。



## Comments

2026-11-17 — 分支点更新（一致性说明，不改 Answer 的机制决策）

- beta 分支 HEAD 已从 `d72b050` 推进到 `cb2faf1`，期间合入工单 30 的 Android 代码
  （`33276e8`）及 beta 文档。
- 因此 handoff 里的 `git worktree add ... d72b050` 不能照做：改为从 **25 GO 后的冻结 tip**
  切分支，或从 `d72b050` 切出后 merge forward。
- 按 Answer 的规则，tag 之前的 Android 改动意味着 Android 侧证据重置。
- 合并方向、`--no-ff`、worktree 命名与证据绑定原则不变。
