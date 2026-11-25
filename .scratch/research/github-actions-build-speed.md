# GitHub Actions 能否优化本项目的构建速度

评估日期：2026-11-24  
代码基线：分支 `integration/v0.6.3-gui-network`，提交 `9a86841907bc8a1ce49d8c72c321172c462abd9a`（本地 `git rev-parse` 取得；工作树当前在 `docs/v0.6.0-curation`，本次只读访问目标分支，未切换分支、未改代码）[1]  
范围：该分支上的 4 个 workflow（`android-ci.yml`、`release.yml`、`server-rewrite.yml`、`update_sourceforge.yml`）及其涉及的两套 Gradle 构建（v2 Android 客户端）、root Cargo workspace（Rust Server + Tauri GUI）与 npm 前端。评估对象仅为「构建/CI 耗时」，不涉及产物正确性、签名安全、测试覆盖率。Docker 层缓存不适用：目标分支的 CI 中没有任何 Docker 构建步骤[2][3][4][5]。仓库内另一个传统 `android-app/` 目录不在这 4 个 workflow 的构建范围内，不纳入评估[3][4]。  
方法：① 用 `git show integration/v0.6.3-gui-network:<path>` 与 `git grep` 只读梳理 CI 与构建配置，所有本地事实均给出目标分支路径与行号；② 用已认证的 `gh` CLI 读取 `W0n9/audio-share` 仓库的真实 workflow run 数据（run ID、每 job/每 step 起止时间），这是仓库内的实测数据，按原文秒数引用；③ 外部事实以 GitHub 官方文档（`docs.github.com`）、Gradle 官方文档（`docs.gradle.org`）、Rust/Cargo 官方文档、action 官方仓库 README 为一手来源，逐条编号引用。GitHub 官方文档中 `docs.github.com/en/actions/how-tos/.../events-that-trigger-workflows` 的若干 URL 及 billing、larger-runners 页面的部分抓取返回 404/ECONNRESET，已改用可达的同主题官方文档或 action README；抓取失败的页面未作为结论依据（详见「未确认与证据边界」）。**本次未修改任何 CI 配置、未在本地复现构建。**

## 摘要

**结论：能优化，而且收益空间很大——当前瓶颈不在编译本身，而在“每次都重复安装系统依赖”和“发布打包在每个 PR/push 上全量跑”。** 有真实 run 数据支撑：2026-11-24 的一次 `server-rewrite.yml` 运行（run `45337047683`，目标分支）从创建到结束 **1423 秒（约 23.7 分钟）**，其中 Linux 两个 job 的 `apt-get` 安装步骤分别耗时 **1300 秒**和 **1169 秒**，均占各自 job 墙钟时间的 90% 左右；同一次运行中，真正编译/测试的 `Clippy` 仅 7 秒、`Test workspace` 70 秒（Linux），Windows 打包 73 秒、Linux 打包 117 秒[6]。另一 run（`45302650376`，同分支）只有 `linux-packaging` 的 apt 耗时 **1322 秒**，而同一 run 中 `Rust (ubuntu-24.04)` 的 apt 仅 **32 秒**；该次 Windows 打包 `Build portable GUI exe` 耗时 342 秒、Windows `Clippy` 174 秒、`Test workspace` 259 秒[7]。对比 `main` 分支 2026-11-11 的 run `43001568578`：总耗时 171 秒，Linux apt 仅 28~29 秒，构建 97~102 秒[8]。但这次运行用的是较早的 workflow 版本（`actions/checkout@v4`、`actions/setup-node@v4`，而目标分支已升到 v7），**不是同一版本的一对一对照**。因此 apt 慢是**间歇性的镜像/网络风险，而非每次必然发生的固定成本**：同一 run 内不同 job 可以相差 40 倍以上[6][7][8]。优化方向是减少 apt 的暴露面与下载量，并保留重试/多镜像等鲁棒性，而不是假设“去掉 apt 就必省 20 分钟”。

最值得做的 3 项（第 2、3 项收益确定、改动量小，优先做；第 1 项针对间歇性但影响最大的风险点）：

1. **降低 `apt-get install` 的重复暴露面**（`server-rewrite.yml` 的 `rust` job 与 `linux-packaging` job）。apt 是唯一出现 20 分钟级耗时的步骤，但同分支不同 run 之间、甚至同一 run 的不同 job 之间差异极大（32 s 到 1322 s），属间歇性镜像/网络风险[6][7][8]。缓存 apt 下载的 `.deb` 或合并两个 Linux job 属于 CI 配置层，作用是减少下载量和暴露次数；同时应保留重试/多镜像/超时保护。注意官方仓库缓存上限 10 GB/7 天未访问即淘汰[26]，本项目无需缓存整包，只需缓存下载目录或安装产物目录。
2. **给两个 Gradle job 加 Gradle 缓存**（`android-ci.yml` 的 `jvm-tests`、`build-assertions`）。当前这两个 job 的 `setup-java` 都没有 `cache: gradle`，也没有 `gradle/actions/setup-gradle`；而 Robolectric 首次运行要下载约 210 MB 的 `android-all-instrumented` jar，配置里也明确写了“expected, not a hang”[3]。`setup-java` 的 `cache: gradle` 可缓存 `~/.gradle/caches` 和 wrapper 发行版[21]；`gradle/actions/setup-gradle` 缓存范围更大（含编译后的 Kotlin DSL 脚本、artifact transforms、本地 build cache）且默认只在默认分支写入[22]。
3. **把发布打包从 PR/push 门禁里拆出来**（`server-rewrite.yml` 的 `windows-packaging`、`linux-packaging` 当前在每次 push 到 `main`/`integration/server-rewrite`/`ticket/**` 以及匹配路径的 PR 上都运行，且 `release.yml` 通过 `workflow_call` 复用同一 workflow 时也会带上它们）[4][5]。改成 `if:` 条件限定只在 `workflow_call`/`workflow_dispatch`/`main` push 时打包，PR 只跑 `rust` 测试门禁；并为 `server-rewrite.yml` 加 `concurrency` 与 `cancel-in-progress: true`（`android-ci.yml` 已有，见下）[24]。

