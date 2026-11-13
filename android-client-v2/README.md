# android-client-v2

Android client for wire protocol v2. The native crate under `native/` owns the
QUIC transport, auth handshake, control state machine and reconnect glue; the
Kotlin app owns the UI and the reconnect policy.

## Build

```sh
# Default release build: no fixture test identity is compiled in.
ANDROID_HOME=$HOME/Android/Sdk ./gradlew assembleRelease

# E2E / device-test build: adds the TEST-ONLY fixture client identity key.
ANDROID_HOME=$HOME/Android/Sdk ./gradlew assembleRelease -PtestSupport=true
```

`-PtestSupport=true` enables the native crate's `test-support` feature, which
provides the fixture client identity/signing path used by the end-to-end
harness. It is TEST-ONLY. Never ship that configuration: default release
builds omit the feature, and the production identity comes from v2 pairing
(phase 6).

## Native tests

The native crate is its own Cargo workspace:

```sh
cd native
cargo test                  # production configuration
cargo test --features test-support
```

The Android NDK toolchain is required only for the Gradle release build; the
host tests above need no device.

## 前台 Service 设备回归

工单 11 的设备工装直接观察真实 Activity 与公开的 `stateSnapshot()` / 本地
Binder 监听接口，不模拟 native READY。测试 APK 只使用 Android 平台
Instrumentation，不增加生产依赖，也不要求提高 API 26 baseline。

```sh
ANDROID_HOME=$HOME/Android/Sdk ./gradlew \
  assembleRelease assembleReleaseAndroidTest \
  -PtestSupport=true -PdeviceTests=true
adb -s "$SERIAL" install -r app/build/outputs/apk/release/app-release.apk
adb -s "$SERIAL" install -r \
  app/build/outputs/apk/androidTest/release/app-release-androidTest.apk

python3 -I scripts/service-device-test.py --serial "$SERIAL" \
  --config "$PUBLIC_CONFIG" --server-log "$SERVER_LOG" \
  --test rebind --output "$RESULT_FILE"
```

`PUBLIC_CONFIG` 是 JSON，包含本次隔离 Server 的 `host`、整数 `port`、
`pinnedSpki`（SPKI SHA-256 hex）和 `serverKey`（完整 Ed25519 公钥 hex）。
不要复制或提交 Server 私钥。Server 必须在隔离测试配置里授权 checked-in
fixture 的 P-256 **公钥**；测试 APK 的 fixture 身份仍仅在显式
`-PtestSupport=true` 时编译。

测试用例为 `ready`、`rebind`、`recreate`、`lock`、`stop`。`rebind` 先让真实
会话到达 READY，再断言注册新监听者时**同步**收到 READY；等待下一次轮询
不算通过。`recreate` 和 `lock` 还要求 `--server-log`，对比 READY 前后的
连接计数，避免通过重连掩盖生命周期缺陷。`stop` 通过公开快照和 Android
实际 foreground 标志检查清理；Server 的 701 关闭码另见设备证据。

MIUI 可能要求用户确认测试 APK 的 USB 安装，并禁止后台自行弹出 Activity。
工装通过 `adb am start` 显式打开界面，不修改这些权限、不使用 root 绕过。
`lock` 会实际关闭屏幕并唤醒，失败时也尝试恢复屏幕。

`-PdeviceTests=true` 仅保留独立测试 APK 消费的少量 Binder API；仍启用 R8。
默认 `assembleRelease` 既不使用这些 keep rules，也不包含 fixture 私钥。
不要发布上述测试构建。2026-11-13 的 RED/GREEN、锁屏及 v0.3.4 smoke 证据见
[工单 11 证据](../.scratch/wire-protocol-v2/evidence/ticket-11-resume/README.md)。
