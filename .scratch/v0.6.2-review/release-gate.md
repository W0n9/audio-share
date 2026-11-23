# v0.6.2 发布边界与验收记录

0.6.2 = v0.6.1（文案/元数据修正）+ 配对管理 UX（撤销、恢复、重配、忘记）。
Wire Protocol v2、音频路径与验证边界不变。发布说明见
[`docs/releases/v0.6.2.md`](../../docs/releases/v0.6.2.md)。证据术语见
[证据政策](../../docs/agents/evidence-policy.md)。

## 来源

| 项 | 值 |
| --- | --- |
| 基线 | tag `v0.6.1`（`019acda`） |
| 配对 UX 合并 | `integration/pairing-ux`（`cbc9c52`）→ merge `1e67589` |
| 版本 | `0.6.2`，Android `versionCode 5` |
| 分支 | `release/v0.6.2` |

## 0.6.1 的实际状态（发布前必须知道）

- tag `v0.6.1` 已存在并推到 `fork`；CI 运行 `45145447815`（`version=0.6.1`，
  `create_draft=false`）6 个构建作业全部成功，7 个产物与 `.sha256` 已本地核对。
- **GitHub 上没有 v0.6.1 Release**（`releases/tags/v0.6.1` 为 404），`gh release
  list` 的 Latest 仍是已发布的 `v0.6.0`。
- 0.6.1 的真机安装/播放与 GUI 截图核对为 `NOT RUN`。
- 因此 0.6.2 的发布说明写明“0.6.1 已打标签/构建但未发 Release，0.6.2 取代它”。

## 自动化证据（配对 UX 集成分支）

| 检查 | 结果 |
| --- | --- |
| Rust `v2-transport/v2-runtime/as-cmd` | `PASS` — 357（349 + 8 `pairing_live`） |
| GUI Rust | `PASS` — 114 |
| Android JVM | `PASS` — 332 |
| `cargo fmt` / `clippy -D warnings` | `PASS` |
| `svelte-check` / Vite production build | `PASS` — 101 files, 0 errors |

## 模拟器与真机

- `EMULATOR`：撤销后 204；SAS 重配命中已存身份并触发服务端显式 Restore；
  恢复后同一 `client_id`、`created_at` 保留；凭据 reload 通过。
- `NOT RUN`：QR 首次配对、QR 重配、Forget UI、完整手动清单、真机。
- 这些结果覆盖的是源码与模拟器，不是 0.6.2 产物的真机验收。

## 0.6.2 产物验收（待执行）

1. 运行 `Make Release`，`version=0.6.2`，`create_draft=false`；确认 6 个构建作业成功。
2. 逐一核对 7 个产物与 `.sha256`：APK `0.6.2` / `versionCode 5`、单 ABI、无
   test-support；Windows exe 版本/Company/LegalCopyright；deb `Version 0.6.2`、
   `Maintainer: Audio Share contributors`、`Homepage: W0n9/audio-share`；AppImage；
   Linux/Windows `as-cmd` 版本。
3. 真机安装与播放、GUI 关于页截图仍按证据政策记录 `PASS` 或 `NOT RUN`。
4. 远端 tag `v0.6.2` 必须指向实际构建提交，再创建与发布 Release。

## 未验证项（沿用 0.6.0/0.6.1 的边界）

声学时延基线、60 分钟单 Client soak、真实网络损伤、设备时钟漂移注入、听感 A/B
均未运行或已豁免；不承诺 p95/p99 可听时延。多 Client 隔离与完整迁移/回滚不在本次范围。
