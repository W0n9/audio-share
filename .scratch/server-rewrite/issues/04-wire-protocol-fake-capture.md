# 04: Wire Protocol slice with fake capture + `as-cmd -b`

**What to build:** 新 Server 用 synthetic capture source 完成 Wire Protocol 全链路：TCP `GET_FORMAT`、`START_PLAY`、3 秒 heartbeat、5 秒超时、UDP id 绑定、裸 PCM 发送、MTU 1492 分片与 `block_align` 对齐；`as-cmd -b` 能启动和停止。harness 独立验证，不依赖真实音频设备。

**Blocked by:** 01 (New Rust workspace + Wire Protocol harness + v0.3.4 golden fixtures)

**Status:** ready-for-agent

- [ ] `GET_FORMAT` 返回与 v0.3.4 兼容的 `AudioFormat`。
- [ ] `START_PLAY` 返回 id；客户端发送 id 后收到 UDP PCM。
- [ ] heartbeat 每 3 秒发送；5 秒无活动关闭会话。
- [ ] MTU 分片与 `block_align` 对齐符合 v0.3.4；多字节整数显式 little-endian。
- [ ] `as-cmd -b host:port` 可启动和停止，harness 全链路通过。
- [ ] 无真实音频设备时测试仍可运行。
