# Wire Protocol v2 — Android 模拟器开发工作流

Status: working draft（2026-11-16）— 不改变任何 accepted 规范或验收门槛
Feature: wire-protocol-v2
相关: `roadmap.md` §3/§4（证据分级与 NOT RUN 项）、`m5-proposal.md`、`scripts/service-device-test.py`

## 0. 定位与证据等级（需维护者裁决）

模拟器用于**开发迭代和提前暴露问题**，不是真机验收替代品。`roadmap.md` §7.3 要求区分
`PASS` / `FAIL` / `NOT RUN` / `WAIVED`，不得把 host stub 或单端测试冒充真机 PASS。

建议新增一个并列状态：

- `EMULATOR` — 在 Android 模拟器上验证通过，**真机仍未运行**。

在维护者书面裁决前，本清单产生的所有模拟器结果一律记录为
「模拟器已验证，真机 NOT RUN」，不得改写 `roadmap.md` 中的 `WAIVED` / `NOT RUN` 项。
以下项目**永远需要真机**，不受本工作流影响：

- 端到端可听时延 p95/p99、underrun 真实性、音质、AudioTrack 缓冲尺寸实验；
- `spec.md` 明写的 “at least two **real** Android Clients concurrently for the 60-minute scenario”；
- MIUI/厂商特有行为（如 `Network.bindSocket` EPERM）、真实 Wi-Fi↔蜂窝切换；
- 最终 release candidate 验收、IPv6/动态 MTU 的真实路径。

## 1. 环境前置

本机（2026-11-16 核对）：

- `/etc/group` 已包含 `kvm:x:108:w0n9`，但当前登录会话可能尚未继承该组；
  若 `python3 -c "open('/dev/kvm','rb+')"` 报 `Permission denied`，用 `sg kvm -c '...'`
  启动模拟器即可，无需重新登录（重新登录后可直接运行）。
- NDK `27.2.12479018` 提供 `x86_64-linux-android26-clang`；
- Java 17、`adb` 37.0.1、`sdkmanager`/`avdmanager` 可用、emulator 37.2.13。

一次性安装：

```bash
export ANDROID_HOME="$HOME/Android/Sdk"
export PATH="$ANDROID_HOME/platform-tools:$ANDROID_HOME/cmdline-tools/latest/bin:$ANDROID_HOME/emulator:$PATH"

yes | sdkmanager --licenses
sdkmanager --install "emulator" \
  "system-images;android-26;google_apis;x86_64" \
  "system-images;android-35;google_apis;x86_64"

rustup target add x86_64-linux-android
```

## 2. ABI 选择

- 默认仍是 **`arm64-v8a`**，面向真机；`assembleRelease` 的对外产物不变。
- 模拟器构建显式加 `-PandroidAbi=x86_64`，原生库与 jniLibs 目录随之切换。

改动位置：

| 文件 | 内容 |
|---|---|
| `android-client-v2/app/build.gradle.kts` | `androidAbi` 属性、`abiFilters`、cargo 目标、jniLibs 目录 |
| `android-client-v2/native/.cargo/config.toml` | `x86_64-linux-android` linker + `CC_`/`AR_` |

注意两个坑：

- `jniLibs.srcDir` 必须保持父目录 `build/generated/jniLibs`（内部是 `<abi>/lib*.so`）；
  指向 ABI 子目录会让 AGP 把 `.so` 判成「not an ABI」并沿用旧产物。ABI 由 `abiFilters` 选择。
- 切换 `-PandroidAbi` 后建议 `clean`，清掉 `merged_jni_libs` 里另一条 ABI 的残留。

构建命令：

```bash
cd android-client-v2
./gradlew assembleRelease -PandroidAbi=x86_64 -PtestSupport=true -PdeviceTests=true
# APK: app/build/outputs/apk/release/app-release.apk
```

> `-PtestSupport=true -PdeviceTests=true` 仅供测试 APK；**不得**用于任何对外构建。
> 默认 `./gradlew assembleRelease` 不带这两个参数，仍是 arm64-v8a。

