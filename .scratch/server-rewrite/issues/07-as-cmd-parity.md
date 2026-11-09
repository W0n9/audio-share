# 07: `as-cmd` parity

**What to build:** `as-cmd` 保留全部现有 flags 与语义：`-b/--bind`、`-l/--list-endpoint`、`--list-encoding`、`-e/--endpoint`、`--encoding`、`--channels`、`--sample-rate`、`-V/--verbose`、`-v/--version`、`-h/--help`；默认地址选择、`server.log` 与退出码保持可判断。Windows 与 Linux 都要列出真实 Endpoint。

**Blocked by:** 05 (Windows real capture path), 06 (Linux real capture path)

**Status:** ready-for-agent

- [ ] 所有 flags 可用；帮助文本列出示例与默认值。
- [ ] `-l` 在 Windows 与 Linux 列出真实 Endpoint 并标记默认。
- [ ] `--list-encoding` 列出 5 种 encoding。
- [ ] 默认 bind 地址选择与旧版一致。
- [ ] `server.log` 继续写入；错误信息可读；退出码可判断。
- [ ] 进程级断言覆盖 help、version、list 与 exit code。