其他结论：现有 CI **已经**做了 Rust 缓存（`Swatinem/rust-cache@v2`）、npm 缓存（`setup-node` 的 `cache: npm`）、`android-ci` 的并发取消、`server-rewrite` 的 PR `paths` 过滤、OS 矩阵与 job 默认并行——这些不要重复实施[3][4][5][20][23][24]。Gradle configuration cache 与 sccache 在本项目属于“需先验证/收益有限”的第二梯队；更大 runner 与自托管 runner 属于“收费/需评估”的第三梯队[19][25][28][31]。

## 1. 现状：CI 在构建什么、怎么触发、已有哪些缓存

### 1.1 四个 workflow 一览

| Workflow | 触发条件 | 主要 job | 产物 |
|---|---|---|---|
| `android-ci.yml` | `pull_request`、`workflow_dispatch`、push 到 `refactor/**` | `jvm-tests`（JDK 21 + JDK 17 toolchain，`-PnoNative=true` 跑 `./gradlew test`）、`build-assertions`（JDK 17 + Rust Android targets，跑 `assembleRelease assembleReleaseAndroidTest`，unzip/readelf/zipalign/apksigner 断言，再跑一次 `-PtestSupport=false` 的 `assembleRelease`） | 仅上传 JVM 测试报告 artifact；不构建 release 签名包[3] |
| `release.yml` | `workflow_dispatch`，输入 `version` 与 `create_draft` | `preflight` → `build_server`（`workflow_call` 复用 `server-rewrite.yml`）与 `build_android`（按 ABI 循环 `arm64-v8a`、`x86_64`，每个 ABI 一次 `assembleRelease` + 打包）→ `create_release`（`needs` 两者，创建 draft） | Server Windows/Linux 包与两个签名 APK；下载 artifact 汇总[5] |
| `server-rewrite.yml` | push 到 `main`/`integration/server-rewrite`/`ticket/**`；PR 带 12 条 `paths` 过滤；`workflow_call`；`workflow_dispatch` | `rust`（`ubuntu-24.04` + `windows-latest` 矩阵，fmt/clippy/test/harness smoke/fixture replay）、`windows-packaging`（`npm ci` + `npm run build` + `npx tauri build --no-bundle` + `cargo build --release -p as-cmd` + 打包上传）、`linux-packaging`（同样前端构建，`npx tauri build --bundles deb,appimage` + as-cmd + 打包上传） | Windows 便携 exe、as-cmd zip、Linux deb/AppImage、Linux as-cmd tar.gz，共 5 类 artifact[4] |
| `update_sourceforge.yml` | 仅 `workflow_dispatch` | `set_default`，调用脚本设置 SourceForge 默认下载 | 无构建，耗时可忽略[2] |

### 1.2 目标分支已有的优化（避免重复建议）

- **Rust 缓存已覆盖全部 Rust job**：`server-rewrite.yml` 的 `rust` job（`Swatinem/rust-cache@v2`，默认 `workspaces`）、`windows-packaging` 与 `linux-packaging`（`workspaces: . -> target`）、`android-ci.yml` 的 `build-assertions` 与 `release.yml` 的 `build_android`（`workspaces: android-client-v2/native -> target`）都用了该 action[3][4][5]。其 README 说明它缓存 `~/.cargo` 与 `./target`（依赖构建产物），默认按 job 分键，且只缓存依赖不缓存 workspace crate 本身[20]。
- **npm 缓存已开启**：两个 packaging job 的 `actions/setup-node@v7` 都显式 `cache: npm` 且 `cache-dependency-path: gui/package-lock.json`[4]。README 说明它缓存全局包数据但不缓存 `node_modules`[18]。
- **`android-ci.yml` 已有 workflow 级 `concurrency`**：`group: android-ci-${{ github.workflow }}-${{ github.ref }}`，`cancel-in-progress: true`[3]。这是 `server-rewrite.yml` 缺少的[4]。
- **`server-rewrite.yml` 的 PR 已有 `paths` 过滤**：只有 Cargo/crates/gui/tools/fixtures/scripts/VERSION/metadata/相关文档/workflow 自身变更才触发 PR 运行[4]。`android-ci.yml` 的 PR 没有 `paths` 过滤[3]。
- **job 默认并行 + 矩阵**：`server-rewrite.yml` 的 `rust` job用 `strategy.matrix.os = [ubuntu-24.04, windows-latest]` 且 `fail-fast: false`；三个顶层 job 之间没有 `needs`，默认并行[4]。`release.yml` 的 `build_server` 与 `build_android` 都只 `needs: preflight`，彼此并行[5]。GitHub Actions 语法文档明确 jobs “run in parallel by default”，需要串行才用 `needs`[14]。
- **超时上限已设**：各构建 job 都有 `timeout-minutes`（`rust` 30、packaging 60、Android 60、preflight 10、create_release 15）[3][4][5]。
- **前端构建已显式分开**：workflow 先有独立的 `Build frontend`（`npm run build`）步骤，再执行 `npx tauri build`[4]；而 `tauri.conf.json` 的 `beforeBuildCommand` 也配置为 `npm run build`[10]。两者是否造成重复的前端构建，本次未从日志中确认（run 的 step 列表只显示 workflow 自己声明的 `Build frontend`），不作为结论依据，也不列为本调研的瓶颈。

