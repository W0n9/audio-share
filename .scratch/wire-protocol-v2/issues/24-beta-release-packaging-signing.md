# 24: Beta release packaging and signing

**What to build:** A signed beta APK and a Server `0.5.0-beta` are packaged in one GitHub Release with checksums, v1 compatibility notes, known limitations, and upgrade/rollback steps. Customers can install the APK and Server without debug-signed builds.

**Blocked by:** 20, 21, 22, 35

**Status:** resolved

- [x] A dedicated release keystore signs the APK; the APK is `dev.audioshare.v2` `0.2.0-beta` (`versionCode` 2).
- [x] Server artifacts are `0.5.0-beta`: Windows portable, Linux deb/AppImage, and `as-cmd`.
- [x] One release contains the APK and Server artifacts with SHA-256 checksums.
- [x] Release notes state v1 compatibility, known beta limitations, and that v1 is unencrypted and trusted-network only.
- [x] Upgrade and rollback steps are documented; the v1 APK remains available.
- [x] No F-Droid publication and no in-app auto-update.
- [ ] Installing from the release and completing one pairing/playback path succeeds on the emulator.
      — **WAIVED** 2026-11-18：维护者按代码等价性接受未复验风险（见 Comments）。

## Answer

本地打包与签名已完成并合入集成分支 `integration/wire-protocol-v2-beta`（merge commit `66d168f`，2026-11-16）。实现提交 `95c5bf0`、`45c52c0`。

**已完成（PASS）**
- Server 全量版本 `0.5.0-beta`：`VERSION`、workspace Cargo、GUI package/Cargo/tauri、interop 锁文件；`as-cmd --version` 与 GUI 显示一致。`get_version.sh -c` 新方案：`0.5.0-beta=500200`、`0.5.0=500999`，不再撞码。
- 专用 release keystore 位于仓库外 `~/.cache/audio-share-beta-keys/audio-share-beta.jks`（目录 0700、文件 0600，alias `audio-share-beta`，RSA 4096）；密码只在仓库外 properties/环境变量，未进入仓库、日志或证据。证书 SHA-256 `90:BE:DC:D1:D8:15:A0:D7:72:89:7E:92:8E:B2:C9:89:4F:6D:1C:A0:38:C5:51:9A:36:DA:6D:B2:7A:A7:A1:3C`。
- 签名 universal APK（arm64-v8a + x86_64，R8，无 test-support/deviceTests）：`dist-android/audio-share-client-v2-0.2.0-beta.apk`，SHA-256 `6da98e4a…845141`；`apksigner` v2/v3 通过，`aapt` 版本/ABI 正确，dex 无 fixture/test harness 路径。测试构建显式改用 debug 签名，不会污染 beta 证书。
- Linux 产物：`dist-linux/` 的 AppImage（`215fb4a5…`）、deb（`2117489c…`）、as-cmd tar.gz（`8276998b…`），SHA-256 校验通过。
- 模拟器（API 35 x86_64，EMULATOR）：安装发布 APK → `as-cmd pair --sas --accept` 配对 → 自动连接 `PLAYING` → Stop `CLOSED` / Server 701；安装包哈希与签名与发布件一致。
- Release notes 草稿：`.scratch/release-notes-v0.5.0-beta.md`；证据：`evidence/ticket-24/`。
- `.github/workflows/release.yml` 已支持 beta APK 与 prerelease draft，仍需人工配置 `ANDROID_KEYSTORE_BASE64` 等 secrets；未执行、未发布。

**仍待人工/CI（因此本工单保持 `ready-for-human`）**
- Windows portable Server：本机无 Windows host、无 `cargo-xwin`/Wine，`NOT RUN`；未用 Linux 产物冒充。
- 正式 GitHub Release 发布：按用户授权仅本地生成，未 push/tag/发布；F-Droid 与自动更新未做。
- Windows 上的 GUI 配对/播放 smoke 属工单 25。

**NOT RUN**：真机、Windows、可听音质/时延/underrun。

## Comments

- 2026-11-16 收尾：发布产物已复制到 worktree 之外并复验 SHA-256：
  - `/home/w0n9/.cache/audio-share-beta-release/dist-android/audio-share-client-v2-0.2.0-beta.apk`
  - `/home/w0n9/.cache/audio-share-beta-release/dist-linux/{AppImage,deb,as-cmd tar.gz}`
  - `/home/w0n9/.cache/audio-share-beta-release/release-notes-v0.5.0-beta.md`
  所有 `.sha256` 在对应目录内校验成功。implementer worktree 已清理，不影响上述产物。

