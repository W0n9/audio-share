# 工单 11/12 最终集成审查

固定点：`83bfd55`。被审查集成 HEAD：`1ff5d50`。
Diff：`git diff 83bfd55...HEAD`。范围是本轮工单 11/12，不重新扩大到此前已延期的媒体、Keystore、pairing、GUI。

## Standards

没有文档标准硬违规。以下均为判断性 Fowler smell，不是 Spec 行为缺陷：

1. **possible Data Clumps**：`crates/v2-transport/src/driver.rs:76–86,113–125,242–261` 和
   `src/state_machine.rs:58–70`，配置/健康/活动标志/实例 ID/时间戳在 `CaptureSnapshot`、
   `SessionServer`、`ServerConfig` 里重复展开并选择来源。建议共享一个状态值，整体传递。
2. **possible Duplicated Code**：`android-client-v2/app/src/androidTest/java/dev/audioshare/v2/ControlSessionInstrumentation.java:54–57,77–80,134–137`，启动/唤醒/重建重复
   currentActivity 非空＋截止时间＋100 ms 等待。共享等待方法，各场景保留独立断言。
3. **possible Duplicated Code**：`android-client-v2/scripts/service-device-test.py:49–52,61–63`，
   正常唤醒/失败清理重复 wake＋dismiss keyguard。共享唤醒动作，严格检查与尽力清理分开。

标准来源：AGENTS.md、docs/agents 文档、GLOSSARY.md、ADR 0001–0004、Server rewrite README/harness。

## Spec

两项实质缺陷，均为静态路径确认；审查未做实际设备故障/调度注入：

1. **P2 — capture 故障后生产 Server 仍接受 Subscribe。** 工单 12 第 20 行要求
   “把真实 capture 状态/当前配置传入 SessionServer”“capture 不可用时保持 501 行为”；
   normative spec 第 321 行要求 unhealthy/absent 时无状态变化。
   `crates/as-cmd/src/main.rs:464–465` 永远克隆启动快照，healthy 一直 true；
   `crates/v2-transport/src/driver.rs:242–261` 又只在连接初始化读取一次。
   启动后 Endpoint 失效，ADR-0003 第 133–137 行规定现有进程仍运行；新会话/现有会话的
   首次 Subscribe 仍可能返回 OK＋旧配置并改变订阅标志，而非 501。
   **只要求真实拒绝订阅，不要求自动 capture 恢复，也不得改变 v1 既有行为。**
2. **P2 — ACTION_STOP 后旧 poller 可把关闭快照永久回写成 READY。** 工单 11 第 22 行
   要求 “显式 ACTION_STOP 触发 701 与 Service 清理”。
   `ClientSession.kt:156–158` 的 handle 检查与写入/发布，不与 stop 第 87–93 行原子化。
   可控交错：poller 读 READY 并通过检查后暂停；STOP 销毁 handle、发布 CLOSED并清理 session；
   poller 恢复后写入并发布 READY。`ControlSessionService.kt:28` 后续从 relay 返回 READY，
   重绑定也重放 READY；线程退出后没有纠正。需要生命周期/会话来源保护，而非再加一次裸检查。

## 修复要求

- 单个实现子代理在新隔离 worktree 统一修复五项；调用 tdd，至少两项缺陷有真正断言失败的
  RED，再 GREEN，不能只有 missing API 编译 RED 或靠重复运行普通真机碰运气。
- 已约定 seam：工单 11 的公开状态/关闭快照及 Service 注册/重绑定；工单 12 的真实
  control Subscribe/501 响应与生产 capture 注入。可用公开状态组件控制 native 更新交错，
  capture backend 外部边界控制故障；不得私有方法反射或在生产 UI 硬写 READY。