### 1.3 构建配置要点

- **Android v2（`android-client-v2/`）**：Gradle Wrapper 9.7.1；`gradle.properties` 中 `org.gradle.jvmargs=-Xmx2048m`，`org.gradle.parallel` 那一行是被注释掉的[11]。`app/build.gradle.kts` 通过 `Exec` 任务直接调用 `cargo build --release --target <abi>`（不用 cargo-ndk），并用 `llvm-strip` 处理 `.so`；`-PnoNative=true` 可跳过 native 编译[12]。`native/Cargo.toml` 明确说该 crate **故意不加入** root Cargo workspace，因此两套 Rust 缓存相互独立[13]。CI 注释指出首次运行 Robolectric 要下载约 210 MB SDK 36 jar[3]。
- **Rust server + GUI**：root workspace 包含 `crates/*`、`gui/src-tauri` 及各 tools；`rust-version = "1.90"`，edition 2021[9]。Tauri GUI 前端是 Svelte 5 + Vite 8 + `@tauri-apps/cli` 2.12.4，`npm run build` 即 `vite build`[10]。Linux 打包依赖 WebKitGTK 4.1、GTK3、PipeWire 等 `-dev` 包，两个 Linux job 各自独立安装一遍[4]。

## 2. 实测数据：慢在哪里

数据取自 `W0n9/audio-share` 的 `gh run view <id> --json jobs`（run `45337047683`、`45302650376`、`43001568578`、`45156050225`）与 `gh run view --log`（apt 日志）。时间为 GitHub API 返回的 step/job `startedAt`/`completedAt` 之差，按整秒计[6][7][8][16][17]。

| Run（分支 / 日期） | 总墙钟 | 最慢步骤 | 结论 |
|---|---|---|---|
| `45337047683`（`integration/v0.6.3-gui-network`，2026-11-24 05:58Z） | 1423 s | `Install Linux build dependencies (Tauri + PipeWire)` **1300 s**；`Install Tauri Linux build dependencies` **1169 s**；`Test workspace`(Win) 114 s；`Build Linux bundles` 117 s；`Build portable GUI exe`(Win) 73 s | apt 占两个 Linux job 时间的约 90%[6] |
| `45302650376`（同分支，2026-11-24 01:07Z） | 2427 s | `Install Tauri Linux build dependencies` **1322 s**；`Build portable GUI exe`(Win) 342 s；`Test workspace`(Win) 259 s；`Clippy`(Win) 174 s；`Build Linux bundles` 285 s | apt 仍是最大单一瓶颈；Windows 编译在冷缓存下也显著变慢[7] |
| `43001568578`（`main`，2026-11-11 10:11Z） | 171 s | apt 28~29 s；`Build Linux bundles` 102 s；`Build portable GUI exe` 97 s | 当时使用 checkout/setup-node v4、workflow 版本较早，仅作镜像/网络波动参考；apt 差 40 倍以上不能当作同版本 A/B[8] |
| `45156050225`（`release/v0.6.2`，2026-11-23 10:25Z，`Make Release`） | 2686 s（含 preflight 与串行等待） | `Install Linux build dependencies` 509 s；`Build portable GUI exe`(Win) 375 s；`Build Linux bundles` 284 s；`Build signed Android v2 APKs` 395 s（其中 `Install NDK...` 132 s、`Build/verify/package` 232 s）；`Rust (ubuntu-24.04)` 796 s（apt 509 s） | 发布路径同样被 apt 与 Windows GUI 构建主导；`build_android` 的两个 ABI 在同一 job 内串行[16] |
| `45347340500`（同目标分支的 `Make Release`，2026-11-24 07:21Z） | — | 抓取时状态为 `in_progress` | 未纳入耗时结论[17] |

apt 慢的原因可从 `45337047683` 的 **`Rust (ubuntu-24.04)` job 日志**（该日志片段仅来自此 job）直接看到：`sudo apt-get update` 从 `us-east-2.ec2.archive.ubuntu.com` 等源拉取索引，`Need to get 100 MB of archives`、`After this operation, 555 MB of additional disk space will be used`，随后几百个 `Get:N` 逐包下载。其中较大的 `libjavascriptcoregtk-4.1-0`（8.1 MB）从 06:00:01 的 `Get:21` 到 06:00:44 的 `Get:22` 约耗时 43 秒（约几百 KB/s），紧随其后的 38.6 kB 小包（`Get:22`→`Get:23`）只用约 1 秒——即慢在大包下载速率，而非每个包都有固定延迟[6]。这是网络/镜像侧耗时，而非编译耗时。**注意**：另一 run 的同一 apt 步骤仅 28~29 秒[8]，但该 run 的 workflow 版本较早；因此“100 MB 下载”本身不解释 1300 s，更可能是镜像瞬时性能/连接问题；优化手段应针对“避免每次都做这件事”，并保留重试/多镜像的鲁棒性。

Android CI 没有可用的真实 run 数据：`gh run list --workflow android-ci.yml` 在 fork 与 upstream 都返回 404（该 workflow 只存在于目标分支，不在默认分支）[19]。所以 Android 侧的收益只能定性判断，不给秒数。

## 3. 可直接改 CI 配置获得的优化（第一优先级）

### 3.1 降低重复的 apt 安装暴露面（间歇性高收益，有实测）

现状：`server-rewrite.yml` 的 `rust` job 在 Linux 上装 12 个包组，`linux-packaging` 装 17 个包组；两者都先 `apt-get update`[4]。实测耗时波动极大：run `45337047683` 中两步分别耗 1300/1169 s，run `45302650376` 中 `linux-packaging` 耗 1322 s 而 `rust` job 仅 32 s[6][7]。因此这里优化的是**风险暴露面**，不能承诺每次都省 20 分钟。

