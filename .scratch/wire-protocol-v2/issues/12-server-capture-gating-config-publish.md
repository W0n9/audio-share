# 12: Server Subscribe capture gating and configuration publication

**What to build:** 修复 2026-11-13 `/code-review` Spec 轴在集成分支发现的 Server 控制平面缺口，使生产 Server 对 `Subscribe` / `RequestConfiguration` 的行为符合 `docs/wire-protocol-v2/spec.md` §5.2；保持 Tracer-Bullet 第 3 步边界，不实现媒体发送、抖动/漂移、生产 pairing。

**Blocked by:** none

**Status:** resolved

**Type:** task

## Findings (from review, verified against HEAD `83bfd55`)

1. `Subscribe` 只检查 `capture_healthy`（`crates/v2-transport/src/state_machine.rs` `handle_subscribe`），不处理 capture **absent**。规范 §5.2：「`Subscribe` | capture unhealthy or absent | Error(501, retryable=true)」。`SessionServer::new`（`driver.rs`）默认 `capture_active:false, capture_healthy:true, current_configuration:None`，`as-cmd`（`main.rs` v2 listener）从不改写，所以生产 Server 对 Subscribe 回无配置的 `SubscribeResponse(OK)`。
2. 生产 Server 从不发布配置：`RequestConfiguration` 返回空 `ConfigurationPublished`，Client 无法满足「订阅前至少收到一次 `ConfigurationPublished`」。
3. `StreamStarted` 使用占位值 `stream_instance_id: 1, initial_timestamp: 0`，规范要求「active instance, current generation, next-frame timestamp」。

## Requirements

- capture 视为 absent（无 `current_configuration` 或 capture 未注入）时 `Subscribe` → `Error(501, retryable=true)`，无状态变化；补 §5.2 行级测试（先红后绿）。
- 让 `as-cmd` 的 v2 listener 把真实 capture 状态/当前配置传入 `SessionServer`（只读状态注入点，不引入媒体发送）；capture 不可用时保持 501 行为。发布的 `ConfigurationPublished` 字段对齐 spec 与 golden vectors。
- `StreamStarted` 的 instance / timestamp 来自 stream 状态而非常量；若本阶段无活动 stream 则保持不可达并以测试说明。
- 不得改变 golden vectors 字节；既有 `crates/v2-transport` 与 `crates/as-cmd` 测试、v1 回归保持通过。
- 不在范围：Keystore、GUI 生命周期、v1 标注/禁用开关、`::` 绑定 host 行为、`media.rs` 精简、`send_queue` 恢复（review 其余项另行裁决）。

## Answer

2026-11-13 — 要求已实现并验证，Status → `resolved`。

- `78c6981`：capture absent/unhealthy 的 Subscribe 返回非致命、retryable的501且不改变订阅状态；as-cmd 发布真实协商格式；StreamStarted从当前capture/stream值读取instance/timestamp，不再写死占位常量。
- `df27ac3`：补 final review 指出的启动快照过时缺陷。`CaptureObserver`只读实际backend线程与immutable格式，不join或自动恢复；生产 `as_cmd::v2_capture_source`每次读取真实可用性，driver在每个control请求前刷新共享CaptureSnapshot。
- 真实backend线程结束的生产注入用例及live-QUIC READY用例均有断言 RED→GREEN；故障后新会话和READY会话的首次/再次Subscribe都为501，而不是OK/402，仍能请求原immutable配置。
- CaptureSnapshot/SessionServer/ServerConfig 的状态组统一传递，保留已有CaptureSnapshot导出路径；Rust内部配置字段改为嵌套capture，调用者已适配，wire与FFI未变。
- 集成代码 `424fd30` 及后续验收提交；最终 workspace465 PASS/4既有ignored，golden/v1 replay、native两配置各102、JVM36、默认release与frozen v034 smoke通过。冻结的protos/fixtures/android-app未改变。
- 本阶段无活动v2媒体流，生产StreamStarted保持不可达；不新增发送/播放、自动恢复、Keystore/pairing或GUI。

证据：[最终审查修复与生产故障RED/GREEN](../evidence/final-review-1112.md)、[最终合入后验收](../evidence/post-fix-device-acceptance.md)。

## Comments

### 2026-11-13 — 两轴最终审查闭环

本轮Standards的3项判断性重复状态/等待/屏幕恢复问题，以及Spec的capture过时/STOP迟到状态两项缺陷，由同一个最终实现子代理修复。完整回归释放65531后通过；用户要求不恢复旧Server进程。工单 scope 外的事项保留原裁决，不称为已实现。