## 3. 创建 AVD

```bash
echo no | avdmanager create avd -n audio-share-api26 \
  -k "system-images;android-26;google_apis;x86_64" -d pixel_2
echo no | avdmanager create avd -n audio-share-api35 \
  -k "system-images;android-35;google_apis;x86_64" -d pixel_6

avdmanager list avd
```

API 26 AVD 用来补「API 26 运行时」这一 WAIVED 项；API 35 AVD 跟当前 `targetSdk = 35`。

## 4. 启动与检查

无窗口（UI/协议/生命周期回归）：

```bash
# 若当前会话未继承 kvm 组：
sg kvm -c 'export ANDROID_HOME="$HOME/Android/Sdk"; \
  export PATH="$ANDROID_HOME/platform-tools:$ANDROID_HOME/emulator:$PATH"; \
  exec emulator -avd audio-share-api26 -no-window -no-audio -no-snapshot \
    -no-boot-anim -gpu swiftshader_indirect'
# 已继承 kvm 组时直接：
# emulator -avd audio-share-api26 -no-window -no-audio -no-snapshot \
#   -no-boot-anim -gpu swiftshader_indirect &
adb wait-for-device
adb shell 'while [ "$(getprop sys.boot_completed)" != "1" ]; do sleep 1; done'
adb devices
```

需要 AudioTrack 功能路径时去掉 `-no-audio`（但音频时序不代表真机，音质/时延仍属真机）：

```bash
emulator -avd audio-share-api35 -no-window -no-snapshot -gpu swiftshader_indirect &
```

## 5. 首批可跑的回归

前台 Service / 生命周期（复用现成脚本，只是把 serial 换成模拟器）：

```bash
python3 android-client-v2/scripts/service-device-test.py \
  --serial emulator-5554 \
  --config /path/to/config.json \
  --output /tmp/emulator-ready.json \
  --test ready
# rebind / recreate / lock / stop 同理
```

直接驱动 instrumentation：

```bash
adb -s emulator-5554 shell am instrument -w -r \
  -e case ready \
  dev.audioshare.v2.test/dev.audioshare.v2.ControlSessionInstrumentation
```

看日志与状态（native 日志 tag 是 `audio_share_client_v2..`，不是 `AudioShareV2`）：

```bash
adb -s emulator-5554 logcat -d | \
  grep -iE "audio_share_client_v2|AndroidRuntime|UnsatisfiedLink"
adb -s emulator-5554 logcat --pid="$(adb -s emulator-5554 shell pidof dev.audioshare.v2)"
adb -s emulator-5554 shell dumpsys activity services dev.audioshare.v2
adb -s emulator-5554 exec-out screencap -p > /tmp/emulator.png
```

## 6. 工序分级

| 工序 | 模拟器 | 备注 |
|---|---|---|
| 最小 UI、地址输入、播放/停止、深色模式、无障碍 | ✅ | 最适合 |
| 生产配对（SAS、fingerprint、QR payload 注入） | ✅ | 不需要真机摄像头 |
| 认证失败、未 pin 身份、撤销、重连负向路径 | ✅ | 纯协议/状态机 |
| Keystore | ✅（软件实现） | 硬件 TEE 不覆盖 |
| 前台 Service、ready/rebind/recreate/lock/stop | ✅ | 用 `--serial emulator-5554` |
| 多 Client 功能隔离 | ✅ | 两个 AVD 并发 |
| API 26 运行时 | ✅ → `EMULATOR` | 不能写成真机 PASS |
| release APK 安装性、R8、资源 | ✅ | — |
| 损伤矩阵（丢包/重排/抖动/拥塞） | ⚠️ 部分 | `-netdelay`/`-netspeed` 或 host `tc netem`；音频时序不真实 |
| 网络切换 | ⚠️ 部分 | 模拟器可切网络；MIUI EPERM/真实 handover 不可复现 |
| 60 分钟 soak / 泄漏趋势 | ⚠️ 部分 | 内存/会话趋势可看；音频链路不代表真机 |
| 端到端可听时延、underrun、音质 | ❌ | 必须真机 |
| 60 分钟双真机 | ❌ | `spec.md` 明写 real Clients |
| IPv6、真实动态 MTU | ❌ | 模拟器默认 NAT 基本只有 IPv4 |
| 性能、二进制体积 | ➖ | 用 arm64 产物测量，不需要跑模拟器 |