可行方向（均需实测选择，不预设哪种一定最快）：
- 用 `actions/cache` 缓存 `/var/cache/apt/archives`（下载的 `.deb`）并在安装前恢复，让 `apt-get install` 命中本地包；GitHub 缓存文档说明缓存按分支/标签作用域、默认 10 GB、7 天未访问淘汰[26]。该目录通常远小于 10 GB，但具体大小需先测。
- 把 Linux 的两个 job 合并成一个（同一 VM 内先测试再打包），使 apt 只装一次；代价是失去测试与打包的并行性。在 apt 慢的 run 中合并可避免重复 20 分钟级下载；在 apt 快的 run 中则未必优于并行。需要改 job 结构并重新验证 artifact 上传，属于“改配置但需重测”。
- 选择预装更多依赖的镜像或减少包组（例如只在真正需要 GUI 编译的 job 装 WebKitGTK，`rust` job 只跑 `cargo clippy/test --workspace` 但仍需链接 GUI，所以不能简单删）[4][9]。这一项要明确标注为**未确认可行**，因为 workspace 包含 Tauri GUI，`rust` job 的注释说明它也需要 WebKitGTK/GTK headers[4]。

不建议把 Docker layer cache 列进来：目标分支没有 Dockerfile/容器构建[2][4][5]。

### 3.2 给 Gradle job 加缓存（Android CI 冷启动）

现状：两个 Android job 的 `setup-java@v6` 均未设置 `cache`，也没有 `gradle/actions/setup-gradle`[3]。`setup-java` README 明确支持 `cache: gradle`，并说明它把 `~/.gradle/caches` 作为依赖缓存、把 wrapper 发行版作为独立缓存、还缓存下载的 JDK；Gradle 的 key 基于 `**/*.gradle*`、`**/gradle.properties`、`**/gradle-wrapper.properties`、`gradle/*.versions.toml` 等文件，可用 `cache-dependency-path` 覆盖[21]。

两种实现的取舍（这是本项目实际会遇到的选择）：
- `setup-java` 的 `cache: gradle`：改动最小，缓存依赖 + wrapper + JDK[21]。
- `gradle/actions/setup-gradle@v6`：缓存范围更大，包含“compiled build scripts（`caches/<version>/kotlin-dsl`、`caches/scripts`）、artifact transforms（`caches/transforms-3`）、local build cache（`caches/build-cache-1`）”；文档明确建议“避免”同时用 `setup-java` 的 `cache: gradle` 或 `actions/cache` 缓存 GRADLE_USER_HOME，否则会互相干扰[22]。
- **注意许可/供应链条款**：`gradle/actions` v6 README 说明默认 `enhanced` provider 使用“proprietary technology”的 `gradle-actions-caching`，公共仓库免费、私有仓库为 Free Preview；若项目希望 100% 开源路径，应显式设 `cache-provider: basic`（基于 `actions/cache`，与 `setup-java` 的 `cache: gradle` 策略相同）[22]。这是把“加缓存”落到本项目时必须记录的配置决策。
- 两个方案都默认只在默认分支写缓存、其他分支只读[21][22]；本项目Android CI 主要在 PR 上触发，所以首跑仍是冷缓存，但同分支重跑与基于 base 分支的恢复会生效[26]。

Robolectric 的约 210 MB jar 是否会被上述缓存覆盖：**未确认**。workflow 注释确认其存在与大小[3]，`setup-gradle` 文档确认 `caches/modules-2` 属于被缓存内容[22]，但 Robolectric 的 `android-all-instrumented` jar 由其自身依赖解析器获取，未必落在 Gradle 模块缓存中。若实测发现未被覆盖，可用 `actions/cache` 单独缓存其实际落盘目录。此项标为**证据边界**，不作为收益承诺。

### 3.3 让打包不参与每个 PR

现状：`server-rewrite.yml` 的三个顶层 job 无 `if`，`push` 到 `main`/`integration/server-rewrite`/`ticket/**` 与匹配 `paths` 的 PR 都会触发全部四个 job（含两个 packaging job）[4]；`release.yml` 又用 `workflow_call` 复用同一 workflow，因此发布也会完整跑测试+打包[5]。GitHub 文档说明 jobs 默认并行、`needs` 用于串行依赖、job 级 `if` 可跳过 job，`on.<push|pull_request>.paths` 可限制触发[14][15]。

建议：`windows-packaging` 与 `linux-packaging` 加条件，仅 `workflow_call`、`workflow_dispatch`、或 `push` 到 `main` 时运行；PR 只跑 `rust` 门禁。这样 PR 的墙钟从“apt 20 分钟 + 打包若干分钟”降到“测试若干分钟”。代价是 PR 不再验证打包脚本本身；可用 `scripts/package-*.test.sh` 这类已有的自测脚本在 `rust` job 中覆盖部分逻辑[4]。是否接受这一点需要项目决策，不在本调研中替用户拍板。

### 3.4 其他小项（配置层）

