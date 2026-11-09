# 13: Windows packaging

**What to build:** 便携 `AudioShareServer.exe`；启动时检测 WebView2，缺失则显示明确提示与下载链接；产物名保持；Windows 10+ 可直接运行。

**Blocked by:** 11 (Windows maintenance: Firewall Repair + Sound Panel)

**Status:** ready-for-human

- [x] 产物为便携 exe，无安装器。
- [ ] 有 WebView2 时正常启动。
- [ ] 无 WebView2 时显示明确提示与下载链接，不崩溃。
- [x] 产物名保持 `AudioShareServer.exe`。
- [ ] 在 Windows 10 与 11 上运行并通过基本 GUI 验收。

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

