# 05: 恢复 Host 的本机网络地址枚举

**What to build:** Network configuration 的 Host 字段恢复 v0.3.4 MFC GUI 的可编辑地址下拉:枚举本机非 loopback IPv4 接口地址供选择,同时保留手动输入。

**Blocked by:** 无

**Status:** ready-for-agent

- [ ] 后端暴露本机 IPv4 地址列表:过滤 loopback、去重、默认地址在前,且不改变 `default_host()` 的结果。
- [ ] Bootstrap 将地址列表传给前端(`hostAddresses`),不新增轮询或网络监听。
- [ ] Host 字段使用可编辑下拉建议(datalist),候选仅来自后端;手动输入仍可保存。
- [ ] Rust 单测覆盖去重、默认优先与空列表;`cargo test`、`npm run check`、`npm run build` 通过。
- [ ] 在 Windows 真实桌面复验下拉建议,并更新验收文档。
- [ ] Wire Protocol、Server Core 的采集与绑定行为不变。

## Comments

- 2026-11-11: reported during Windows acceptance. v0.3.4's MFC field was an
  editable `CBS_DROPDOWN` combo (`CServerTabPanel.cpp:83-92`) populated from
  `network_manager::get_address_list()` (`GetAdaptersAddresses`, skipping
  loopback and non-Up interfaces); the modernized Tauri field regressed to a
  plain `<input type="text">`. The Rust core already enumerates interfaces for
  `default_host()` via `if_addrs`, so the fix can reuse that path.