- **`server-rewrite.yml` 加 `concurrency`**：`android-ci.yml` 已有 group + `cancel-in-progress: true`[3]，`server-rewrite.yml` 没有[4]。GitHub 文档说明同 group 同时最多一个运行，新运行会取消正在运行的项[24]。对“推了 5 个 ticket 分支提交”的场景，这能直接省掉被取消运行的剩余时间。
- **`android-ci.yml` 的 PR 加 `paths` 过滤**：目前任何 PR 都跑 Android 测试，而该 workflow 只与 `android-client-v2/**`、`protos/**`、`scripts/**` 等相关[3]。语法文档说明 `paths` 至少一个模式匹配才运行，且可与分支条件叠加[15]。
- **合并 `build-assertions` 的两次 Gradle 调用**：该 job 先 `assembleRelease assembleReleaseAndroidTest`，又一次 `assembleRelease -PtestSupport=false -PnoNative=true`[3]。第二次是有独立断言目的的构建，不应简单删除；但可评估能否让第一次同时产出两个变体、减少一次配置与任务图开销。这属于“需改构建脚本/验证”的范畴，放在第 4 节。
- **`release.yml` 的 ABI 循环改为矩阵**：`build_android` 在一个 job 内 `for abi in arm64-v8a x86_64; do ./gradlew assembleRelease ...` 串行构建并打包[5]。矩阵可让两个 ABI 并行，但会各自重复 SDK/NDK/protoc 安装与 Rust 缓存恢复；在实测 `45156050225` 中，`Install NDK...` 132 s、`Build/verify/package` 232 s[16]，并行是否净收益未知，属于“需 A/B 测量”的中优先级项。GitHub 文档说明矩阵 job 默认不保证顺序，可用 `jobs.<job_id>.strategy.max-parallel` 限制并行数（默认由 runner 可用性决定），`fail-fast` 控制某个 job 失败时是否取消整个矩阵[14]。

## 4. 需要改构建脚本/架构或额外验证的优化（第二优先级）

- **Gradle configuration cache**：`android-client-v2/gradle.properties` 未启用[11]。Gradle 文档说明 configuration cache 会缓存配置阶段结果，命中时跳过整个配置阶段并反序列化任务图；Gradle 10 将默认开启[27]。但本项目 `app/build.gradle.kts` 用自定义 `Exec` 任务在 execution time 解析 `ndkDir`（`providers.environmentVariable(...)` 在任务动作中取值）[12]，而 Gradle 文档明确“referencing the `Project` object at execution time”会在存储时失败，并要求任务字段可序列化[27]。因此必须先在本地跑 `./gradlew ... --configuration-cache` 验证兼容性，不能直接承诺收益。
- **Gradle build cache（任务输出缓存）**：Gradle 文档说明它“reusing outputs produced by other builds”，本地或远程均可，且“even works across developer machines and build agents”[29]。`setup-gradle` 会缓存 `caches/build-cache-1`（本地 build cache）[22]。但本项目每次 CI 都是全新 checkout、任务输入变化不大时收益才明显；是否值得自建远程 build cache（例如 Develocity）属于架构选择，文档中也提示 `cache-provider: external` 用于外部 GRADLE_USER_HOME 缓存[22]。不列为直接可落地项。
- **sccache**：Cargo 官方文档说明 sccache 是第三方工具，可通过 `RUSTC_WRAPPER` 在不同 workspace 间共享编译产物[30]。但本项目每个 Rust job 都已用 `Swatinem/rust-cache@v2` 缓存 `~/.cargo` 与 `target`[4][20]；在 CI 这种单次任务场景，sccache 的边际收益很可能很小，且增加服务端/密钥管理。除非已有可复用的 sccache 后端，否则不建议作为近期项。
- **artifact 复用避免重复构建**：`upload-artifact@v7`/`download-artifact@v8` 支持从其他 workflow run 下载（需 `run-id` + `github-token`），artifact 默认保留 90 天[31]。理论上可“在 push 到 main 时构建一次产物，release 直接复用”，从而让 `release.yml` 跳过完整 `server-rewrite` 测试矩阵；但 `release.yml` 当前特意 `workflow_call` 复用完整门禁[5]，这更像是有意的质量保证而非浪费。因此只把它列为“需产品/发布流程决策”的候选，并明确：工件复用会改变发布验证语义。

## 5. 收费/runner 层面的选项（第三优先级）

- **更大/自托管 runner**：GitHub 官方文档列出 `ubuntu-24.04` 公共仓库为 4 CPU / 16 GB，私有仓库为 2 CPU / 8 GB；更大 runner 面向 Team/Enterprise Cloud 计划，提供“More RAM, CPU, and disk space”，且“Larger runners require a valid payment method”[25]。billing 文档给出相对价格：Linux 2-core `$0.006`/分钟，Windows 2-core `$0.010`/分钟（约 1.67×），macOS 3/4-core `$0.062`/分钟（约 10.33×）；公共仓库标准 runner 免费[28]。**关键判断**：本项目瓶颈是 apt 网络下载，不是 CPU；换更大 runner 不能直接解决第 2 节实测的主导项。只有在apt问题被解决后、Windows GUI 构建（342~375 s）成为新瓶颈时，才值得评估更大 Windows runner 的性价比[7][16]。
- **自托管 runner（可常驻缓存 apt 与 Gradle 依赖）**：能从根本上避免重复安装，但引入运维、安全与缓存污染责任。GitHub 缓存文档强调低信任触发（如 fork PR）读取默认分支缓存存在投毒风险[26]；自托管 runner 上运行不可信 PR 代码风险更高。本次不推荐，仅列为选项。

## 6. 优先级与预期收益（定性）

