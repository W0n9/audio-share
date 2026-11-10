# 16: Big-bang cutover to 0.4.0

**What to build:** 删除旧 Server Core 与 MFC Server GUI；更新 README、协议说明、构建与发布文档；版本 `0.4.0`；CI 产出最终产物；形成干净的 release candidate。

**Blocked by:** 15 (v0.3.4 Android Client acceptance + soak)

**Status:** ready-for-human

- [x] 旧 Server Core 与 MFC Server GUI 从主分支移除。
- [x] README 覆盖构建、运行、打包、Linux 依赖与已知限制。
- [x] 版本号为 `0.4.0`，产物名与发布说明一致。
- [x] CI 在 `windows-latest` 与 `ubuntu-24.04` 产出最终产物。
- [x] 干净检出后可复现构建与测试。
- [ ] release candidate 通过 v0.3.4 Android Client 冒烟。

## Comments

2026-11-09 — branch `ticket/16-cutover`, commits `707619a` (legacy deletion),
`b724ed2` (0.4.0 versioning), `1da41ac` (docs), `dc7ddfb` (CI/release).
Integration tip at update time was `b456c10`; `git merge integration/server-rewrite`
reported "already up to date", so no merge commit was needed. Ticket 15 docs were
not present on the integration branch.

### Cutover and versioning

- Deleted `server-core/` and `server-mfc/` entirely (56 files, including CMake,
  vcpkg presets, MFC project/resources, `pack.sh`, the old unit test, and icon
  sources) plus `.github/workflows/build-server-core-linux.yml`.
- Kept `protos/client.proto`, `LICENSE`, `metadata/`, and the Android app
  untouched. `android-app` versionName/versionCode remain `0.3.4`.
- `VERSION` is `0.4.0` with no trailing newline. The workspace, all six crates,
  `gui/package.json`, `gui/package-lock.json`, `gui/src-tauri/Cargo.toml`, and
  `gui/src-tauri/tauri.conf.json` are `0.4.0`.
- `scripts/apply_version.sh` now updates the Rust workspace (`Cargo.toml`,
  workspace entries in `Cargo.lock`), the GUI npm manifests/lockfile, the GUI
  crate, and the Tauri config; the CMake/MFC sed lines are gone. It fails if any
  checked location disagrees with `VERSION`.

### Docs and release metadata

- README rewritten for the Rust Server: Windows portable exe + WebView2, Linux
  deb/AppImage and dependencies, cargo+npm builds, `as-cmd` flags, config/log
  locations, frozen v0.3.4 protocol compatibility, known limitations, license.
- `docs/protocol.md` expanded with TCP framing, little-endian integers, command
  values, `AudioFormat` protobuf, heartbeat 3 s / timeout ~6 s, UDP bind/MTU
  rules, the 0.4.0 startup-format guarantee, and stop semantics.
- Added `docs/server-rewrite/README.md` index and
  `docs/adr/0003-server-rewrite-deviations.md` (writable config fallback,
  CLI `server.log`, close-to-tray new-install default, unknown-encoding parse
  rejection, `-b value` acceptance, RFC1918 host correction,
  format-known-before-bind).
- Added `metadata/en-US/changelogs/4000.txt`; it states the Android Client stays
  at v0.3.4.

### CI and release

- `.github/workflows/server-rewrite.yml` keeps rust/windows-packaging/
  linux-packaging, adds `workflow_call`, and fails when version metadata is not
  in sync with `VERSION`.
- `release.yml` calls the packaging workflow, verifies the exact
  `AudioShareServer.exe`, `audio-share-server-cmd-windows.zip`,
  `AudioShareServer_0.4.0_amd64.deb`, `AudioShareServer_0.4.0_amd64.AppImage`,
  and `audio-share-server-cmd-linux.tar.gz` names plus their `.sha256` files,
  rejects any APK, and creates a draft `v0.4.0` release from changelog 4000.
- `scripts/setup_sourceforge_release.sh` and `update_sourceforge.yml` point at
  the new Windows exe and Linux AppImage defaults and no longer set an Android
  default.

### Clean-build verification (fresh clone of `dc7ddfb` at `/tmp/ash-16-verify`)

- `cargo fmt --check` — pass.
- `cargo clippy --workspace --all-targets -- -D warnings` — pass.
- `cargo test --workspace` — 263 passed, 0 failed, 3 ignored (PipeWire-gated:
  `cli_pipewire`, `capture_linux`, `headless_pipewire`).
