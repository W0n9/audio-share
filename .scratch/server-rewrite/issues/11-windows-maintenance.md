# 11: Windows maintenance: Firewall Repair + Sound Panel

**What to build:** Windows 上提供提权修复 TCP/UDP 入站规则与打开系统声音面板；失败时给出可读错误。

**Blocked by:** 10 (Lifecycle + i18n + About + update check)

**Status:** resolved

> Windows 11 runtime executed on 2026-11-10: UAC-approved firewall repair,
> UAC-cancel error, and Sound Panel all pass. The real v0.3.4 Android Client
> connection remains part of tickets 15/16; a local client over the WLAN IP
> connected through the repaired rules.

- [x] Firewall Repair 提权后重建入站规则，客户端可连接。Windows 11：TCP/UDP 入站规则（Domain+Private、Allow、程序限本 exe）经 `netsh ... verbose` 核对；本机客户端走 `10.8.144.155:65530` 收到 120 个对齐 datagram。真实 Android Client 见 ticket 15/16。
- [x] Sound Panel 打开系统声音设置。Windows 11：`control.exe /name Microsoft.Sound` 打开 `#32770` “声音” 对话框，GUI 保持响应。
- [x] 用户取消提权或修复失败时，GUI 给出明确错误。Windows 11：取消 UAC 后显示 “Firewall repair was cancelled because administrator permission was not granted.”，无崩溃，已有规则不受影响。
- [x] 非 Windows 平台不显示这两个入口。

## Comments

2026-11-09 — branch `ticket/11-windows-maintenance` (worktree
`/home/w0n9/workspaces/audio-share-wt/11`).

Implemented:
- `gui/src-tauri/src/maintenance.rs`: pure, unit-tested profile-mask ->
  `profile=` conversion (Public kept only when it is the only active profile),
  PowerShell repair-script generation with single-quote escaping, argument
  quoting, UAC-cancel HRESULT (1223) classification, netsh exit-code mapping,
  and en-US/zh-CN localization of every error.
- `gui/src-tauri/src/windows_maintenance.rs` (`#[cfg(windows)]`): active
  profiles via `INetFwPolicy2::CurrentProfileTypes` (COM, not localized netsh
  output); temp `.ps1`; `ShellExecuteExW` `runas`, wait + exit code on
  `spawn_blocking`; temp script deleted on every path;
  `control.exe /name Microsoft.Sound` spawned without waiting.
- Commands `repair_firewall` and `open_sound_panel` registered in `main.rs`
  (non-Windows returns the localized unsupported error); Windows-only settings
  rows with success/error text; en-US/zh-CN keys.
- `windows 0.62.2` features added under `[target.'cfg(windows)'.dependencies]`
  (minor not bumped). Rules: `Audio Share Server(TCP-In)` /
  `Audio Share Server(UDP-In)`, inbound, allow, enabled, application-scoped,
  deleted before add.
- Docs: `docs/server-rewrite/windows-maintenance.md` (rules, profiles,
  elevation/UAC, manual steps, residual list).

Commands/results (Ubuntu 24.04; this host has no Windows runtime or MSVC
linker):
- `cargo test -p audio-share-server-gui` 72 passed (9 new maintenance tests:
  profile combinations/fallbacks, script order/contents, quoting for spaces
  and quotes, unsafe input rejection, UAC-cancel and exit-code mapping,
  bilingual messages); `cargo test --workspace` pass.
- `cargo fmt --all -- --check` pass; `cargo clippy --workspace --all-targets --
  -D warnings` pass.
- `cargo check`/`cargo clippy --target x86_64-pc-windows-msvc -p
  audio-share-server-gui -- -D warnings` pass (only the known tauri-build GNU
  compiler warnings on this host).
- `npm run check` 0 errors/0 warnings; `npm run build` pass.
- Linux gating verified at the model/build level: bootstrap `platform` is
  `linux` and both rows are inside `{#if server.platform === "windows"}`; the
  Linux build contains no visible entry. No Linux screenshot was taken.

Executed on Windows 11 Enterprise 10.0.26300 (2026-11-10):

- UAC approval: both rules created and verified with
  `netsh advfirewall firewall show rule name="Audio Share Server(TCP-In)"
  verbose` (and UDP): Enabled Yes, Direction In, Profiles Domain,Private,
  Program = the running exe, Action Allow, Protocol TCP/UDP.
- UAC cancellation: GUI showed a readable, localized error
  (“Firewall repair was cancelled because administrator permission was not
  granted.”), stayed responsive, and left the existing rules intact.
- Sound Panel: opened the classic `#32770` “声音” dialog; the GUI stayed
  responsive while it was open.
- Connectivity: a local client over the WLAN IP `10.8.144.155:65530` received
  120 aligned datagrams through the repaired rules.
- en-US and zh-CN maintenance rows/errors were seen (the app was switched
  between both languages).

Remaining (tracked in tickets 15/16): the released v0.3.4 Android Client
connecting and playing through the repaired rules; the server-side LAN path and
rules are verified. Profile switching to a Public-only network was not
executed (the host has Domain/Private profiles).
