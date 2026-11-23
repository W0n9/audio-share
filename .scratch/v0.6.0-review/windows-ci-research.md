# Windows CI 与 Action 版本调研（2026-11-23）

## 结论

1. **「Windows 很慢」不是编译慢，而是一个测试永久挂起。** `v2-transport` 的
   `pairing_live::malformed_pairing_capabilities_are_401_live` 在 `windows-latest` 上从
   01:13:59 开始运行，01:14:59 被标记「超过 60 秒」，直到 02:12 我取消运行仍在等。
   作业没有 `timeout-minutes`，默认可挂 6 小时。这是 Beta 之后新增的测试；Windows
   CI 上一次绿色是 2026-11-10，那时整组测试只要 27 秒。
2. **真正的编译耗时并不高。** 热缓存下 Windows 打包：GUI 92 秒、`as-cmd` 15 秒；
   冷缓存：288 秒 / 56 秒。Linux 对应的 bundle 构建 114–377 秒，两者同量级。
3. **不需要交叉编译。** 交叉编译不减少编译工作量，只是换平台；而且按证据政策，
   `cargo-xwin` 产物在 Windows 上执行前只能记 `NOT RUN`。仓库此前正是这么做的
   （`v0.5.0-beta` 交叉构建后必须在 Windows 11 冒烟）。
4. **Action 版本全部落后一个大版本以上，且 Node 20 已于 2026-09-23 从 runner 移除**，
   当前靠强制 Node 24 运行。已升级到各 Action 的 node24 主版本。

## 实测耗时（GitHub API 的步骤时间）

| 作业 / 步骤 | 冷缓存（45043427082） | 热缓存（45092646791） |
| --- | --- | --- |
| Windows：Build portable GUI exe | 288 s | 92 s |
| Windows：Build Windows as-cmd | 56 s | 15 s |
| Windows：Clippy（Rust 矩阵作业） | — | 166 s |
| Windows：Test workspace（Rust 矩阵作业） | — | **挂起 > 62 分钟，取消** |
| Linux：Build Linux bundles（deb + AppImage） | 377 s | 114 s |
| Linux：Build Linux as-cmd | 48 s | 8 s |