| 优先级 | 手段 | 类别 | 预期收益 | 证据 |
|---|---|---|---|---|
| P0（间歇性高收益） | 降低 apt 重复安装暴露面（缓存包目录 / 合并 Linux job / 减少包组 / 重试与多镜像） | 改 CI 配置 | **高但波动大**。单次 run 可达 1300 s 级别，另一次仅 32 s，无法承诺固定收益[6][7][8] | run 数据 + workflow 行号[4][6][7][8] |
| P0 | Android job 加 Gradle 缓存（择 `setup-gradle` 或 `setup-java cache: gradle`） | 改 CI 配置 | **中高**。消除 Gradle 依赖/wrapper/210 MB Robolectric jar 的重复下载；无 Android run 实测，不给秒数[3][21][22] | 配置现状 + action 文档[3][21][22] |
| P0 | packaging job 加 `if` 条件，PR 不打包；`server-rewrite.yml` 加 `concurrency` | 改 CI 配置 | **中高**。PR 路径直接省掉两个 packaging job；重复 push 自动取消[4][14][15][24] | workflow 行号 + 官方文档 |
| P1 | `android-ci.yml` 加 `paths` 过滤 | 改 CI 配置 | **中**，仅对非相关 PR 生效[3][15] | 同上 |
| P1 | `release.yml` Android ABI 矩阵并行 | 改 CI 配置 + A/B 测量 | **不确定**。两个 ABI 并行 vs 重复环境准备，需实测[5][16] | run 数据 + 语法文档[14] |
| P2 | Gradle configuration cache | 改构建脚本 + 验证 | **未知，可能为正也可能被自定义 Exec 任务阻断**[11][12][27] | Gradle 官方文档 |
| P2 | Gradle remote build cache / artifact 复用 | 架构/流程决策 | **中但需决策**。改变发布验证语义或引入新服务[5][22][29][31] | 官方文档 |
| P3 | 更大 Windows runner | 付费 | **待评估**。当前瓶颈是 apt 网络；仅在 Windows 编译成为新瓶颈后考虑[7][16][25][28] | 实测 + 官方价格文档 |
| 不建议近期做 | sccache | 增加基础设施 | 已有 rust-cache，边际收益低[4][20][30] | action README + Cargo 官方文档 |

## 7. 已明确排除的项

- **Docker layer cache**：目标分支 CI 无 Docker 构建[2][3][4][5]。
- **重复实施 Rust/npm 缓存**：4 个 workflow 中凡涉及 Rust/npm 的 job 都已有对应缓存 action[3][4][5][20][18]。
- **重复实施 job 并行/矩阵**：`server-rewrite.yml` 的 OS 矩阵、三个顶层 job 的并行、`release.yml` 的两个 build job 并行都已存在[4][5][14]。
- **重复实施 PR `paths` 过滤**：`server-rewrite.yml` 已有[4]。
- **给 `android-ci.yml` 加 `concurrency`**：已有[3]。

## 未确认与证据边界

1. **Android CI 没有真实 run 数据**：`gh run list --workflow android-ci.yml` 在 fork 与 upstream 均 404（workflow 不在默认分支）[19]。因此第 3.2、3.4 节对 Android 的收益均为定性判断，不写秒数。
2. **apt 慢的根因未完全确定**：日志能确认 100 MB 下载、逐包延迟与镜像来源[6]，但无法区分“镜像瞬时故障”与“持续慢”，因此没有把“换镜像”当作确定有效的措施，而是建议“避免每次都安装”。
3. **Robolectric 210 MB jar 的缓存归属未本地验证**：只根据 workflow 注释[3]与 `setup-gradle` 缓存 `caches/modules-2` 的文档[22]推断，未下载验证其实际落盘路径。
4. **Gradle configuration cache 兼容性未验证**：自定义 `Exec` 任务在 execution time 解析 NDK 路径[12]，与 Gradle 文档的执行期 `Project` 访问限制可能冲突[27]，需本地试跑 `--configuration-cache` 后才能定论。
5. **网页抓取失败**：`docs.github.com/en/actions/how-tos/.../events-that-trigger-workflows`、`docs.github.com/en/actions/reference/workflows-and-actions/dependency-caching` 的部分深链、GitHub billing/larger-runners 页的首次抓取、`docs.github.com/en/actions/.../store-artifacts-...` 均返回 404；`github-hosted-runners` 首次抓取 ECONNRESET（重试成功）。这些失败页面的内容未作为结论依据；改用可达的官方页面、action README 与 `gh` API 原文。
6. **`gh` run 数据的口径**：时间来自 GitHub Actions REST API 的 `startedAt`/`completedAt`，按整秒取整；run `45347340500` 抓取时仍 `in_progress`，未纳入[17]。artifact 下载/上传的排队时间未单独分析。

## 引用

[1] 本地仓库：`git rev-parse integration/v0.6.3-gui-network` = `9a86841907bc8a1ce49d8c72c321172c462abd9a`；`git status --short --branch` 显示当前分支 `docs/v0.6.0-curation`。本调研只通过 `git show integration/v0.6.3-gui-network:<path>` 读取目标分支文件。

[2] `integration/v0.6.3-gui-network:.github/workflows/update_sourceforge.yml` 全文（仅 `workflow_dispatch` + 一行脚本调用）。

[3] `integration/v0.6.3-gui-network:.github/workflows/android-ci.yml`，行 15-20（触发）、25-27（concurrency）、34-77（jvm-tests）、45-51（JDK 17/21）、63-67（`-PnoNative=true` 与 210 MB jar 注释）、79-210（build-assertions，含 112-115 rust-cache、137-150 双 ABI 构建、152-190 断言、196-210 第二次 `assembleRelease`）。

[4] `integration/v0.6.3-gui-network:.github/workflows/server-rewrite.yml`，行 3-26（触发/paths/workflow_call）、28-40（rust 矩阵）、43-61（工具链与 apt）、76-89（fmt/clippy/test/harness/replay）、91-163（windows-packaging，含 101-105 setup-node 缓存、107-109 rust-cache、111-126 构建、147-163 上传）、165-244（linux-packaging，含 175-179 setup-node 缓存、192-194 rust-cache、196-211 构建、225-244 上传）。