## 7. 网络

- 模拟器访问 host 用 `10.0.2.2`；Server 的 v2 UDP `65531` 默认应可达。
- 模拟器默认 NAT 只有 IPv4，IPv6 行为不可依赖；IPv6 归 host live-QUIC 或真机。
- 模拟器网络延迟/带宽可用 `-netdelay` / `-netspeed` 模拟，但只用于功能性损伤，不作为时延证据。

## 8. 已知限制

- 模拟器音频 HAL 与真机不同，`underruns`、AudioTrack 缓冲尺寸、端到端时延都不可采信。
- M4 末期的 `F32 write(byte[])` 恒返回 −3 这类设备栈问题，模拟器大概率复现不了。
- MIUI 的 `bindSocket` EPERM 是设备特有，模拟器不能验证该分支。
- 每张工单收尾仍建议保留一次真机关键路径抽检，把真机时间集中在里程碑末尾。

## 9. 2026-11-16 环境验证记录

| 项目 | 结果 |
|---|---|
| x86_64 原生库构建 | PASS — `x86_64-linux-android` |
| x86_64 release APK | PASS — 仅含 `lib/x86_64/libaudio_share_client_v2_native.so` |
| 默认 release APK | PASS — 仍仅含 `lib/arm64-v8a/...`，对外产物未变 |
| API 35 模拟器启动 | PASS — `emulator-5554`，SDK 35，ABI x86_64 |
| API 26 模拟器启动 | PASS — SDK 26，ABI x86_64 |
| APK 安装与 `primaryCpuAbi` | PASS — 两个 API 级别均为 `x86_64` |
| native `.so` 加载 | PASS — logcat `Load .../lib/x86_64/...so ... ok`，随后 `connecting 10.0.2.2:65531 ALPN audio-share/v2` |
| JVM 单元测试 | PASS — `testReleaseUnitTest` 48 tests / 0 failures（`-PnoNative=true`） |
| 前台媒体 Service | PASS — `isForeground=true`，`mediaPlayback` |
| 崩溃 | 无 |
| 真机 | **NOT RUN** — 本节全部只能记 `EMULATOR` |

> 验证时未启动 Server，因此只证明「加载/启动/重连策略」路径，不证明认证、媒体或播放成功。

### 端到端 READY / 生命周期（2026-11-16）

证据：[`evidence/emulator-e2e-2026-11-16/README.md`](evidence/emulator-e2e-2026-11-16/README.md)。

- `ready` / `rebind` / `stop`：PASS（含 Server 连接复用计数与 701 关闭）。
- `recreate`：首次失败（重建后 `AUTHENTICATING`），重试 PASS；日志显示启动期出现一次
  701 后立即重连，疑似重复 `ACTION_START` 竞态，待单独开票。
- `lock`：断言失败，因为测试只接受 `READY`，实际状态是 `PLAYING`；手动 `dumpsys` 证明
  锁屏期间与唤醒后前台 Service 和会话均存活。

**注意**：`lock` 与 `recreate` 用例写于 M3；`post-fix-device-acceptance.md` 引用的
`postfix-clean-{lock,recreate}.log` 也是 M3 日志。M4 媒体合入后这两条用例没有重跑，
`lock` 的 READY 断言在真机播放时同样可能失败。不能把这次结果简单归因于模拟器。

以上全部只能记 `EMULATOR`；真机 NOT RUN。

## 10. 维护规则

- 每张工单如使用模拟器，证据文件需写明设备型号位为 `emulator:<avd-name>` 与 API 级别，
  并显式标注「真机 NOT RUN」。
- 是否把 API 26 等 `WAIVED` 项改记为 `EMULATOR`，由维护者书面裁决后更新 `roadmap.md`。
