# v0.6.3 发布边界与验收记录

0.6.3 = 0.6.2（配对管理）+ GUI/网络配置修复（配对对话框、确认框、Host 控件、二维码地址）+ Android v2 启动器图标。Wire Protocol v2、音频路径与支持的 Client 数量不变。发布说明见 `docs/releases/v0.6.3.md`。

## 来源

| 项 | 值 |
| --- | --- |
| 基线 | `integration/v0.6.3-gui-network`（`d6d1879`），基于 `release/v0.6.2` |
| 版本 | `0.6.3`，Android `versionCode 6` |
| 发布分支 | `release/v0.6.3`（本地，未推送） |
| 构建提交 | squash 后的发布 tip（推送后以远端 tag `v0.6.3` 为准） |

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

## 构建与发布状态

- `release/v0.6.3` 与 tag `v0.6.3` 尚未推送。
- 正式 Make Release（`version=0.6.3`、`create_draft=false`）尚未运行。
- GitHub Release / draft 尚未创建。
- 合回 main / fork main 的策略尚未确定。
- CI 稳定性 ticket（Windows UDP 端口偶发失败）尚未处理；Linux runner 在 `sudo apt-get update` 上超时（0.6.2 首次构建与 run 45347340500 各一次）尚未单独登记。

## 待决

- `worktree-agent-*` merge 提交已在本地 squash（tree 与 squash 前完全一致）；`integration/v0.6.3-gui-network` 保持原历史不动。
- Linux apt 超时已并入 `.scratch/ci-stability/issues/01-windows-udp-port-flake.md`。
- 是否合回 main / fork main。
