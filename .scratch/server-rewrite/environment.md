# Server rewrite — implementation environment (2026-11-09)

Shared context pointer for all implementer/merger agents. Do not edit the main repo from this file's instructions; this is environment knowledge only.

## Repo / branches
- Main repo: `/home/w0n9/workspaces/audio-share`
- Integration branch: `integration/server-rewrite` (based on `refactor` @ c7b31cd)
- Ticket worktrees live under `/home/w0n9/workspaces/audio-share-wt/<ticket>`
- Spec: `.scratch/server-rewrite/spec.md`; tickets: `.scratch/server-rewrite/issues/`
- Prototype A: `git show prototype/server-gui:prototype/server-gui/index.html` (branch `prototype/server-gui`)

## OS / toolchain
- Ubuntu 24.04.5, x86_64, no display (Xvfb installed), Rust 1.98.1, Node 22.18.0, npm 10.9.3.
- cargo uses a China mirror configured in `~/.cargo/config.toml` (`bfsu` sparse index); network works.
- Tauri build deps installed: webkit2gtk-4.1 (2.52.6), gtk+-3.0, libsoup-3.0, ayatana-appindicator3, librsvg2, patchelf.
- PipeWire 1.0.5 + wireplumber + pw-cli/pw-play/pw-record installed. No system daemon; use a private headless session.
- `libpipewire-0.3-dev` 1.0.5 installed.
- Old C++ deps installed: libasio-dev, libprotobuf-dev, protobuf-compiler, libspdlog-dev, libcxxopts-dev, libfmt-dev.

## Windows type-checking IS available
`cargo check --target x86_64-pc-windows-msvc` works. The target std was installed manually into the toolchain sysroot (rustup target add failed against the mirror). Verified with `wasapi 0.27.0` (it compiled). Use it for all `#[cfg(windows)]` code. `cargo clippy --target x86_64-pc-windows-msvc` should also work. There is no MSVC linker, so `cargo build`/`test` for Windows will fail at link time; `check`/`clippy` are the supported validation.
- `wasapi` crate resolved to 0.27.0; `windows` crate 0.62.2.

## Old v0.3.4 baseline assets
- Old `as-cmd` 0.3.4 Linux binary: `/home/w0n9/workspaces/audio-share-notes/server-rewrite/bin/as-cmd-0.3.4-linux`
- Source: `server-core/` (will be deleted at cutover ticket 16).
- It was built from a temp copy with two local patches (not in repo):
  1. `find_package(Protobuf CONFIG)` -> `find_package(Protobuf)` and an imported `asio::asio` interface target (Ubuntu packages lack CMake configs).
  2. `fmt::println` -> `fmt::print(... "\n")` because Ubuntu fmt is 9.1.0.
- If it must be rebuilt: copy `server-core` and `protos` into a temp tree, apply those patches, `cmake -S . -B build -DCMAKE_BUILD_TYPE=Release && cmake --build build -j`.

## Headless PipeWire recipe
```bash
TMP=$(mktemp -d)
export XDG_RUNTIME_DIR=$TMP/xdg; mkdir -p "$XDG_RUNTIME_DIR"; chmod 700 "$XDG_RUNTIME_DIR"
dbus-run-session -- bash -c '
  pipewire >/tmp/pw.log 2>&1 & PW=$!
  sleep 2
  wireplumber >/tmp/wp.log 2>&1 & WP=$!
  sleep 2
  pw-cli create-node adapter '{ factory.name=support.null-audio-sink node.name=TestSink node.description=TestSink media.class=Audio/Sink object.linger=1 audio.position=[FL,FR] priority.session=1000 }'
  sleep 1
  # play a tone into the sink while capturing:
  # pw-play --target TestSink tone.wav &
  # run as-cmd / tests here
  kill $WP $PW
'
```
- Verified: the adapter node appears as `media.class = "Audio/Sink"` with `object.serial = "38"` (serial varies).
- Generate a tone WAV (48 kHz, stereo, s16 or f32) with Python `wave` or `ffmpeg` if available.

## Useful facts
- Old as-cmd defaults to port 65530 when bind has no port; `-b` with no value uses the default address.
- Old server protocol: u32 LE cmd (GET_FORMAT=1, START_PLAY=2, HEARTBEAT=3); GET_FORMAT response cmd+u32 size+protobuf; START_PLAY response cmd+i32 id; heartbeat every 3s, timeout 5s; UDP client sends i32 LE id; PCM datagrams max 1464 bytes aligned down to block_align.
- `as-cmd` does not write `server.log`; only the MFC GUI does (verify in ticket 07).
