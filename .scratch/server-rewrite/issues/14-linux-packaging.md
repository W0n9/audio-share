# 14: Linux packaging

**What to build:** deb 与 AppImage；在 Ubuntu 24.04+ 安装与运行；明确 WebKitGTK 4.1 与 PipeWire 依赖；产物名可预测并写入发布说明。

**Blocked by:** 12 (Linux equivalents)

**Status:** resolved

- [x] deb 可安装、卸载并启动。
- [x] AppImage 可下载、`chmod +x` 后运行。
- [x] 依赖缺失时给出可读错误或文档说明。
- [x] deb 与 AppImage 产物名可预测，并写入发布说明。
- [x] 在干净的 Ubuntu 24.04 环境验证依赖。
- [x] 托盘与 Start/Stop 在打包版本中可用。

## Comments

2026-11-09, branch `ticket/14-linux-packaging`.

Build (Ubuntu 24.04.5 host, not a fresh VM; release profile):
- `cd gui && npm ci && npm run check && npm run build` pass (svelte-check 0
  errors/0 warnings).
- `npx tauri build --bundles deb,appimage` passes without `NO_STRIP`.
  `tauri.linux.conf.json` provides `targets=["deb","appimage"]`, deb depends
  `["libpipewire-0.3-0"]` (Tauri appends its defaults), publisher, homepage,
  short/long description, Apache-2.0 license.
- Artifacts:
  `target/release/bundle/deb/AudioShareServer_0.4.0_amd64.deb`
  (6,869,236 B / 6.55 MiB) and
  `target/release/bundle/appimage/AudioShareServer_0.4.0_amd64.AppImage`
  (85,248,504 B / 81.30 MiB).
- `cargo build` at the repo root and `cargo build -p audio-share-server-gui`
  pass with no `gui/dist/` (dev-URL binary). `gui/dist/index.html` was removed
  from git; the directory is generated and ignored, so builds leave a clean
  `git status`.

deb metadata and install:
- `dpkg -I` shows `Package: audio-share-server`, `Version: 0.4.0`,
  `Architecture: amd64`, `Maintainer: Audio Share maintainers`,
  `Homepage: https://github.com/mkckr0/audio-share`, description, and
  `Depends: libpipewire-0.3-0, libayatana-appindicator3-1,
  libwebkit2gtk-4.1-0, libgtk-3-0`.
- `sudo dpkg -i` succeeds; installs `/usr/bin/AudioShareServer`, the desktop
  entry, and hicolor icons. `ldd` reports no missing libraries.
- `xvfb-run + dbus-run-session + private PipeWire null sink`:
  `/usr/bin/AudioShareServer --smoke-test` printed
  `SMOKE visible=Ok(true) size=Ok(PhysicalSize { width: 420, height: 640 })
  title=Ok("Audio Share Server")` and exited 0. The installed binary cannot
  write `/usr/bin`, so config fell back to
  `~/.config/audio-share-server/config.ini` (configVersion=2, port=65530).
- UI (xdotool, packaged binary): Start -> `LISTEN 192.168.123.24:65530`,
  `[App] Running=1`; Stop -> port closed, `Running=0`; footer Quit exited.
- Tray (minimal `org.kde.StatusNotifierWatcher`, same method as ticket 12): item
  `/org/ayatana/NotificationItem/tray_icon_tray_app_main_tray`, title
  `AudioShareServer`, status `Active`; dbusmenu layout Show / Start / Stop
  (disabled while stopped) / Exit. Activating Start over dbusmenu opened the
  port and flipped Start disabled/Stop enabled; Stop closed it; Exit ended the
  process. No visual icon/mouse right-click was performed (Xvfb has no panel).
- `sudo dpkg -r audio-share-server` removes the binary, desktop entry, and
  icons; `dpkg -s` reports not installed. `~/.config/audio-share-server/`
  remains by design.

AppImage:
- Direct execution with FUSE 2 (`libfuse2t64` installed) works; smoke test
  printed the same 420x640 window line and exited 0.
- `APPIMAGE_EXTRACT_AND_RUN=1` also works (exit 0) for hosts without FUSE 2.
- UI Start/Stop/Quit verified as above; no mount or process left behind.
  Bundles libwebkit2gtk-4.1, libgtk-3, libayatana-appindicator3; does not
  bundle libpipewire or a PipeWire daemon. Built on 24.04 => glibc 2.39
  baseline.

Clean Ubuntu 24.04 dependency check:
- Pristine `ubuntu:24.04` container: `apt-get install` of the deb resolved and
  installed libwebkit2gtk-4.1-0 2.52.6, libgtk-3-0t64 3.24.41,
  libayatana-appindicator3-1 0.5.93, libpipewire-0.3-0t64 1.0.5; `ldd` found
  no missing libraries. The GUI/runtime checks were not repeated in the
  container.

Release packaging and CI:
- `scripts/package-linux-artifacts.sh` (self-tested by
  `scripts/package-linux-artifacts.test.sh`, TDD red->green) writes
  `dist-linux/` with the deb/AppImage, `audio-share-server-cmd-linux.tar.gz`
  (top-level `audio-share-server-cmd/` with `as-cmd`, LICENSE, README.md), and
  sha256 sidecars; `sha256sum -c` passes.
- `.github/workflows/server-rewrite.yml` gains a `linux-packaging` job on
  `ubuntu-24.04` (Tauri/PipeWire/FUSE apt deps, Node 22, npm ci/build, tauri
  build, as-cmd, packaging self-test, uploads). Merged with ticket 13's
  `windows-packaging` job as a union; both jobs are retained.
- Verification: `cargo fmt --check`, `cargo clippy --workspace --all-targets
  -- -D warnings`, `cargo test --workspace`, npm check/build, and
  `cargo check --target x86_64-pc-windows-msvc -p audio-share-server-gui` all
  pass. shellcheck is not installed on the host.

Residual / not claimed:
- No real desktop panel: tray icon rendering and mouse right-click were not
  executed; D-Bus registration and menu actions were.
- No Snap/Flatpak. AppImage autostart records the `APPIMAGE` path, so moving
  the file breaks the XDG entry.
- Build-time AppImage tooling downloads `linuxdeploy` from GitHub; a
  network-restricted CI needs a primed `~/.cache/tauri` (or use `NO_STRIP=true`
  if stripping fails).