[5] `integration/v0.6.3-gui-network:.github/workflows/release.yml`，行 5-15（workflow_dispatch 输入）、20-22（concurrency）、25-53（preflight）、55-58（`build_server` 复用 `server-rewrite.yml`）、60-140（`build_android`，含 93-95 rust-cache、111-131 ABI 循环）、142-163（create_release + download-artifact）。

[6] `gh run view 45337047683 --repo W0n9/audio-share --json jobs`：run `Server rewrite`，分支 `integration/v0.6.3-gui-network`，created `2026-11-24T05:58:44Z`，updated `2026-11-24T06:22:27Z`（1423 s）。job/step 实测：`Rust (ubuntu-24.04)` 1418 s，其中 apt 1300 s、clippy 7 s、test 70 s；`Linux deb/AppImage packaging` 1341 s，其中 apt 1169 s、bundles 117 s；`Windows portable packaging` 145 s（GUI exe 73 s）；`Rust (windows-latest)` 205 s（test 114 s）。`gh run view 45337047683 --log`：`Need to get 100 MB of archives.`、`After this operation, 555 MB of additional disk space will be used.`、镜像 `us-east-2.ec2.archive.ubuntu.com`、逐包 `Get:N` 时间戳。URL: https://github.com/W0n9/audio-share/actions/runs/45337047683

[7] `gh run view 45302650376 --repo W0n9/audio-share --json jobs`：同分支，created `2026-11-24T01:07:42Z`，updated `2026-11-24T01:48:09Z`（2427 s）。`Linux deb/AppImage packaging` 1676 s（apt 1322 s、bundles 285 s）；`Windows portable packaging` 507 s（GUI exe 342 s、as-cmd 68 s）；`Rust (windows-latest)` 550 s（clippy 174 s、test 259 s）；`Rust (ubuntu-24.04)` 404 s（apt 32 s、clippy 111 s、test 210 s）。URL: https://github.com/W0n9/audio-share/actions/runs/45302650376

[8] `gh run view 43001568578 --repo W0n9/audio-share --json jobs`：分支 `main`，created `2026-11-11T10:11:54Z`，updated `2026-11-11T10:14:45Z`（171 s）。Linux apt 28~29 s、bundles 102 s；Windows GUI exe 97 s；`Rust (ubuntu-24.04)` 94 s。URL: https://github.com/W0n9/audio-share/actions/runs/43001568578

[9] `integration/v0.6.3-gui-network:Cargo.toml`，行 1-18（workspace members、`rust-version = "1.90"`、edition 2021）；`gui/src-tauri/Cargo.toml`，行 26-50（Tauri 2.12.5、插件、平台依赖）。

[10] `integration/v0.6.3-gui-network:gui/package.json`，行 1-24（Svelte 5.57.3、Vite 8.3.6、`@tauri-apps/cli` 2.12.4、`build: vite build`）；`gui/src-tauri/tauri.conf.json`，行 7-12（`frontendDist`、`beforeBuildCommand: npm run build`）；`gui/src-tauri/tauri.linux.conf.json`，行 1-14（deb/AppImage 目标）。

[11] `integration/v0.6.3-gui-network:android-client-v2/gradle.properties`，行 9-13（`org.gradle.jvmargs=-Xmx2048m`、`# org.gradle.parallel=true`）；`android-client-v2/gradle/wrapper/gradle-wrapper.properties`，行 1-10（Gradle 9.7.1）。

[12] `integration/v0.6.3-gui-network:android-client-v2/app/build.gradle.kts`，行 15-21（ndkDir）、128-131（abiFilters）、215-218（jvmToolchain 17）、224（`tasks.withType<Test>()`）、271-272（protobuf）、288-345（cargo/strip `Exec` 任务、`onlyIf { !project.hasProperty("noNative") }`、execution time 解析 ndkDir）。

[13] `integration/v0.6.3-gui-network:android-client-v2/native/Cargo.toml`，行 1-5 注释（“Standalone crate — deliberately NOT part of the repo root cargo workspace”）、28-36（prost/quinn/rustls 依赖）；`native/.cargo/config.toml`，行 1-31（开发者机器 NDK 路径与环境变量覆盖）。

[14] GitHub Docs, "Workflow syntax for GitHub Actions"（`jobs.<job_id>.needs`：“run in parallel by default”；`strategy.matrix`；“GitHub Actions does not guarantee the order that matrix jobs will run in.”；`jobs.<job_id>.strategy.max-parallel`：“By default, GitHub will maximize the number of jobs run in parallel depending on runner availability.”；`jobs.<job_id>.strategy.fail-fast`：“applies to the entire matrix.”且“GitHub will cancel all in-progress and queued jobs in the matrix if any job in the matrix fails.”；`on.workflow_call`）. https://docs.github.com/en/actions/writing-workflows/workflow-syntax-for-github-actions

[15] GitHub Docs, "Events that trigger workflows"（`on.<push|pull_request>.<paths|paths-ignore>`：“If at least one path matches a pattern in the `paths` filter, the workflow runs.”；“Path filters are not evaluated for pushes of tags.”）. 目标深链抓取 404，改引同一语法页 https://docs.github.com/en/actions/writing-workflows/workflow-syntax-for-github-actions 的 path-filter 段落。

[16] `gh run view 45156050225 --repo W0n9/audio-share --json jobs`：run `Make Release`，分支 `release/v0.6.2`，created `2026-11-23T10:25:58Z`，updated `2026-11-23T11:10:44Z`（2686 s）。`Rust (ubuntu-24.04)` 796 s（apt 509 s、clippy 81 s、test 154 s）；`Build signed Android v2 APKs` 395 s（NDK 安装 132 s、ABI 循环构建 232 s）；`Windows portable packaging` 545 s（GUI exe 375 s）；`Linux deb/AppImage packaging` 549 s（bundles 284 s）。URL: https://github.com/W0n9/audio-share/actions/runs/45156050225