- 新旧会话边界也必须正确：旧会话的迟到事件不得覆盖关闭或之后的新会话；有效新会话仍能到 READY。
- capture 信息可新增必要的只读 Server Core 观察接口，但不新增媒体发送、恢复 supervisor，
  不改变 v1 framing/PCM/状态行为；配置 immutable 的现有边界仍按 ADR-0003。
- 保持 golden vectors 原字节、身份/test-support 安全边界、API 26 与 JNI 热路径要求。
- 回归：workspace、native 默认/test-support、Kotlin/JVM、默认 release；修复后再做设备验证。

## 修复前已验证的基线

工单 11 提交 `058a905`；工单 12 提交 `78c6981`，集成 tip `1ff5d50`。
463 workspace PASS/0 FAIL/4既有ignored；native 两配置各102 PASS；JVM31 PASS；默认 release PASS。
普通真机 READY/rebind/recreate/lock/stop701 PASS（不覆盖上述并发/故障），frozen v0.3.4 APK
SHA-256匹配、实际 AudioTrack 与 wire smoke通过，不宣称新音质/60分钟验收。

用户已授权停止无用的旧 Server PID 4107018，并明确不必恢复；65531 已释放。
父会话的临时测试 Server PID 115157 / port 65533 仍用于验证；实现子代理不得操作它或设备。
私钥只在临时目录；不复制或提交任何 identity 材料。

## 2026-11-13：五项统一修复

实现分支：`review/wire-v2-11-12-fixes`。独立工作树：
`/home/w0n9/workspaces/audio-share/.claude/worktrees/agent-ae8d4922334c6e2c4`。
首次检查该工作树无修改、HEAD 为 `342751f`，仅在自己的工作树重置到集成
`1ff5d50bd43e04f64ac675c658f37524a730dd0f` 后创建上述分支。未重新委派。
本文件是审查报告的副本；原报告、原始工单和其他工作树均未编辑。

### 修复内容

1. **实时 capture 门控**：Server Core 的 `CaptureObserver` 只读观察实际
   `JoinHandle::is_finished()` 和 immutable Capture Format；观察不启动新线程、
   不 join、不恢复 capture。`as_cmd::v2_capture_source` 保留配置模板，每次读取
   真实线程可用性；driver 在每个 control 请求前刷新整个 capture 值。
   故障后的新会话及已经 READY 的会话均以非致命、retryable 的 501 拒绝首次
   Subscribe；再次 Subscribe 仍为 501，RequestConfiguration 仍返回原配置。
   v2 stream 仍 inactive，不新增媒体发送或恢复 supervisor。
2. **STOP / 旧事件隔离**：生产 `ClientSession` 使用同一公开状态组件的
   `Source.poll` 读取真实 JNI 状态。事件来源校验、终态写入和回调发布处于同一
   monitor；native 读取不持有该锁。关闭或更换来源后，迟到 READY 被丢弃。
   ClientSession 的 start/create/reconnect/NetworkCallback 同时带有独立生命周期
   token 与 native 来源，不凭可复用的数值 handle 接受旧任务。
   Service 也为每次 ClientSession 所有权创建来源，STOP/onDestroy 先关闭它；
   snapshot 和 Binder 重绑定都读取该 relay。native CLOSED 终止 native 来源，
   但 Service 所有者直到 STOP 才终止，故合法 native 重连仍可进入 READY。
3. **共享 capture 值**：`CaptureSnapshot` 作为 `SessionServer.capture` 和
   `ServerConfig.capture` 的共同值，选择、传递及更新整组数据，不再逐字段
   map/source 选择。保留 `driver::CaptureSnapshot` 导出路径、原字段和 `absent()`，
   以及 SessionServer 的构造/服务接口；两个配置类型的旧扁平字段改为嵌套
   `capture`，仓库中的构造点已适配（Rust 源码层兼容性变化，不是 wire/FFI 变化）。
4. **Java Activity 等待**：三处共用 `awaitResumedActivity`，保留 10 s / 10 s / 5 s
   截止时间、100 ms 轮询、各自错误信息及重建实例/真实 READY 的独立断言。
