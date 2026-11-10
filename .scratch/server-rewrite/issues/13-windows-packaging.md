# 13: Windows packaging

**What to build:** 便携 `AudioShareServer.exe`；启动时检测 WebView2，缺失则显示明确提示与下载链接；产物名保持；Windows 10+ 可直接运行。

**Blocked by:** 11 (Windows maintenance: Firewall Repair + Sound Panel)

**Status:** ready-for-human

> Windows 11 runtime executed on 2026-11-10: the portable exe launches with
> WebView2 present, the full GUI checklist passes, and the CI artifacts
> execute. Missing-WebView2 on a clean VM and Windows 10 are still not
> executed.

- [x] 产物为便携 exe，无安装器。
- [x] 有 WebView2 时正常启动。Windows 11 + WebView2 157.0.4341.42：窗口、托盘、Start/Stop 正常。
- [ ] 无 WebView2 时显示明确提示与下载链接，不崩溃。**NOT EXECUTED**：本机已安装 WebView2，且无干净 Windows 10 VM；决策逻辑有单元测试。
- [x] 产物名保持 `AudioShareServer.exe`。
- [ ] 在 Windows 10 与 11 上运行并通过基本 GUI 验收。Windows 11 PASS；Windows 10 未执行。

## Comments

2026-11-09 — branch `ticket/13-windows-packaging`.

Implemented (code/CI/docs):

- `gui/src-tauri/src/webview2_check.rs`: pure detection decision with 11 unit
  tests (HKLM `WOW6432Node` and HKCU `pv`; missing/blank/`0.0.0.0` => absent;
  fixed-version `WEBVIEW2_BROWSER_EXECUTABLE_FOLDER` + `msedgewebview2.exe`
  honored; machine > user precedence). Windows-only registry read and
  `MessageBoxW`/`ShellExecuteW` preflight; `main()` calls it before
  `tauri::Builder` and exits 0 when missing. Added
  `Win32_UI_WindowsAndMessaging` to the Windows features.
- `.github/workflows/server-rewrite.yml`: new `windows-packaging` job
  (`windows-latest`): Rust MSVC + Node 22, `npm ci`, `npm run build`,
  `npx tauri build --no-bundle`, `cargo build --release -p as-cmd`, asserts
  `<workspace>/target/release/{AudioShareServer.exe,as-cmd.exe}`, runs the
  packaging self-test and packager, uploads `AudioShareServer-windows`
  (exe + `.sha256`) and `audio-share-server-cmd-windows` (zip + `.sha256`).
- `scripts/package-windows-artifacts.sh` + `.test.sh`: flat
  `dist-windows/AudioShareServer.exe` (+ checksum) and
  `audio-share-server-cmd-windows.zip` with v0.3.4 layout
  (`audio-share-server-cmd/{as-cmd.exe,LICENSE,README.md}`) + checksum. The
  self-test covers exact names, archive layout, checksum format/content,
  re-run idempotency, and the missing-binary failure path.
- `docs/server-rewrite/build-windows.md`: build steps, WebView2 detection,
  portable exe/artifact paths, Windows 10/11 manual acceptance checklist.
- `tauri.conf.json` already had `productName`/`mainBinaryName` =
  `AudioShareServer`, version 0.4.0, stable identifier, `bundle.targets: []`,
  and `icon.ico`; left unchanged.

Verification on this Linux host (2026-11-09):

- `cargo fmt --check` OK.
- `cargo clippy --workspace --all-targets -- -D warnings` OK.
- `cargo test --workspace` OK (includes the 11 WebView2 tests).
- `cargo check`/`cargo clippy --target x86_64-pc-windows-msvc
  -p audio-share-server-gui --all-targets -- -D warnings` OK; only the two
  known `tauri-build` "GNU compiler is not supported for this target" warnings.
- `npm ci && npm run check && npm run build` OK; `gui/dist/index.html`
  restored to the committed placeholder afterward.
- `actionlint` v1.7.12 on the workflow: clean. Packaging self-test: PASS.

Residual Windows verification (not executed here):

1. Native MSVC build/link of `AudioShareServer.exe` (no Windows linker on this
   host); the first real build is the `windows-packaging` CI job.
2. Windows 10/11 runtime acceptance: window/tray, Start/Stop, endpoint list,
   config migration, firewall UAC, sound panel, autostart, single instance,
   i18n, update check, v0.3.4 Android Client playback.
3. WebView2 present and missing paths: dialog, Yes opens the Microsoft page,
   process exits 0, no Tauri panic; per-user and fixed-version detection.
4. SHA-256 of the real CI artifacts and executing the packaged `as-cmd.exe`
   (`--version`, `-l`, `--list-encoding`).

2026-11-09 — CI verification (branch `integration/server-rewrite` @ `6f1f429`).