[17] `gh run list --repo W0n9/audio-share --workflow server-rewrite.yml --limit 5` 与 `gh api repos/W0n9/audio-share/actions/runs`：run `45347340500`（`Make Release`，目标分支，created `2026-11-24T07:21:16Z`）在抓取时 `status=in_progress`。

[18] GitHub, `actions/setup-node` README（行 165-170：“uses `actions/cache` under the hood for caching global packages data”；“The action does not cache `node_modules`”；行 167：`cache-dependency-path`）. https://github.com/actions/setup-node

[19] `gh run list --repo W0n9/audio-share --workflow android-ci.yml`、`gh run list --repo mkckr0/audio-share --workflow android-ci.yml`：均返回 HTTP 404 “workflow android-ci.yml not found on the default branch”。

[20] GitHub, `Swatinem/rust-cache` README（行 124-131：“only caches the dependencies of a crate”；stable Rust 最有效；行 137-171：缓存 `~/.cargo` 与 `./target`，按 job 键控，清理后不缓存 workspace crate；行 185-193：10 GB 总上限与淘汰、base 分支缓存对 PR 可用）. https://github.com/Swatinem/rust-cache

[21] GitHub, `actions/setup-java` README（行 290-298：三类缓存表——dependency `~/.gradle/caches`、wrapper `~/.gradle/wrapper`、JDK；行 298-315：`cache: gradle` 与 key 默认基于 `**/*.gradle*`、`**/gradle.properties`、`**/gradle-wrapper.properties`、`gradle/*.versions.toml`；行 326-329：`cache-dependency-path` 用于 monorepo）. https://github.com/actions/setup-java

[22] GitHub, `gradle/actions` README 与 `docs/setup-gradle.md`（v6 默认 enhanced provider 使用“proprietary technology”、公共仓库免费/私有 Free Preview、`cache-provider: basic` 走 MIT 路径；行 119-136：缓存 Gradle User Home、provider 选择；行 157-178：“only write to the cache from Jobs on the default (`main`/`master`) branch”；行 186-189：避免同时用 `setup-java cache: gradle` 或 `actions/cache` 缓存 GRADLE_USER_HOME；行 221-228：`caches/modules-2`、`caches/transforms-3`、`caches/build-cache-1` 等；行 203-216：GitHub Actions 缓存不可变、分支作用域、restore key）. https://github.com/gradle/actions

[23] GitHub Docs, "Workflow syntax for GitHub Actions"（jobs 默认并行、`needs` 串行）与 `server-rewrite.yml:36-39`（OS 矩阵）、`release.yml:55-63`（两个 build job 仅 `needs: preflight`）. https://docs.github.com/en/actions/writing-workflows/workflow-syntax-for-github-actions

[24] GitHub Docs, "Using concurrency"（“at most one running job or workflow in a concurrency group at any time.”；“To also cancel any currently running job or workflow in the same concurrency group, specify `cancel-in-progress: true`.”）. https://docs.github.com/en/actions/writing-workflows/choosing-what-your-workflow-does/using-concurrency

[25] GitHub Docs, "GitHub-hosted runners"（公共仓库 `ubuntu-24.04` = 4 CPU / 16 GB、私有 = 2 CPU / 8 GB；`windows-latest` 类似；更大 runner “available for organizations and enterprises on GitHub Team and GitHub Enterprise Cloud plans”、“More RAM, CPU, and disk space”、“Larger runners require a valid payment method.”）. https://docs.github.com/en/actions/reference/runners/github-hosted-runners

[26] GitHub Docs, "Caching dependencies to speed up workflows"（默认 “10 GB per repository”、“Any usage beyond 10 GB is billed”；“not been accessed in over 7 days” 淘汰；“order of last access date, from oldest to most recent”；分支/标签作用域；PR 可恢复 base 分支缓存；低信任触发的缓存投毒风险）. https://docs.github.com/en/actions/writing-workflows/choosing-what-your-workflow-does/caching-dependencies-to-speed-up-workflows

[27] Gradle Docs, "Configuration cache"（“caches the result of the configuration phase”；命中时“the entire configuration phase is skipped”；Gradle 10 默认启用；“referencing the `Project` object at execution time” 会在 store 时失败；任务字段需可序列化）. https://docs.gradle.org/current/userguide/configuration_cache.html

[28] GitHub Docs, "About billing for GitHub Actions"（“The use of standard GitHub-hosted runners is free: In public repositories”；Linux 2-core `$0.006`/min、Windows 2-core `$0.010`/min、macOS 3/4-core `$0.062`/min；“Larger runners require a valid payment method.”）. https://docs.github.com/en/billing/managing-billing-for-github-actions/about-billing-for-github-actions

[29] Gradle Docs, "Build cache"（“a cache mechanism that aims to save time by reusing outputs produced by other builds”；“When using a shared build cache for task output caching this even works across developer machines and build agents.”）. https://docs.gradle.org/current/userguide/build_cache.html

[30] Rust/Cargo Docs, "Cargo Build Cache"（“Cargo stores the output of a build into the `target` and `build` directories.”；“A third party tool, sccache, can be used to share built dependencies across different workspaces.”；`export RUSTC_WRAPPER=sccache`）. https://doc.rust-lang.org/cargo/reference/build-cache.html

[31] GitHub, `actions/upload-artifact` README（行 344-346：“Artifacts are retained for 90 days by default.”）与 `actions/download-artifact` README（行 18、81-93：“Download Artifacts from other Workflow Runs or Repositories”，需 `github-token` 与 `run-id`）. https://github.com/actions/upload-artifact 、 https://github.com/actions/download-artifact