5. **Python 唤醒动作**：共用 `wake_screen(strict=...)`。正常路径严格检查返回值；
   失败清理分别尽力 wake / dismiss，不因清理失败或超时掩盖原故障。
   host-only CLI 检查使用仅含 fake adb 的 PATH，验证正常 exit 0 和故障 exit 1
   后两个清理动作；没有调用真实 adb 或任何设备。

Android/JNI/Binder 既有公开签名与 API 26 baseline 不变。`android-app`、protos、
v1 fixtures 和 golden vector 字节均没有修改；没有复制 identity 或私有 fixture scalar。

### 真正断言 RED → GREEN

以下命令均在本工作树执行；统一路径变量：

```sh
WT=/home/w0n9/workspaces/audio-share/.claude/worktrees/agent-ae8d4922334c6e2c4
```

**driver 实时状态**：真实 QUIC 完成认证/协商并 READY 后，控制外部 capture 来源
变为 unhealthy；首次与再次 Subscribe 均应为 501，而非 OK/402。

```sh
cargo test --manifest-path "$WT/Cargo.toml" -p v2-transport --features test-support \
  --test session_driver ready_capture_failure_refuses_first_subscribe_without_subscribing \
  -- --exact --nocapture
```

RED exit **101**：`assertion left == right`，实际 `SubscribeResponse(status.code=0)`，
错误代码 `None`，预期 `Some(501)`。见 [red-live-capture.log](red-live-capture.log)；
同一测试 GREEN 见 [green-live-capture.log](green-live-capture.log)。

**生产 capture 注入**：通过公开 `CaptureBackend` / `CaptureSession::from_thread`
让实际 backend 线程结束，等只读 observer 确认其已结束；使用 main.rs 同一生产
注入函数，不以测试闭包伪造 healthy=false。用两套新生成的内存 Client key
分别验证已 READY 和故障后才连接的会话，并断言不改变订阅状态及 immutable 配置。

```sh
cargo test --manifest-path "$WT/Cargo.toml" -p as-cmd --test v2_server \
  production_capture_thread_exit_refuses_new_and_ready_subscriptions -- --exact --nocapture
```

RED exit **101**：线程已结束但生产模板仍返回 `SubscribeResponse(OK)`，
`None != Some(501)`。见 [red-production-capture.log](red-production-capture.log)；
同一测试 GREEN 见 [green-production-capture.log](green-production-capture.log)。

**STOP 调度**：先把原 split-check/write 行为抽入生产也使用的公开状态组件，
提供可控 native read（不是 missing API 编译红）。读任务通过来源检查后暂停，
STOP 发布 CLOSED，恢复读任务使其返回预定 READY。修复前断言永久回写 READY；
修复后输入在提交/发布前通过同一锁检查，关闭和重绑定保持 CLOSED。

```sh
ANDROID_HOME="$HOME/Android/Sdk" "$WT/android-client-v2/gradlew" \
  --project-dir "$WT/android-client-v2" testReleaseUnitTest -PnoNative=true \
  --tests dev.audioshare.v2.SessionStateRelayTest.stop_during_native_update_retains_closed_snapshot_on_rebind
```

RED exit **1**：`org.junit.ComparisonFailure: late native READY must not revive a stopped
session expected:<[CLOSED]> but was:<[READY]>`。见 [red-stop-state.log](red-stop-state.log)
和保留下来的 [red-stop-state.xml](red-stop-state.xml)；GREEN 见
[green-stop-state.log](green-stop-state.log) 与 [green-session-sources.log](green-session-sources.log)。

