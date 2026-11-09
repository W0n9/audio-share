# 14: Linux packaging

**What to build:** deb 与 AppImage；在 Ubuntu 24.04+ 安装与运行；明确 WebKitGTK 4.1 与 PipeWire 依赖；产物名可预测并写入发布说明。

**Blocked by:** 12 (Linux equivalents)

**Status:** ready-for-agent

- [ ] deb 可安装、卸载并启动。
- [ ] AppImage 可下载、`chmod +x` 后运行。
- [ ] 依赖缺失时给出可读错误或文档说明。
- [ ] deb 与 AppImage 产物名可预测，并写入发布说明。
- [ ] 在干净的 Ubuntu 24.04 环境验证依赖。
- [ ] 托盘与 Start/Stop 在打包版本中可用。
