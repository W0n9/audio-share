# M5-06: Impairment conformance matrix and parser fuzz

**What to build:** 按 spec §11.2/§11.3 建立确定性损伤注入矩阵与 parser fuzz，逐行产出
独立证据。这是 M5 DoD 的验收主体。

**Blocked by:** M5-03, M5-04

**Status:** ready-for-human

**Type:** task

## Requirements

- [ ] §11.2 矩阵逐行独立证据，每行 `PASS`/`FAIL`/`NOT RUN`：
      - [ ] 2% loss
      - [ ] 25% reorder
      - [ ] 50% reorder
      - [ ] 5% duplicate
      - [ ] 10 ms ± 5 ms jitter
      - [ ] 300 kbit / queue50 congestion
      - [ ] MTU 1280
      - [ ] ±100 ppm drift
      - [ ] 25 s idle
      - [ ] connection-kill reconnect
- [ ] **network loss 与 `late_dropped` 分开计数**，不得混为一个指标。
- [ ] parser fuzz 工装：畸形包、未知字段、状态违规、截断；不得 panic，按 §6 registry
      返回确定性错误。
- [ ] 损伤注入必须是确定性的（可复现 seed），不能靠真实网络碰运气。
- [ ] 负向/未知字段/状态违规用例覆盖。
- [ ] 结果写进 `.scratch/wire-protocol-v2/evidence/m5-matrix/`，不入库。

## Notes

- 规范：spec §11.2（impairment）、§11.3（fuzz）。
- 不得把"网络到达指标"当成"可听延迟指标"（spec §8 明确二者不同）。

## Answer

**实现（`integration/m5`；提交 `d8f5e0e`，review 修复 `f799599`；仅 host 侧）**

- 新增四个测试文件（共约 2.3k 行，仅测试，无生产代码/proto/默认值改动）：
  - `android-client-v2/native/tests/impairment_matrix.rs`、`parser_fuzz.rs`
  - `crates/v2-transport/tests/impairment_matrix_server.rs`、`parser_fuzz_server.rs`
- 确定性注入：内联 splitmix64 seeded PRNG，seed 打印且同 seed 可复现；不使用真实网络碰运气，不 sleep（用注入时钟）。
- **network loss 与 `late_dropped` 分开断言**，并有专门测试证明二者独立变化；不以「包到达/ACK」当播放证据。
- parser fuzz：控制面 schema 解码 + 24 字节媒体头；共 **70,000** seeded 用例（server 40,000 / client 30,000），无 panic、无静默忽略，错误按 §6 registry 确定性分类（401/402/101-102/203 等）。
- review 修复：行 6 的 300 kbit/s 预算原无 carry-over（实际未发送），已改为跨 tick 结转并断言 ≥95% 实际发送、队列有界、满速恢复。

**§11.2 矩阵结果（逐行独立）— host 模拟 PASS / 真实网络或设备 NOT RUN**
| 行 | 状态 | 层面 |
|---|---|---|
| 1 loss 2% media/concealment/accounting | PASS | host app layer |
| 1 有损路径 control clean | NOT RUN | 真实网络 |
| 2/3 reorder 25%/50% | PASS | host app layer |
| 3b late_dropped 与 loss 分离 | PASS | host app layer |
| 4 duplicate 5%（应用 sequence dedup） | PASS | host app layer |
| 4 QUIC packet-number dedup | NOT RUN | 真实 QUIC |
| 5 jitter 10±5 ms | PASS | host app layer |
| 6 300 kbit/s / queue 50 | PASS | 进程内 queue/pacer + 注入时钟 |
| 6 真实 300 kbit/s 路径 | NOT RUN | 真实网络 |
| 7 MTU 1280 chunker + client parser | PASS | 进程内 + host |
| 7 DPLPMTUD re-probe | NOT RUN | 真实 QUIC |
| 8 ±100 ppm drift | PASS | 纯 estimator/controller + 模拟缓冲 |
| 8 真实设备时钟 | NOT RUN | 真实设备 |
| 9 idle 配置 + 601 事件 | PASS | 纯状态机 + 常量 |
| 9 真实 25 s 存活 | NOT RUN | 真实网络 |
| 10 connection-kill teardown | PASS | 纯状态机 |
| 10 backoff reconnect | NOT RUN | Kotlin-only 策略，不在允许测试路径 |

**验证（host，PASS）**
- `cargo test -p v2-transport`：**289 passed, 0 failed**。
- `cargo test -p audio-share-core`：**118 passed, 0 failed, 1 ignored**。
- native 默认与 `--features test-support`：各 **227 passed, 0 failed**。
- 证据：`evidence/m5-matrix/`（`matrix.md`、`fuzz.md`、各 `.log`，未跟踪）。

**NOT RUN（不写成 PASS）**
- 上表所有「真实网络 / 真实 QUIC / 真实设备 / Kotlin backoff」行：NOT RUN，留给人工真机矩阵（M5-07）。

## Comments

### 2026-11-20 Windows 真机验收

本轮 Windows 环境无 `tc netem`，按决定真实网络损伤行全部 NOT RUN；host 矩阵与 fuzz 结果不变。