另一个纵向用例 `service_same_owner_reconnect_replays_the_new_native_ready` 捕获了首次
修复中过早终止 Service 所有者的问题：实际 CLOSED、预期 READY，见
[red-reconnect-owner.xml](red-reconnect-owner.xml)；区分 native / Service 所有权终态后 GREEN。
保留的用例还覆盖：暂停的旧读不能覆盖之后的 AUTHENTICATING、新 native READY 可见、
STOP 与已接受回调的顺序、旧 Service 来源不能覆盖新来源、同步重绑定重放。
未使用生产 UI 硬写 READY、私有方法测试或反射。

### 全量 GREEN 与边界

汇总：[verification-summary.json](verification-summary.json)。完整命令未过滤 workspace 用例：

```sh
cargo test --manifest-path "$WT/Cargo.toml" --workspace
cargo test --manifest-path "$WT/android-client-v2/native/Cargo.toml"
cargo test --manifest-path "$WT/android-client-v2/native/Cargo.toml" --features test-support
ANDROID_HOME="$HOME/Android/Sdk" "$WT/android-client-v2/gradlew" \
  --project-dir "$WT/android-client-v2" testReleaseUnitTest -PnoNative=true
ANDROID_HOME="$HOME/Android/Sdk" "$WT/android-client-v2/gradlew" \
  --project-dir "$WT/android-client-v2" compileReleaseAndroidTestJavaWithJavac -PnoNative=true
ANDROID_HOME="$HOME/Android/Sdk" "$WT/android-client-v2/gradlew" \
  --project-dir "$WT/android-client-v2" assembleRelease
python3 -I "$WT/.scratch/wire-protocol-v2/evidence/check-wake-cli.py" "$WT"
```

| 回归 | 结果 | 证据 |
|---|---|---|
| workspace（含 v1 replay、v2 golden replay） | 465 PASS / 0 FAIL / 4 既有 ignored | `green-workspace.log` |
| native 默认 | 102 PASS / 0 FAIL / 0 ignored | `green-native-default.log` |
| native test-support | 102 PASS / 0 FAIL / 0 ignored | `green-native-test-support.log` |
| Kotlin/JVM | 36 PASS / 0 FAIL / 0 skipped | `green-jvm.log` |
| 修改后的 Android instrumentation Java 编译 | PASS | `green-instrumentation-compile.log` |
| 默认 assembleRelease（实际 NDK/native、R8） | PASS | `green-default-release-initial.log`（首次完整 NDK 编译）、`green-default-release.log`（最终重跑） |
| Python CLI 正常/失败唤醒，仅 host stub | PASS | `green-harness-wake.log` |

`noNative` 仅用于 JVM 和设备工装的 host 编译；默认 release 没有 `noNative`、
`testSupport` 或 `deviceTests` 参数。实际 arm64 release native fingerprint 的 features 为
`[]`（不编入 fixture 身份），见 [release-native-features.json](release-native-features.json)。
完成全量测试后，保留并来源保护原有诊断日志，
再次完整运行 JVM 36 项和默认 release，仍 PASS。Rust 仅格式化改动的 12 个文件，
`git diff --check` 通过；没有 namespace/root/sudo 绕过宿主限制。

### 父会话复验指针 / 未跑项

真机、父会话 Server PID 115157 / 65533 均未操作。真机、Windows、音质/60 分钟
验收本轮 NOT RUN，按分工由父会话合入后处理；不把 host stub 或 Java 编译称作设备 PASS。

- 设备构建/运行命令见 `android-client-v2/README.md` 的“前台 Service 设备回归”，
  以及 `android-client-v2/scripts/service-device-test.py`；其公开 CLI 和用例名不变。
- 父会话重建显式 `-PtestSupport=true -PdeviceTests=true` 的测试 APK，使用自己的
  public config / Server log，复验 `rebind`、`recreate`、`lock`、`stop` 与 Server 701。
  本工作树默认 release APK 不含 fixture 身份，不能假称已用它完成认证真机复验。
- 原有真实 RED/GREEN 和 v0.3.4 smoke 证据仍在
  `.scratch/wire-protocol-v2/evidence/ticket-11-resume/README.md`，没有重跑或篡改。
