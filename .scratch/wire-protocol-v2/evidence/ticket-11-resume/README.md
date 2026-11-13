# 工单 11：前台 Service 回归 — 2026-11-13

基线 Android 代码：`83bfd55`。实现分支：`ticket/11-service-regression`，已先合入
集成分支 `ee41d47`（工单 12）。真机：Xiaomi M2007J22C，Android 12 / API 31，
arm64-v8a，经 tailscale/adb。测试使用全新临时 Server identity、隔离授权文件及
显式 fixture test-support 构建；没有提交任何私钥或完整 TLS trace。

## 反馈循环与根因

原始 release APK 的初次认证/协商能到达 READY，本次未复现初次连接持续 IDLE，
也未观察到 JNI lookup/R8 或 native unwind 崩溃。不能将历史截图一概解释为线程崩溃。

确定的缺陷是 **Service 不保存并重放最近状态**：真实会话到达 READY 后，新本地
Binder 监听者注册时得到 `null`，必须等下一次轮询。测试在 setter 返回的同一回调
中保存立即观察值，后续轮询无法把 RED 变成假通过。

- `device-rebind-red.log`、`device-rebind-red-repeat.log`：恢复原始 Service/轮询行为，
  仅开放 Binder 公共 getter 作为测试 seam；两次都因立即观察值 `null` 而失败。
- `device-rebind-green.log`：Service 保存最近状态并同步重放后，同一真实设备测试通过。
- `device-snapshot-red.log` / `device-snapshot-green.log`：公开快照与 native 实际状态
  同步的独立 RED/GREEN；快照先停留 IDLE，再由实际轮询更新成 READY。没有硬写 READY。

复现命令（安装测试构建后；环境变量只指向 public config / 测试结果）：

```sh
python3 -I android-client-v2/scripts/service-device-test.py \
  --serial "$SERIAL" --config "$PUBLIC_CONFIG" --test rebind --output "$RESULT_FILE"
```

MIUI 曾拒绝 USB 安装测试 APK；用户确认后重新请求安装成功。AndroidX 测试 runner
与 minified APK 的已裁剪 Kotlin runtime 不兼容，且 MIUI 禁止测试进程从后台弹出
Activity。这些是工装失败，不记作行为 RED。最终工装使用 Android 平台 Instrumentation，
并由宿主显式 `adb am start` 打开真实 Activity，不修改手机权限、不使用 root 绕过。

## 真机结果

| 验证 | 结果 | 证据 |
|---|---|---|
| 真实认证/协商 → READY | PASS | `device-candidate-control-3.log`、`device-base-control.log` |
| READY → 新监听者同步重放 + 公开快照 | PASS | `device-snapshot-green.log` |
| Activity.recreate 后仍显示实际 READY | PASS | `device-recreate-green.log` |
| 真实锁屏 2 s → 唤醒，前台 Service 与 READY 保留 | PASS | `device-lock-result.log`，Server 连接计数 24→24 |
| ACTION_STOP → CLOSED 与实际 foreground 清理 | PASS | `device-stop-foreground-green.log` |
| Server 收到正常关闭码 701 | PASS | `server-stop-701.log` |

设备工装针对 R8 release APK。仅 `-PdeviceTests=true` 时保留独立测试 APK 所消费的
公开 Binder API；生产 `assembleRelease` 不使用这些 keep rules，也不编入 fixture 身份。

## 自动化回归

- `cargo test --workspace`：**463 PASS / 0 FAIL / 4 既有 ignored**，没有人为过滤用例。
- Android native：默认生产配置 **102 PASS**；`--features test-support` **102 PASS**。
- Kotlin/JVM：**31 PASS**（其中公开 snapshot/replay 测试 5 项）。
- 默认 `assembleRelease`：**PASS**，没有 `testSupport` / `deviceTests` 参数。
- 前次两个 IPv6/default-port 用例被既有 65531 Server 占用。用户明确确认该进程已无用，
  授权 SIGINT 停止并要求不恢复；释放端口后两项均 PASS。独立 network namespace 被宿主
  拒绝，未绕过该限制。

汇总见 `test-summary.json`。4 个既有 ignored 用例不是本次引入或临时跳过。

## frozen v0.3.4 APK smoke

未重建或更新旧 APK。设备已有版本 `0.3.4` / `versionCode=3004`，SHA-256 与已记录
发布件完全一致：`77cc93d32a3e6b6117b9c202ff5be69082ea5711087c23e6877cef5ac4a471b4`。

旧 APK 连接本次 `--fake-capture` v1 Server，界面显示“已开始”，真实 AudioTrack
为 **F32 / stereo / 48000 Hz**。命令中的请求格式不作为实际格式：synthetic backend
在 `capture/mod.rs:317-329` 固定使用 F32；`GET_FORMAT` hex 为
`0100000008000000080110021880f702`。wire 兼容 probe 的 PCM block alignment、MTU
和收包检查通过，见 `v034-wire-compat-probe.json`。旧 APK 的地址已恢复为测试前的值，
本次 v1 smoke Server 已正常停止。

首次 probe 默认要求 440 Hz tone，因此整体 FAIL；synthetic backend 实际产生
32-frame sawtooth（`capture/mod.rs:219-224`），不是 440 Hz sine。该失败保留在
`v034-server-probe.json`，不能称为音质 PASS。随后明确使用 `--tone-hz 0` 仅做 wire
兼容检查。没有重新做听觉、underrun、60 分钟音质矩阵或 Windows 验收。

## 范围

只修前台控制会话状态/重放，不改变 Wire Protocol、v1、媒体播放、Keystore、生产 pairing
或 GUI。native 源码未修改；对 native 生命周期的只读核查不等于完整安全审计。
