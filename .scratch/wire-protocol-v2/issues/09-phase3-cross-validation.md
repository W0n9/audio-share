# 09: Phase-3 cross-validation and acceptance — Rust ↔ Android authenticated control channel

**What to build:** 第 3 阶段的交叉验收：真机 Android Client ↔ 生产 Rust Server 完成「连接 → 认证 → 协商 → READY」的完整控制通道，正/负/限额/撤销全部有设备证据；v1 回归确认。交付 phase-3 报告。

**Blocked by:** 06, 07, 08

**Status:** ready-for-agent

**Type:** task

**Environment note:** 设备与通路已验证可用（见 memory `prototype-device-validation-setup.md` 与 `prototype/wire-protocol-v2-transport/evidence/device-validation.log`）：MIUI 真机 adb over tailscale、USB 安装需用户侧开关、uiautomator 不可用（用 screencap + input）、`--ez` 传布尔 extra。

## Requirements

- E2E 正向：真机连接生产 Server（65531）→ AuthInit/Challenge/Proof/Result → Negotiate/NegotiateResponse → READY 稳态 → 用户主动停止（Close 正常码）；全程 Server/Client 日志 + 状态截图入 `evidence/`（沿用带命令头的记录惯例）。
- 负向（每项都有捕获结果 + 日志引用）：
  - 错 SPKI pin（握手层拒绝，无应用数据）；
  - 错签名 auth_context / 错公钥身份（202 AUTHORIZATION_UNKNOWN）；
  - 撤销授权：连接中撤销 → 活跃会话 ≤5 s 终止 + 新连接 204；
  - 状态违规：READY 前杂散帧 → 可判定 401/402（按 §5.2）；
  - malformed：坏长度/未知 oneof case → 401 + Close；
  - 限流：连续失败触发 TM-5 上限的可判定行为。
- 空闲与 keep-alive：READY 稳态静置 > 60 s（D3：30 s timeout + 10 s keep-alive）连接不假死；锁屏/亮屏（前台 Service）会话存活；飞行模式/断网恢复 → 有界退避重连成功。
- 资源基线：设备端 CPU/RSS + Server 端会话计数（对照 prototype 的桌面基线）。
- **v1 回归（红线）**：`protocol-harness replay` 全绿；v1 集成测试全绿；v0.3.4 兼容线（`android-app/`）构建不受影响；服务器同时跑 v1+v2 互不干扰（v1 客户端 fixture 连 v1 端口、v2 客户端连 v2 端口并发）。
- Rust↔Android「对拍 fixtures 而非互为编码器」的证据在两侧各留一份（复用 06/08 的回放工装在真实构建上重跑）。
- 报告：`.scratch/wire-protocol-v2/phase-3-report.md` —— 逐项 PASS/FAIL/NOT RUN、版本表、证据链接、遗留事项；报告不宣称媒体/统计能力（那是第 4 步）。

## Definition of done

- 上述每项有 PASS/FAIL/NOT RUN + 证据；无 FAIL（有 FAIL 按裁决修或升格工单）。
- phase-3 报告落档；本 ticket resolved 即第 3 阶段（Tracer-Bullet 第 3 步）完成，第 4 步（单 Client PCM 媒体 + 统计 + 显式关闭）解锁。