- `cd gui && npm ci && npm run check && npm run build` — 95 files, 0 errors,
  0 warnings; Vite build succeeded.
- `cd gui && npx tauri build --debug --no-bundle` — built
  `target/debug/AudioShareServer`.
- `bash scripts/package-linux-artifacts.test.sh` — PASS.
- `bash scripts/package-windows-artifacts.test.sh` — PASS.
- `cargo build --release -p as-cmd`, then
  `cargo run -p protocol-harness -- replay --fixture fixtures/protocol/v0.3.4/linux/f32-48k-stereo --server-cmd target/release/as-cmd --server-arg=--bind=127.0.0.1:{port} --server-arg=--fake-capture`
  — live replay ok, `format_bytes_match=true`, `start_play_id=1`, 1 heartbeat,
  1770 datagrams, aligned payload 1464.
- `cargo test -p as-cmd --test cli_pipewire -- --ignored --nocapture` — pass
  against a real headless PipeWire 1.0.5 + WirePlumber null sink (real capture
  replay and SIGINT stop).
- `actionlint` v1.7.12 on `server-rewrite.yml`, `release.yml`,
  `update_sourceforge.yml`, and `test.yml` — pass.
- `git status` clean after all builds; `gui/dist` and `target` remain ignored.
- No `vcpkg`/`MSVC`/`MFC`/`server-core`/`server-mfc` references remain in
  README, docs, scripts, or CI, except historical `docs/research/` and
  ADR-0001/0002. The only lowercase `msvc` strings are the official Rust target
  triple `x86_64-pc-windows-msvc` in verification commands.

### Residual release gates (NOT EXECUTED here)

- **Real v0.3.4 Android Client smoke on a phone is NOT EXECUTED** (no Android
  device in this environment). The final checkbox remains unchecked; no
  real-client pass is claimed.
- Ticket 15 acceptance/soak evidence (real-APK format/channel/rate matrix,
  8-hour soak, hot-plug/sleep, GUI acceptance checklist) is not on the
  integration branch yet; this ticket does not claim those results.
- Remote GitHub Actions execution and real Windows runtime (portable exe /
  WebView2 preflight, WASAPI capture, UAC firewall repair) were not available
  on this Linux host. CI configuration was validated with `actionlint`; the
  Linux jobs' commands were reproduced locally as listed above.

### 2026-11-10 — Windows 11 runtime + fixes

The Windows runtime portion of this ticket was executed on Windows 11
Enterprise 10.0.26300 with the CI artifacts from run 42710876060 and a local
MSVC build. The portable exe, WebView2-present launch, real WASAPI capture,
the full GUI checklist, firewall UAC, Sound Panel, and the v0.3.4 CLI A/B are
recorded in `docs/server-rewrite/build-windows.md`. Two defects found in this
run were fixed:

- Windows update checks failed with “no TLS backend is configured”; the ureq
  `native-tls` feature now gets an explicit `native_tls::TlsConnector`
  (`schannel`).
- WASAPI `Initialize` for mono at the mix rate returned raw `0x88890008`; the
  backend and spike now map it to a readable error.
- The UDP broadcast loop awaited `send_to`; a slow Wi-Fi peer could fill the
  send buffer, block the loop and capture, and silence every client while TCP
  heartbeats kept sessions alive. It now uses `try_send_to` and drops on
  `WouldBlock` (commit `c6aeecd`). This third fix also needs a fresh CI run.

Consequences for the 0.4.0 RC: the follow-up CI run
[42766244106](https://github.com/W0n9/audio-share/actions/runs/42766244106)
(`ebdd468`) is green on all four jobs, and its Windows artifacts include the
TLS and `Initialize` fixes and were smoke-tested on Windows 11 (as-cmd
version/list/encoding, mono readable error, GUI HTTPS update check reaching
GitHub). The broadcast fix is later than that run; a further CI run is required
before the final artifact. The older `bad71d3` artifacts remain the pre-fix
baseline. The real v0.3.4 APK smoke was executed on one Android 12 phone
(single-phone matrix + 60-minute playback); the two-Android-client and mono
items remain open, so the final checkbox stays unchecked and `Status` stays
`ready-for-human`.
