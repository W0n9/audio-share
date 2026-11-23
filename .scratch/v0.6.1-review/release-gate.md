# v0.6.1 发布边界与验收记录

v0.6.1 跟随已发布的 0.6.0 构建（`publishedAt 2026-11-23T08:02:33Z`），是 0.6.x 线的文案与元数据修正版。0.6.0 的关于页仍带 beta 字样与上游版权方，因此需要这个修正版本。0.6.1 不改变 Wire Protocol v2、音频路径或验证边界，只修正对外文案与署名：

- v2 Android 关于页移除 beta 字样，纠正抖动缓冲/丢包掩蔽/漂移校正的过时说明，新增版权行
- GUI 关于页与打包元数据统一为 `Audio Share contributors`（2022-2026，Apache-2.0）
- GUI 主页、更新检查、`as-cmd --version`、deb homepage 指向 `W0n9/audio-share`

发布说明见 [`docs/releases/v0.6.1.md`](../../docs/releases/v0.6.1.md)。证据术语见[证据政策](../../docs/agents/evidence-policy.md)。

## 验收运行

最终验收运行 [45145447815](https://github.com/W0n9/audio-share/actions/runs/45145447815)，提交 `a82c18b`，参数 `version=0.6.1`、`create_draft=false`。6 个构建作业全部成功，draft 作业按预期跳过。此前两次试跑 [45135570907](https://github.com/W0n9/audio-share/actions/runs/45135570907) 与 [45143516492](https://github.com/W0n9/audio-share/actions/runs/45143516492) 发现并修复了 Windows PE 默认 `CompanyName=github`、缺 `LegalCopyright` 以及 deb `Maintainer` 仍为 `Audio Share maintainers` 的问题。

| 项目 | 结果 |
| --- | --- |
| 7 个产物与其 `.sha256` sidecar | `PASS`（本地逐一校验） |
| APK：`0.6.1` / `versionCode 4`、单 ABI、`CN=Audio Share Beta`、无 test-support | `PASS`（arm64-v8a、x86_64 各自核对） |
| APK 关于页：`About Audio Share v2` / `关于 Audio Share v2`、版权行 | `PASS` |
| Windows exe：File/Product Version `0.6.1`、`Company Name` 与 `Legal Copyright` 为 Audio Share contributors | `PASS` |
| deb：`Version 0.6.1`、`Maintainer: Audio Share contributors`、`Homepage: W0n9/audio-share` | `PASS` |
| AppImage：`0.6.1`，内部二进制含新版权与更新 URL | `PASS` |
| Linux / Windows `as-cmd`：`0.6.1`、homepage `W0n9/audio-share` | `PASS` |
| 0.6.1 产物真机安装与播放 | `NOT RUN` |
| GUI 关于页界面截图核对 | `NOT RUN`（二进制文案已确认） |

0.6.0 draft 产物曾在真机安装播放与 Windows 运行上 `PASS`，升级路径 `WAIVED`；这些结果只覆盖 0.6.0 构建，不外推到 0.6.1。

## 未验证项（沿用 0.6.0 的边界）

声学时延基线、60 分钟单 Client soak、真实网络损伤矩阵、设备时钟漂移注入、听感 A/B 均未运行或已豁免；不承诺 p95/p99 可听时延指标。多 Client 隔离与完整迁移/回滚验收不在本次范围。