- 2026-11-16 追加：Windows x64 产物已由 Linux `cargo-xwin` 交叉构建，保存在
  `~/.cache/audio-share-beta-release/dist-windows/`：
  - `AudioShareServer.exe` SHA-256 `2117e7af…b6712`（PE32+ GUI）
  - `audio-share-server-cmd-windows.zip` SHA-256 `857c5e05…874f31`（内含 `as-cmd.exe`）
  两者均未在 Windows/Wine 上运行，状态为 built / NOT RUN；Windows 验收步骤见
  `.scratch/wire-protocol-v2/windows-handoff.md`。工单继续保持 `ready-for-human`。

## Comments (post-fix rebuild, 2026-11-17)

工单 29 修复后（HEAD `a380ee0`），Linux 与 Windows 产物已重建并复验；APK 未变。

```
97bc00e1dc3c896539899e28925c746ef1f327a610d20e4601d837524c2b4a92  AudioShareServer_0.5.0-beta_amd64.AppImage
1e093c5d82a44979640e6fcd2c7cda690c07046046d133098060168f807258eb  AudioShareServer_0.5.0-beta_amd64.deb
c6b3b16385a85ea42963238620dabdd8a108ee140228f090ca076b8ef8be29e4  audio-share-server-cmd-linux.tar.gz
e4575d25b19ab12fcdf626d1a592f52ac382a2f052c05e6bd82702a9c3c8e97c  AudioShareServer.exe
2bc4ceb18bd0103286c22029f602fa1aed7e8329c4faf9bc56a04a21d9d8550d  audio-share-server-cmd-windows.zip
```

Windows zip 内 `as-cmd.exe` SHA-256 `d2917467b28eb023e8dda7e675bd10a37595111fd14400219c4a966020e73ba5`。
清单：`~/.cache/audio-share-beta-release/post-fix-artifacts.txt`。Windows 产物仍为
built / NOT RUN；正式发布仍未执行。

## Comments (APK post-fix rebuild, 2026-11-17)

工单 30（客户端配对状态重置/陈旧横幅）修复后，签名 universal APK 已重建：

```
c1d1b23634abb05bf760bcb975bd442e65abc674610dfac11140f6dd7992af58  audio-share-client-v2-0.2.0-beta.apk
```

仍是 `dev.audioshare.v2` / `0.2.0-beta` / versionCode 2 / arm64-v8a + x86_64，
v2+v3 签名，证书 `90bedcd1…a13c`；不含 test-support 字符串。旧 APK
`6da98e4a…845141` 保留为 `.prev`，已被取代。Server 产物未变。

## Comments (final Server rebuild, 2026-11-17)

按最终 HEAD `0aec442`（含工单 29/30 与 QR 一步配对/失效邀请恢复）重建 Server 产物；
APK 保持 `c1d1b236…2af58`。最终清单见 `~/.cache/audio-share-beta-release/post-fix-artifacts.txt`。

```
3b804c054269198faeb5fc17e014f18fc33a947a6724c50858a72765531da763  AudioShareServer_0.5.0-beta_amd64.AppImage
60382a4d0edbfac37b55ec6d7af82ee8e741f827ffe2d41dd27748f7f08d067b  AudioShareServer_0.5.0-beta_amd64.deb
04823095bd9e523f7b9b0c9566906dda198a3403aec6bd01a870a6b20cbb6df9  audio-share-server-cmd-linux.tar.gz
0a7dd9cef054e0d7a7175391144ae8d261a70cee675b8d7a11b15ce1c3dd4ca7  AudioShareServer.exe
afe3b660a54172d9bbad8283db8e53f59ca1ee2fb7e110fb126d16c23233ff72  audio-share-server-cmd-windows.zip
```

Windows zip 内 `as-cmd.exe` SHA-256 `3a50d8bee9e0fbe681b1922175c3e45835bbe2d09bce113f707f9c8bd57e2420`。
Windows 产物 built / NOT RUN；正式发布仍未执行。工单保持 `ready-for-human`。

- 2026-11-17 状态核验：Windows 便携产物已由工单 25 在 Windows 11 上实跑（GUI `0.5.0-beta`，
  `AudioShareServer.exe` SHA-256 `e4575d25…c8e97c` 与本地产物一致），原“Windows NOT RUN”备注
  由该证据取代。本工单保持 `ready-for-human` 的唯一原因是正式 GitHub Release 尚未发布
  （`ANDROID_KEYSTORE_BASE64` 等 secrets 未配置），发布动作需人工执行。

- 2026-11-18 发布前置：`v0.5.0-beta` tag 现在落后集成分支 41 个提交（含 Android 现代化），
  发布前必须先裁决 [35](35-beta-tag-placement-decision.md)。本工单已加入 `Blocked by: 35`；
  裁决前不要 push tag 或创建 GitHub Release。

- 2026-11-18 裁决落地：工单 35 选 B，`v0.5.0-beta` 已在本地删除并重建指向 `362d023`
  （含 Android 现代化 + 工单 33），未 push。产物从该 tip 重建：
  - 新 APK `2024ae1423090670f6b46b92277fbed39df40dcc8f0ad74d4b801fa44722677a`
    （`0.2.0-beta` / versionCode 2 / arm64-v8a + x86_64 / beta 证书 `90bedcd1…a13c`）；
    旧 APK `c1d1b236…2af58` 保留为 `.prev`。
  - Server 产物重建中；完成后更新 manifest 与 release notes 的哈希。
  真机/Windows 证据需绑定新哈希后重跑，完成前本工单保持 `ready-for-human`。

