# 05: 恢复 Host 的本机网络地址枚举

**What to build:** Network configuration 的 Host 字段恢复 v0.3.4 MFC GUI 的可编辑地址下拉:枚举本机非 loopback IPv4 接口地址供选择,同时保留手动输入。

**Blocked by:** 无

**Status:** resolved

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
- 2026-11-11: implemented on `ticket/gui-mod-05-host-addresses` (commit
  `bb6e24b`), merged fast-forward into `integration/server-gui-modernization`.
  Core adds `local_addresses()` (deduplicated, v0.3.4 default first) and
  routes `default_host()` through it; the Tauri `Bootstrap` carries
  `hostAddresses`; the Host input uses a `datalist` while keeping free text.
  Checks: `cargo test -p audio-share-core` 35 passed (3 new),
  `cargo test -p audio-share-server-gui` 93 passed, `npm run check` 0 errors /
  0 warnings, `npm run build` 139 modules. Verified on the Windows 11 desktop
  with a debug build: the Host field shows the datalist arrow and local
  address suggestions
  (`docs/server-rewrite/img/gui-mod-05-host-addresses-zh.png`). The CI
  artifact was re-cut in run 42988001959 at `6dd11de`
  (`sha256 722aa366…f06f3`, 12,031,488 bytes; the CI log shows the same
  `index-DKDcBJa4.js` frontend asset as the local build). Release-artifact
  behavior verified from the background via the WebView2 CDP endpoint
  (`WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port`): `#host`
  has `list="host-addresses"` and the datalist contains `192.168.137.1`,
  `100.96.123.50`, and `10.8.144.155`; the window stayed
  minimized/background, and the debug port was closed with the process.