2026-11-10 的绿色运行（[42777988420](https://github.com/W0n9/audio-share/actions/runs/42777988420)）：
Windows `Clippy` 31 秒、`Test workspace` 27 秒，整个 Windows Rust 作业 1 分钟。
公共仓库的标准 runner 两个平台都是 4 vCPU / 16 GB，因此不是核数差异。

## 挂起证据与当前处理

- 日志停在 `pairing_live.rs` 的 8 个测试：7 个在 0.6 秒内通过，第 8 个
  （`malformed_pairing_capabilities_are_401_live`）再无输出。
- 该测试里只有几处没有超时的等待：`connect()` 的 QUIC 握手、`open_bi()`、
  `send.write_all()`。`read_frame` 和 `connection.closed()` 自带 5 秒超时。
- 已在 `connect()` 外包 10 秒超时，失败信息写明阶段。下次运行若报
  `QUIC handshake did not complete within 10 s`，即锁定在握手；否则继续在流阶段加超时。
- 在 Linux 上重复运行 25 次、每次 8 个 live 测试全部通过（整个文件 0.62 秒），
  说明是 Windows 环境或平台行为，不是普通竞态。
- 已给 Rust 矩阵作业加 `timeout-minutes: 30`，避免再次占用 6 小时。
- **间歇性，而非必现**：升级 Actions 后的运行
  [45100003428](https://github.com/W0n9/audio-share/actions/runs/45100003428) 里同一测试
  通过，Windows `Test workspace` 168 秒、`Clippy` 105 秒，全部作业成功。
  因此暂不能把「每次都会挂」写进结论；10 秒握手超时与 30 分钟作业上限保留为守卫，
  下次再挂会直接给出阶段。

## 是否需要交叉编译

| 维度 | 原生 Windows 构建（现状） | `cargo-xwin` 在 ubuntu 交叉编译 |
| --- | --- | --- |
| 编译工作量 | MSVC target 编译一遍 | 同样要编译 MSVC target，依赖不共享 |
| 实测参考 | GUI 冷 288 s / 热 92 s | Linux 上同类 GUI 构建 114–377 s |
| 额外成本 | 无 | 下载并缓存 MSVC CRT/SDK（约 1 GB），需维护 `cargo-xwin` |
| 证据政策 | 可在 Windows 上直接验证 | 交叉产物只能记 `NOT RUN`，必须在 Windows 冒烟才能升级 |
| 仓库先例 | 当前 CI 路径 | `v0.5.0-beta` 用过；发布说明明确「cross-built, NOT RUN」 |

**结论：不值得用交叉编译替换发布路径。** 它不节省总编译时间，还会让正式产物失去原生
Windows 验证。若只是想让 PR 更快拿到「Windows 能编译」的信号，可以额外加一个
`cargo-xwin` 的 host 检查；但它的价值取决于是否真的缺少 Windows runner 容量。

## 更有效的提速手段

1. **PR 上不构建发布包**（收益最大）。`server-rewrite.yml` 目前对 PR 也运行
   `windows-packaging` 与 `linux-packaging`，每次都要做完整 release 构建。建议改为：

   ```yaml
   windows-packaging:
     if: github.event_name != 'pull_request'
   linux-packaging:
     if: github.event_name != 'pull_request'
   ```

   PR 仍保留 `rust` 矩阵（fmt、Clippy、测试）。发布和 main 推送照旧产出资产。
2. **先修挂起测试**，否则 Windows 作业永远到不了「通过」。
3. **可选：限制 Windows 测试范围**。如果只想守住 Windows 专属代码，可把 Windows 上的
   `cargo test --workspace` 收窄到 `audio-share-core`（WASAPI）与必要 crate，
   其余集成测试交给 Linux。需评估覆盖损失，未实施。
4. **可选：合并两个 Windows 作业**，省一次 checkout/setup（约 20–30 秒），收益有限。

## Action 版本（2026-11-23 最新）

| Action | 原版本 | 新版本 | 运行时 | 主要变更 |
| --- | --- | --- | --- | --- |
| `actions/checkout` | v4 | **v7.0.1** | node24 | v6 凭据存到独立文件；v7 禁止 `pull_request_target`/`workflow_run` 检出 fork PR，改 ESM |
| `actions/setup-java` | v4 | **v6.3.0** | node24 | v6 迁移 Azul Metadata API、ESM；`jdkFile` 改 `jdk-file`（旧名保留别名，本项目未用） |
| `actions/setup-node` | v4 | **v7.1.0** | node24 | v5 起按 `packageManager` 自动缓存（本项目显式 `cache: npm`，不受影响）；v7 移除假 `NODE_AUTH_TOKEN` |
| `actions/upload-artifact` | v4 | **v7.0.2** | node24 | v7 新增单文件直传（`archive: false`），默认仍是归档 |
| `actions/download-artifact` | v4 | **v8.0.2** | node24 | v8 改 ESM；哈希不匹配默认报错；按 `Content-Type` 决定是否解压 |
| `Swatinem/rust-cache` | v2 | v2（浮动，已到 2.9.3） | node24 | 无需改 |
| `android-actions/setup-android` | v4 | v4（浮动，已到 4.0.4） | node24 | 无需改 |
| `dtolnay/rust-toolchain` | stable | 不变 | 复合 Action，无 Node | 无需改 |

Node 20 时间线（[GitHub 变更日志](https://github.blog/changelog/2025-09-19-deprecation-of-node-20-on-github-actions-runners/)）：
2026-06-16 默认切到 Node 24，**2026-09-23 移除 Node 20**，此后 node20 的 Action 只能被强制
运行在 node24 上。本仓库的旧版本仍能跑，但属于不受支持的组合，且会持续告警。

## 本次已改

- 四个工作流的 Action 版本升级（上表）。
- `server-rewrite.yml`：Rust 矩阵作业加 `timeout-minutes: 30`。
- `release.yml`：preflight 10 分钟、draft 15 分钟上限。
- `pairing_live.rs`：`connect()` 加 10 秒握手超时，失败信息带阶段。

升级后运行 [45100003428](https://github.com/W0n9/audio-share/actions/runs/45100003428)
全部作业成功（`create_draft=false`，draft 作业按预期跳过）；日志中不再出现
`Node.js 20 is deprecated`。Windows 产物 `AudioShareServer.exe` 与
`audio-share-server-cmd-windows.zip` 的 SHA-256 清单已在下载后核对通过。

真机与 Windows 上的挂起根因仍需在 Windows 上复现：

```powershell
cargo test -p v2-transport --test pairing_live malformed_pairing_capabilities_are_401_live -- --nocapture
```

## 待决定

- 是否接受「PR 不构建发布包」的策略（上节第 1 条）。
- 是否在 Windows 上复现并修复该挂起测试；在修复前，Windows CI 会继续红。