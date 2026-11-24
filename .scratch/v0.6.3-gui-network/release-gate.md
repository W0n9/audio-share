# v0.6.3 发布边界与验收记录

0.6.3 = 0.6.2（配对管理）+ GUI/网络配置修复（配对对话框、确认框、Host 控件、二维码地址）+ Android v2 启动器图标。Wire Protocol v2、音频路径与支持的 Client 数量不变。发布说明见 `docs/releases/v0.6.3.md`。

## 来源

| 项 | 值 |
| --- | --- |
| 基线 | `integration/v0.6.3-gui-network`（`d6d1879`），基于 `release/v0.6.2` |
| 版本 | `0.6.3`，Android `versionCode 6` |
| 发布分支 | `release/v0.6.3`（已推送 fork，tip `bc60e6a`） |
| 构建提交 | `bc60e6a`（annotated tag `v0.6.3` 指向此提交） |

## 自动化证据

| 检查 | 结果 |
| --- | --- |
| Rust `cargo fmt --check` / `cargo clippy --workspace --all-targets -- -D warnings` | `PASS` |
| Rust `cargo test --workspace` | `PASS` — 676 passed / 0 failed |
| GUI `svelte-check` | `PASS` — 101 files, 0 errors / 0 warnings |
| GUI Vite production build | `PASS` |
| Android v2 JVM tests（JDK 21, offline） | `PASS` — 332 tests / 0 failures / 0 errors / 0 skipped |
| `scripts/check-release-version.sh 0.6.3` | `PASS` |
| `scripts/release-workflow.test.sh` | `PASS` |

## 真机与证据边界

- 维护者已报告 01–05 GUI/网络修复通过，以及 Android v2 图标显示正常、比例与 v1 一致。
- 这些结果覆盖的是 0.6.2 版本号、SHA 分别为 `095f2d65…` / `4d43e376…` 的 Windows exe 与 run 45359916642 的签名 APK；01/02/05 未在最终 exe 上复测。
- 最终 0.6.3 版本号的产物尚未构建，真机安装/播放与 GUI 截图核对均记为 `NOT RUN`；拿到正式产物后应做一次冒烟测试。
- 声学时延基线、60 分钟单 Client soak、真实网络损伤、设备时钟漂移注入、听感 A/B 均未运行或已豁免；多 Client 隔离与完整迁移/回滚不在本次范围。

## 正式产物验收

正式 Make Release [45366095967](https://github.com/W0n9/audio-share/actions/runs/45366095967)（attempt 1，`version=0.6.3`、`create_draft=false`，ref=`v0.6.3`）**全绿**：前置检查与 6 个构建 job 全部 PASS，Linux job 未再遇到 apt 超时；draft job 按参数跳过。产物位于 `.build-artifacts/v0.6.3-release/final/`。

| 项目 | 结果 |
| --- | --- |
| 7 个产物与 7 个 `.sha256` sidecar | `PASS`（逐目录 `sha256sum -c` 全部成功） |
| APK：`0.6.3` / `versionCode 6`、双 ABI | `PASS`（arm64-v8a、x86_64 各自核对） |
| APK 启动器图标 | `PASS`（`application-icon` 指向自适应图标，foreground/background/monochrome） |
| APK 签名：`CN=Audio Share Beta`，SHA-256 `90bedcd1…a13c` | `PASS`；与 v0.6.2 发布 APK 证书一致，可直接覆盖安装 |
| deb：`Version 0.6.3`、`Maintainer: Audio Share contributors`、`Homepage: W0n9/audio-share` | `PASS` |
| Windows GUI exe：PE32+ x86-64，含 `0.6.3`、`© 2022-2026 Audio Share contributors` | `PASS` |
| AppImage：内部二进制含 `0.6.3` / `Audio Share Server 0.6.3 starting` / 贡献者署名 | `PASS` |
| Linux `as-cmd`：`version: 0.6.3`、`url: W0n9/audio-share` | `PASS` |
| Windows `as-cmd`：含 `0.6.3` 与仓库 URL | `PASS` |
| 0.6.3 正式产物真机安装与播放 | `NOT RUN` |
| GUI 关于页界面截图核对 | `NOT RUN` |

关键 SHA-256：
- `audio-share-client-v2-0.6.3-arm64-v8a.apk` — `e667ca37e8aedb1f401b47cdc55d864b9640727138f5678b02b0d945ae90fa29`
- `AudioShareServer.exe` — `673c2648f902358704e83b43dc48930334a4130b5e2f390adb535e8105a43a05`

## 构建与发布状态

- `release/v0.6.3` 已推送 fork，tip `bc60e6a`。
- annotated tag `v0.6.3` 已推送 fork，peeled 指向 `bc60e6a`。
- 正式 Make Release run 45366095967 attempt 1 全绿，产物已按上表验收。
- GitHub Release / draft 尚未创建（本次 `create_draft=false`）。
- 合回 main / fork main 的策略尚未确定。
- CI 稳定性 ticket 已并入 Linux apt 超时证据（Windows UDP 端口问题仍待实现）。

## 已完成/待决

- `worktree-agent-*` merge 提交已在本地 squash（tree 与 squash 前完全一致）；`integration/v0.6.3-gui-network` 保持原历史不动。
- Linux apt 超时已并入 `.scratch/ci-stability/issues/01-windows-udp-port-flake.md`。
- 待决：是否创建 GitHub Release / draft、是否合回 main / fork main、正式产物真机冒烟。

## GitHub draft

- 文档提交 `9e455c1` 已推送 fork `release/v0.6.3`；tag `v0.6.3` 仍指向 `bc60e6a`，未移动。
- Make Release run 45373863456 attempt 1 全绿（CI 中 tag 前置检查通过），`create_draft=true`。
- GitHub draft release 已创建：v0.6.3，14 个附件（7 产物 + 7 `.sha256`），`isDraft=true`。URL: https://github.com/W0n9/audio-share/releases/tag/untagged-022ea23635a81b253310
- 真机冒烟（`NOT RUN`）与合回 main / fork main 策略仍未完成/未决定。