GitHub Actions run
[42638565632](https://github.com/W0n9/audio-share/actions/runs/42638565632)
is green for all four jobs, including `Rust (windows-latest)` (fmt, clippy,
`cargo test --workspace`) and `Windows portable packaging`.

Downloaded and verified on the Linux host:

- `AudioShareServer-windows`: `AudioShareServer.exe` 11,918,848 B; `file`
  reports `PE32+ executable (GUI) x86-64, for MS Windows`; `sha256sum -c`
  passes (`27a57a65…75e2b`).
- `audio-share-server-cmd-windows`: zip 686,080 B; `sha256sum -c` passes
  (`eb745b78…ea60`); contains
  `audio-share-server-cmd/{as-cmd.exe,LICENSE,README.md}`.

Residual item 4 is partly closed: the artifact SHA-256 is verified. Executing
`as-cmd.exe` (`--version`, `-l`, `--list-encoding`) still requires Windows.

The first push exposed pre-existing CI defects, fixed in `95521b7`, `e3c584e`,
and `6f1f429`: the Ubuntu `rust` job lacked Tauri/WebKitGTK build dependencies;
the synthetic-capture stop test assumed an empty frame channel right after
`stop()`; the listener-release test assumed an immediate connect refusal on an
ephemeral port. Runtime boxes remain open; `Status` stays `ready-for-human`.

### 2026-11-10 — Windows 11 runtime acceptance

Host: Windows 11 Enterprise 10.0.26300 x64; WebView2 157.0.4341.42; MSVC
14.44.35207; Rust 1.99.1. Artifacts from CI run
[42710876060](https://github.com/W0n9/audio-share/actions/runs/42710876060)
(`bad71d3`) were downloaded and executed:

- `AudioShareServer.exe` 11,918,848 B, SHA-256
  `21c5b26478a0a4ad12241e60eec724b4613fbc07ad2f85c0fb479d1d2851faa6`
  (sidecar matches); `audio-share-server-cmd-windows.zip` 686,081 B, SHA-256
  `6f8f864779d0290ab720af5c3f1855075765b7652bdd0fe04533ad665af4bd42`.
- The portable exe launched without a console, from a path containing spaces;
  the window is 420×640 at 150% scaling; the process image is
  `AudioShareServer.exe`; the tray icon and menu work.
- Start/Stop used real WASAPI; a 10 s `protocol-harness probe` passed; Stop
  closed the client session; the WLAN path `10.8.144.155:65530` streamed to a
  local client.
- Settings persistence/migration/Reset, WhenClose 0/1, WhenAppStart 0/1/2,
  autostart HKCU entry, single instance (including `--hide`), update check,
  firewall UAC approve/cancel, Sound Panel, en-US/zh-CN, About/homepage, and
  `server.log` were all exercised. See
  `docs/server-rewrite/build-windows.md` for the full table.
- `as-cmd.exe --version` 0.4.0, `-l`, and `--list-encoding` execute from the
  zip.

Three runtime defects were found and fixed: the update check had no Windows
TLS backend (ureq `native-tls` needs an explicit connector), mono capture
surfaced a raw `0x88890008` instead of a readable error, and the UDP broadcast
loop could stall on a slow Wi-Fi peer. The final CI run
[42777988420](https://github.com/W0n9/audio-share/actions/runs/42777988420)
(`9626b10`) is green on all four jobs and its artifacts include all three fixes:

- `AudioShareServer.exe` 11,966,464 B, SHA-256
  `a3ee857ac315b7e6ec3b79ad1248317f75b1cbe6b20a6ed220100d0ec77b2cbf`.
- `audio-share-server-cmd-windows.zip` 688,182 B, SHA-256
  `216c95808d79a55c3309cf4b58004db4534165398083a19beb9378fa3a7993ef`.
- Smoke: `as-cmd --version` 0.4.0, `-l`, `--list-encoding`; mono printed the
  readable `AUDCLNT_E_UNSUPPORTED_FORMAT` message; the final `as-cmd.exe`
  (SHA-256 `8d984573…ea831c`) was played by the real v0.3.4 Android Client for
  ~11 minutes (6 probes, 1,997–2,000 datagrams/6 s, 440 Hz, 0 heartbeat misses,
  0 app crashes). The GUI HTTPS update check was smoke-tested on run
  42766244106.

Run 42766244106 (`ebdd468`, `AudioShareServer.exe`
`93a456642ad52db1c934b0f818b441cfe25a232650d1065ebff7e2e8abab2e0c` and zip
`b200f5fd8d8d4f84387028e586b717db72f6097249fdc8931a1cff1282c01839`) remains
the baseline that includes the first two fixes but not the broadcast fix; run
42710876060 remains the original pre-fix baseline.

Not executed: missing-WebView2 clean-VM dialog, per-user/fixed-version
WebView2, Windows 10, and a real reboot/login. A real Android Client was
executed on one Android 12 phone (see ticket 15).