- 2026-11-18 产物重建完成（`362d023`，manifest：`~/.cache/audio-share-beta-release/post-modernization-artifacts.txt`）：
  - APK `2024ae1423090670f6b46b92277fbed39df40dcc8f0ad74d4b801fa44722677a`（7,595,154 B）
  - deb `2b8a4ba246791b5f09a0b123ae7ba2b698c54561eca1d963ea039866678b5f9d`
  - AppImage `78e91548ab188784df97b40e253b905be12a15e89f5f5e0d7d1a628f80a7b085`
  - as-cmd linux `66dd752065fd5ec54cfb429d6132d8866fcffd21cac369d17543cf63171aec5a`
  - Windows `AudioShareServer.exe` `78874511654260e77a6a5c40bcb0c504a71e8dcd81fb458bce22ab0816c6805c`
  - Windows zip `d5f9e977702670520547cc27b2633bbb9c443956fb25ababdd234cae4602e9a8`
- 在 `362d023` 重跑 JVM：`testDebugUnitTest` 250/250、`testReleaseUnitTest` 250/250。
- 仍为 NOT RUN：真机全矩阵、Windows 执行（含 33 的 701 日志门）、该 release APK 的
  EMULATOR 矩阵、Release 发布动作。完成前保持 `ready-for-human`，不 push tag、不创建 Release。

## 2026-11-18 WAIVED decision (maintainer)

维护者裁决：**按代码等价性 `WAIVED` 最终产物上的复验**，不要求在发布前重跑真机、Windows 或
EMULATOR。依据与边界如下（`WAIVED` 不是 `PASS`，不得改写为验证通过）：

**代码等价性依据**
- Android App 源码自 `86b2635` 后未再改动；`362d023` 相对它的新增只有工单 33（Server/GUI
  Rust 侧日志）。`f6976ef`（配对交接）与 `f93ba15`（重连计数）已在 `86b2635` 中，并有 JVM 覆盖。
- Server 侧自上次全量验收（`0aec442`）后的改动是 Android 现代化（不影响 Server 协议/媒体）
  与工单 33（仅新增日志事件）。
- 新产物在 `362d023` 上重跑了 JVM/Robolectric：250/250 × 2 variant PASS。

**因此 WAIVED 的项（real device / Windows / EMULATOR on final hashes）**
- 真机：QR+SAS 配对导航修复（`f6976ef`）、5–10 分钟播放、锁屏、断网重连计数（`f93ba15`）、
  Stop、撤销→204、重配对、身份重置→205、30 分钟、v1 共存。
- Windows：新 GUI 的新配对流程、5 分钟播放、`server.log` 的 701/配对事件。
- 最终 release APK 的 EMULATOR 安装/配对/播放矩阵。

**仍为 `NOT RUN`，未被本豁免升级为 PASS**
- 新产物上的真机与 Windows 执行。
- 工单 33 的 Windows 701 日志验证（该票保持 `ready-for-human`；本豁免只针对发布门槛，
  不验证 33 的新代码路径）。
- 正式 GitHub Release 发布（secrets 与 push 仍是人工动作）。

发布说明必须继续声明：上述项基于更早构建的 PASS，最终产物未复跑；不得写成已在最终产物验证。

- 2026-11-18 发布准备已执行（仍为 draft，未 publish）：
  - tag 已 push 到 **fork** `W0n9/audio-share`（不是 `origin`/上游）：`refs/tags/v0.5.0-beta`
    → tag object `6d9363f2…` → commit `362d023`。
  - draft release：`https://github.com/W0n9/audio-share/releases/tag/untagged-49c91ed9993e54671e17`
    （id `450126052`，tagName `v0.5.0-beta`，7 个资产）。
  - 7 个资产的 GitHub API `digest` 与本地 `SHA256SUMS` 逐一匹配。
  - 剩余人工动作：在草稿中审阅渲染与资产，然后 `gh release edit v0.5.0-beta --repo
    W0n9/audio-share --draft=false` 发布。发布前本工单保持 `ready-for-human`。

- 2026-11-18 **已发布**：`https://github.com/W0n9/audio-share/releases/tag/v0.5.0-beta`
  （`isDraft=false`、`isPrerelease=true`、publishedAt `2026-11-18T15:03:22Z`、7 个资产）。
  发布后仅更新了 release notes 的状态行，资产未变。最终产物复验按本工单 2026-11-18 裁决
  记为 `WAIVED`，不是 `PASS`。工单 33 的 Windows 701 验证仍是独立的人工项。
