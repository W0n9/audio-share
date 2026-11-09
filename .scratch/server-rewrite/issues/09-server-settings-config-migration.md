# 09: Server settings + config migration

**What to build:** Server GUI 提供 Host、Port、Endpoint、Encoding 与 Reset；读取并迁移现有 `config.ini`；设置持久化并驱动 core。改设置后用 v0.3.4 Android Client 验证生效。

**Blocked by:** 08 (Tauri tray-first shell + Start/Stop)

**Status:** ready-for-agent

- [ ] Host、Port、Endpoint、Encoding 可编辑并持久化。
- [ ] 现有 `config.ini` 的键被读取并迁移，不丢设置。
- [ ] configVersion 不匹配时给出明确迁移结果，不静默丢弃用户配置。
- [ ] Reset 恢复默认并写回。
- [ ] 修改设置后启动 Server，v0.3.4 Android Client 按新设置连接。
- [ ] 配置写入原子化。
