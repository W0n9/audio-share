# 工单 11/12 最终合入后验收 — 2026-11-13

核心修复：`058a905`、`78c6981`、`df27ac3`；集成代码 tip：`424fd30`。
分支：`phase-2/wire-protocol-v2`。本记录覆盖最终合入后的父会话复验，不替换历史 RED/失败证据。

## 两轴 review 闭环

- Standards：0 文档硬违规；3 项判断性维护问题全部处理（共享 capture 状态、统一 Activity 等待、统一屏幕恢复）。
- Spec：2 项缺陷都有真正断言 RED→GREEN：实际 capture 线程结束后的 Subscribe 501、旧 native READY 在 STOP 后不得复活快照。
- 代码/断言证据：[最终 review 与修复报告](final-review-1112.md)。
- 不新增 capture 自动恢复、v2 媒体发送/播放、生产 pairing/Keystore 或 GUI；v1 字节和所有 golden vector 不变。

## 最终自动化结果

在父会话的实际集成树重新运行：

| 命令 | 结果 |
|---|---|
| `cargo test --workspace` | 465 PASS / 0 FAIL / 4 既有 ignored |
| native 默认配置 | 102 PASS / 0 FAIL |
| native `--features test-support` | 102 PASS / 0 FAIL |
| `gradlew testReleaseUnitTest -PnoNative=true` | 36 PASS / 0 FAIL |
| 默认 `gradlew assembleRelease` | PASS，无 test-support/deviceTests 参数 |
| 公开 CLI 的 host-only 屏幕准备/严格恢复/失败清理 | PASS；新增初始屏幕准备断言曾 RED |

4 个既有 ignored：3 项依赖真实 headless PipeWire/wireplumber，1 项访问真实 GitHub release API；没有人为过滤可运行用例。

## 真机复验

Xiaomi M2007J22C / Android 12 / API31 / arm64，测试 APK 显式使用
`-PtestSupport=true -PdeviceTests=true`；仍启用 R8。

| 实际场景 | 结果 | 证据 |
|---|---|---|
| 真实认证/协商 READY + 公开快照/同步重绑定 | PASS | `ticket-11-resume/postfix-clean-rebind.log`，连接9→9 |
| Activity 重建后仍为 READY，不重连 | PASS | `ticket-11-resume/postfix-clean-recreate.log`，11→11 |
| 真正锁屏/唤醒 + foreground 与 READY 保留 | PASS | `ticket-11-resume/postfix-clean-lock.log`，13→13 |
| ACTION_STOP → CLOSED / foreground 清理 | PASS | `ticket-11-resume/postfix-clean-stop.log`，14→14 |
| Server 收到正常关闭码 701 | PASS | `ticket-11-resume/postfix-server-701.log` |

首次最终复验曾停在 IDLE，保留于 `postfix-rebind.log`。定向诊断确认 native 和
Service 均 READY、接受状态发布；手机在长期等待期间自动休眠，Activity 已解绑。
实际 window policy 为 `INTERACTIVE_STATE_SLEEP` / `mIsScreenOn=false`。普通唤醒及
无密码 keyguard dismiss 后，同一 APK 转绿。随后工装在每次显式启动 Activity 前
准备屏幕，host-only CLI 断言 RED→GREEN，且移除所有 `[DEBUG-ticket11-final]` 临时日志，
用无日志 minified APK 再完成上述四项 PASS。没有绕过密码、root 或安装权限。

## frozen v0.3.4 APK

原有发布 APK 没有重建或更新；版本 0.3.4 / versionCode3004，SHA-256：
`77cc93d32a3e6b6117b9c202ff5be69082ea5711087c23e6877cef5ac4a471b4`。
最终 Server Core 上再做实际 smoke：界面“已开始”、AudioTrack 活动，实际 F32/stereo/48000；
wire probe `--tone-hz 0` **PASS**，3942 datagrams、PCM alignment/MTU正确、2 heartbeats、0 misses。
见 `ticket-11-resume/postfix-v034-wire-probe.json`、`postfix-v034-audiotrack.txt`、截图。
Synthetic backend 产生 sawtooth，不把该检查称为 440 Hz sine 或听觉质量通过。

旧 APK 的地址只在点击播放时持久化；早先仅恢复文本不足以恢复最后连接值。
用户明确授权保存原地址后立即停止。已执行并冷启动核验：持久化为原值
`10.8.144.155:65530`，播放已停止。用户还明确授权切回集成 checkout 并清理本轮三个
已合入 worktree；原有 prototype/diagnostic worktree 与原始未跟踪文件保留。

## 未运行/不声称

未重新做 Windows、API26 真机、长时间音质/60分钟矩阵；这些不是本轮授权的新增验收。
不将 Java 编译、host stub 或本次 v1 smoke 冒充上述验收。
临时 Server 与测试 identity 在收尾清理；用户指定的旧 PID4107018 不恢复。
