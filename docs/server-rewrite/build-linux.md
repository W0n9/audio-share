# Linux build and packaging (ticket 14)

Status: **deb and AppImage built, installed, run, and uninstalled on the
development host** (Ubuntu 24.04.5, x86_64, no physical display). This host is
not a pristine VM or container; dependency availability was checked on it and
the runtime was exercised headlessly under Xvfb, a private D-Bus session, and a
private PipeWire instance with a null sink. The tray icon was exercised through
a minimal StatusNotifierWatcher plus `com.canonical.dbusmenu` calls (menu items
and actions verified); visual icon rendering still needs a real desktop panel.
See [Known limitations](#known-limitations).

The Linux release ships two GUI bundles plus the `as-cmd` archive. Protocol and
behavior are identical to the Windows build; only packaging and the
platform-specific capture backend differ.

## Artifacts

Built with `npx tauri build --bundles deb,appimage` from `gui/` and
`cargo build --release -p as-cmd` from the repository root.

| Artifact | Exact name | Size | Contents |
|---|---|---:|---|
| Debian package | `AudioShareServer_0.4.0_amd64.deb` | 6,869,272 B (6.55 MiB) | `/usr/bin/AudioShareServer`, desktop entry, hicolor icons |
| AppImage | `AudioShareServer_0.4.0_amd64.AppImage` | 85,252,600 B (81.30 MiB) | Self-contained GUI image (bundles WebKitGTK/GTK libraries) |
| `as-cmd` archive | `audio-share-server-cmd-linux.tar.gz` | 977,038 B (0.93 MiB) | Top-level `audio-share-server-cmd/` with `as-cmd`, `LICENSE`, `README.md` |

Each file has a `sha256sum`-format `.sha256` sidecar produced by
`scripts/package-linux-artifacts.sh`. The names are the Tauri bundle names:
`<productName>_<version>_<arch>.<bundle>`. The `as-cmd` archive keeps the
v0.3.4 name and layout; names do not include the version, which is embedded in
the binaries.

Local build checksums (for provenance, not a release commitment; final
artifacts were rebuilt after the integration merge and re-verified):

```text
9c8be4cb048d4de6b17b0a1c5e31ad86b62a2d7d88546def22f223da3fa60c4c  AudioShareServer_0.4.0_amd64.deb
c8717a00bdd63eb5fa4bce7245d8f6d99d5d3dfaa157063d63412f2a7c124f4a  AudioShareServer_0.4.0_amd64.AppImage
6481df396ac007c7d146c01a7ceb0c98668dabc77762207787a7f08234c132ed  audio-share-server-cmd-linux.tar.gz
```

## Runtime and build dependencies

The deb declares the audio runtime dependency explicitly and Tauri appends its
own defaults. Observed control file:

```text
Depends: libpipewire-0.3-0, libayatana-appindicator3-1, libwebkit2gtk-4.1-0, libgtk-3-0
```

`tauri.linux.conf.json` declares only the extra `libpipewire-0.3-0`; do not add
WebKitGTK/GTK/appindicator entries again or the control file gets duplicates.
On Ubuntu 24.04 the non-t64 names are satisfied by `Provides:`:

| Runtime dependency | Ubuntu 24.04 package |
|---|---|
| `libwebkit2gtk-4.1-0` | `libwebkit2gtk-4.1-0` |
| `libgtk-3-0` | `libgtk-3-0t64` (Provides) |
| `libayatana-appindicator3-1` | `libayatana-appindicator3-1` |
| `libpipewire-0.3-0` | `libpipewire-0.3-0t64` (Provides) |
| FUSE 2 (AppImage direct execution) | `libfuse2t64` (Provides `libfuse2`) |

A running PipeWire session (PipeWire + WirePlumber) is required for real audio
capture. `ldd /usr/bin/AudioShareServer` on the verification host resolved every
library with no `not found` entries, including `libwebkit2gtk-4.1.so.0`,
`libgtk-3.so.0`, `libsoup-3.0.so.0`, and `libpipewire-0.3.so.0`.
`libayatana-appindicator3-1` is required for the tray and is declared by the deb.

Build dependencies on Ubuntu 24.04:

```bash
sudo apt-get install -y build-essential curl wget file pkg-config libssl-dev \
  libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev \
  libayatana-appindicator3-dev librsvg2-dev libxdo-dev patchelf \
  libpipewire-0.3-dev libspa-0.2-dev libclang-dev libfuse2t64
```

Node.js 22+, npm, and Rust stable (MSRV 1.90) are also required.

### Clean Ubuntu 24.04 dependency check

The deb was installed into a pristine `ubuntu:24.04` container (no prior
packages), which resolved and installed every declared dependency:

```bash
docker run --rm -v "$PWD/AudioShareServer_0.4.0_amd64.deb:/tmp/app.deb:ro" \
  ubuntu:24.04 bash -c '
    export DEBIAN_FRONTEND=noninteractive
    apt-get update && apt-get install -y --no-install-recommends /tmp/app.deb
    ! ldd /usr/bin/AudioShareServer | grep -q "not found"
  '
```

Observed packages: `libwebkit2gtk-4.1-0` 2.52.6, `libgtk-3-0t64` 3.24.41,
`libayatana-appindicator3-1` 0.5.93, `libpipewire-0.3-0t64` 1.0.5. `ldd`
reported no missing libraries. The GUI smoke/start-stop checks were run on the
host, not inside the container.

## Build

Run from the repository root. The Cargo workspace puts outputs under
`<repo>/target`, not `gui/src-tauri/target`.

```bash
cd gui
npm ci
npm run check                 # svelte-check
npm run build                 # generates the ignored gui/dist/
npx tauri build --bundles deb,appimage
cd ..
cargo build --release -p as-cmd
```

`tauri.linux.conf.json` supplies the Linux bundle configuration; the base
`gui/src-tauri/tauri.conf.json` keeps `bundle.targets: []` for the Windows
portable exe. The Linux config sets:

```json
{
  "bundle": {
    "targets": ["deb", "appimage"],
    "linux": { "deb": { "depends": ["libpipewire-0.3-0"] } }
  }
}
```

plus publisher, homepage, short/long description, and Apache-2.0 license
metadata. The AppImage build downloads `linuxdeploy` and its AppImage plugin at
build time; network access (or a primed `~/.cache/tauri`) is required. If
linuxdeploy stripping fails on a newer host, rerun with `NO_STRIP=true`. It did
not fail on this host.

`gui/dist/` is **not tracked**; it is generated by `npm run build` /
`tauri build` and ignored by git. A bare `cargo build -p audio-share-server-gui`
works without it: without `tauri/custom-protocol` that binary is a dev binary
that loads `devUrl` (`http://localhost:1420`). `tauri build` enables
`custom-protocol` and embeds the freshly built `dist/`.

Expected bundle paths:

```text
<repo>/target/release/bundle/deb/AudioShareServer_0.4.0_amd64.deb
<repo>/target/release/bundle/appimage/AudioShareServer_0.4.0_amd64.AppImage
```

Package the release artifacts and checksums (used by CI):

```bash
bash scripts/package-linux-artifacts.test.sh
bash scripts/package-linux-artifacts.sh \
  --root "$PWD" \
  --bundle-dir "$PWD/target/release/bundle" \
  --target-dir "$PWD/target/release" \
  --out-dir "$PWD/dist-linux" \
  --arch amd64
```

The script validates the predictable names derived from
`gui/src-tauri/tauri.conf.json`, copies the bundles, writes `.sha256` files for
all three artifacts, and creates the `as-cmd` tarball.

## Install, run, uninstall (deb)

```bash
sudo dpkg -i target/release/bundle/deb/AudioShareServer_0.4.0_amd64.deb
AudioShareServer                  # or start "AudioShareServer" from the desktop menu
sudo dpkg -r audio-share-server
```

`dpkg -i` resolved all declared dependencies on the verification host and
installed:

```text
/usr/bin/AudioShareServer
/usr/share/applications/AudioShareServer.desktop
/usr/share/icons/hicolor/{32x32,128x128,256x256@2}/apps/AudioShareServer.png
```

`sudo dpkg -r audio-share-server` removed the binary, desktop entry, and icons;
`dpkg -s audio-share-server` then reported the package as not installed. The
user configuration under `~/.config/audio-share-server/` is intentionally left
behind by `dpkg -r` (remove it manually to reset settings).

Start/Stop was verified through the actual installed UI under Xvfb: clicking
Start made the server listen on `192.168.123.24:65530` and wrote
`[App] Running=1`; clicking Stop closed the port and wrote `Running=0`; the
footer Quit exited the process.

The tray was verified in the installed package by running a minimal
`org.kde.StatusNotifierWatcher` on the private session bus. The app registered
`/org/ayatana/NotificationItem/tray_icon_tray_app_main_tray` with
`Title=AudioShareServer` and `Status=Active`. Its `com.canonical.dbusmenu`
layout contained Show, Start, Stop (disabled while stopped), a separator, and
Exit. Invoking Start through the menu opened the TCP port and wrote `Running=1`
(Start disabled, Stop enabled); invoking Stop closed the port and wrote
`Running=0`; invoking Exit terminated the process. This uses the same tray
path a desktop StatusNotifierHost uses, but does not test visual icon rendering
or mouse interaction.

## AppImage

```bash
chmod +x AudioShareServer_0.4.0_amd64.AppImage
./AudioShareServer_0.4.0_amd64.AppImage
```

Direct execution requires FUSE 2. On Ubuntu 24.04 install `libfuse2t64`
(provides `libfuse2`). Without FUSE 2, use:

```bash
APPIMAGE_EXTRACT_AND_RUN=1 ./AudioShareServer_0.4.0_amd64.AppImage
# or
./AudioShareServer_0.4.0_amd64.AppImage --appimage-extract
./squashfs-root/AppRun
```

Both direct (FUSE) and `APPIMAGE_EXTRACT_AND_RUN=1` runs were verified: the
window opened at 420x640 with title "Audio Share Server", Start exposed
`192.168.123.24:65530` and wrote `Running=1`, Stop closed it, and Quit exited.
No AppImage mount or process was left behind. The AppImage bundles WebKitGTK/GTK
libraries but **not** a PipeWire daemon; audio capture still needs the system
PipeWire session. It was built on Ubuntu 24.04 (glibc 2.39), so the baseline is
Ubuntu 24.04+; it will not run on 22.04.

## Config and log locations

Resolution order (implemented in `crates/audio-share-core/src/paths.rs`):

1. `AUDIO_SHARE_CONFIG` (absolute path to the config file), if set.
2. `config.ini` next to the executable, if that directory is writable
   (portable `as-cmd` tarball, Windows exe).
3. `$XDG_CONFIG_HOME/audio-share-server/config.ini`, default
   `~/.config/audio-share-server/config.ini`.

The installed deb lives in `/usr/bin` (not writable by the user) and the
AppImage mount is read-only, so both fall back to
`~/.config/audio-share-server/config.ini`. Verified after an installed-package
run: the file was created with `configVersion=2`, `port=65530`, and
`WhenClose=1`. `server.log` is resolved to the same directory; both `as-cmd`
and the GUI write it. The GUI truncates the file at application startup, and
`as-cmd` truncates it on each server launch.

## Headless verification recipe

```bash
TMP=$(mktemp -d)
export XDG_RUNTIME_DIR=$TMP/xdg
mkdir -p "$XDG_RUNTIME_DIR" && chmod 700 "$XDG_RUNTIME_DIR"
xvfb-run -a -s "-screen 0 1280x800x24" dbus-run-session -- bash -c '
  export WEBKIT_DISABLE_COMPOSITING_MODE=1 WEBKIT_DISABLE_DMABUF_RENDERER=1
  export LIBGL_ALWAYS_SOFTWARE=1 GDK_BACKEND=x11
  pipewire >/tmp/pw.log 2>&1 & PW=$!
  sleep 2
  wireplumber >/tmp/wp.log 2>&1 & WP=$!
  sleep 2
  pw-cli create-node adapter "{ factory.name=support.null-audio-sink
    node.name=TestSink node.description=TestSink media.class=Audio/Sink
    object.linger=1 audio.position=[FL,FR] priority.session=1000 }"
  sleep 1
  timeout 40 /usr/bin/AudioShareServer --smoke-test
  rc=$?
  kill $WP $PW
  exit $rc
'
```

The smoke test prints
`SMOKE visible=Ok(true) size=Ok(PhysicalSize { width: 420, height: 640 }) title=Ok("Audio Share Server")`
and exits 0. Harmless noise under Xvfb/headless includes `libEGL warning: failed
to open /dev/dri/...` and `error: fuse init failed: Can't mount path
/run/user/1000/doc` from the desktop portal; neither affects the app. A
StatusNotifierHost/panel is not present, so tray menu interaction is not
asserted there.

## CI

`.github/workflows/server-rewrite.yml` has a `linux-packaging` job on
`ubuntu-24.04` that installs the build dependencies above, runs
`npm ci`/`npm run build` and `npx tauri build --bundles deb,appimage`, builds
`as-cmd`, runs the packaging self-test, and uploads the deb, AppImage, and
`as-cmd` archive
archive (each with its `.sha256`) as separate artifacts. The Windows portable
packaging job is independent and kept alongside it.

## Known limitations

- Tauri on Linux does not emit tray left-click events; the tray is controlled
  through the right-click menu (menu Show/Start/Stop/Exit). The packaged tray
  menu was exercised over D-Bus with a minimal StatusNotifierWatcher;
  visual icon rendering and mouse interaction require a real desktop panel.
- The deb and AppImage need WebKitGTK 4.1, GTK 3, libayatana-appindicator3, and
  PipeWire. Missing runtime libraries are reported by `dpkg`/the dynamic loader;
  the AppImage bundles most GUI libraries but not PipeWire.
- AppImage direct execution requires FUSE 2 (`libfuse2t64`); otherwise use
  `APPIMAGE_EXTRACT_AND_RUN=1`.
- AppImage autostart records the `APPIMAGE` path; moving or renaming the file
  breaks the XDG autostart entry.
- The AppImage's glibc baseline is Ubuntu 24.04 (glibc 2.39).
- No Snap/Flatpak packaging in v0.4.0.
- The interactive/runtime verification host is Ubuntu 24.04.5 but not a freshly
  provisioned machine; it has the full Tauri/PipeWire development stack
  installed. Declared dependency resolution and dynamic linking were separately
  verified in a pristine `ubuntu:24.04` container (see above); the GUI/runtime
  checks were not repeated inside that container.
