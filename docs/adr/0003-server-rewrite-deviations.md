# ADR-0003: Intentional deviations from v0.3.4 in the 0.4.0 Server

Status: accepted (2026-11-09)

## Context

The 0.4.0 Server is a big-bang Rust replacement for the v0.3.4 C++ Server Core
and GUI. Behavioral parity with v0.3.4 is the baseline, and the Wire Protocol
is frozen so the released Android Client v0.3.4 keeps working unchanged. A few
v0.3.4 behaviors are unsafe, platform-dependent, or incompatible with packaged
Linux installs. This ADR records every intentional observable deviation so
future maintenance does not treat them as regressions.

The deviations below affect Server CLI/GUI behavior or file locations. None
changes the Wire Protocol; the compatibility contract remains
[`docs/protocol.md`](../protocol.md) and the v0.3.4 Android Client.

## Decisions

### 1. Writable config/log fallback on Linux packages

v0.3.4 always used `config.ini` next to the executable. Linux packages install
the GUI under `/usr/bin`, and AppImages mount read-only, so that location
cannot be written.

0.4.0 resolves the config file in this order:

1. `AUDIO_SHARE_CONFIG`, when set;
2. `config.ini` next to the executable, when that directory is writable;
3. `$XDG_CONFIG_HOME/audio-share-server/config.ini` (default
   `~/.config/audio-share-server/config.ini`).

`server.log` always sits next to the resolved `config.ini`. Portable Windows
and development builds therefore keep the v0.3.4 location; packaged Linux
installs use the XDG location. If a v0.3.4 `config.ini` is found in the
executable directory while the fallback is active, it is migrated rather than
ignored.

Consequences: Linux users upgrading a package install no longer need write
access to `/usr/bin`; scripts that expect a `config.ini` next to a packaged
binary will not find one. The override variable is new.

### 2. `as-cmd` writes `server.log`

v0.3.4 `as-cmd` logged to stdout only; only the GUI wrote `server.log`.
Ticket 07 and the spec require the CLI to keep a log file for troubleshooting.

0.4.0 `as-cmd` writes the same lines to stdout and to `server.log` in the
resolved config directory. The file is truncated on each server launch, is
never touched by `--help`, `--version`, `--list-encoding`, or `-l`, and the
default level is `info` (`-V` raises it to `trace`). Fatal errors are written
to the file as well.

Consequences: running `as-cmd -b` from a writable directory now creates or
truncates `server.log`; the old CLI created no file. A failure to open the log
file is a warning, not a startup failure.

### 3. New-install close-to-tray default

v0.3.4's `[App Settings] WhenClose` default was `0` (exit). The rewrite keeps
reading and writing the same values (`0` = exit, non-zero = minimize to tray),
so existing configs are unchanged. For a brand-new install with no
`WhenClose` key, the GUI now defaults to **minimize to tray** (`1`) because the
tray-first design treats closing the window as hiding it.

Consequences: users who never open the settings see different close behavior
on first run. Selecting Exit in Settings restores the v0.3.4 behavior and
persists `WhenClose=0`.

### 4. Unknown `--encoding` is rejected during parsing

v0.3.4 accepted any string; invalid values became `encoding_invalid` and
failed later, differently per platform (Linux exited 1 during capture; Windows
kept serving without audio).

0.4.0 validates `--encoding` against `default`, `f32`, `s8`, `s16`, `s24`, and
`s32` during argument parsing and exits 1 with a readable message. The six
valid values and the default are unchanged.

Consequences: a script that passed an invalid encoding now fails immediately
instead of starting a server that cannot capture. Exit code 1 is consistent
with other parse errors.

### 5. `-b value` (space form) is accepted

v0.3.4 used an option with an implicit value, so `--bind value` silently
ignored the space-separated value and used the default address; only
`--bind=value` set the requested address. 0.4.0 accepts both
`--bind=host:port` and `-b host:port`, as well as a bare `-b`.

Consequences: an old script that accidentally relied on the ignored value now
binds the address it named. No supported v0.3.4 usage depended on the quirk;
the documented `=` form and bare form behave identically.

### 6. Default host follows RFC 1918

v0.3.4 selected a "private" address by comparing an address with a mask that
was reused as the network value. It matched `10.x.x.x`, only `172.16.x.x` of
the `172.16.0.0/12` range, and also `192.169.x.x` through `192.175.x.x`.

0.4.0 uses `Ipv4Addr::is_private()`: `10.0.0.0/8`, `172.16.0.0/12`, and
`192.168.0.0/16`, in interface enumeration order, otherwise the first
non-loopback IPv4 address. This fixes both the missed `172.17`–`172.31`
addresses and the false positives above `192.168`.

Consequences: on networks with a `172.17`–`172.31` address or a
`192.169`–`192.175` address, the default Host shown by the GUI or used by a
bare `as-cmd -b` can differ from v0.3.4. The selected address is displayed and
can be overridden.

### 7. Capture format is known before the server binds

v0.3.4 bound TCP/UDP before capture negotiation completed. A client that sent
`GET_FORMAT` early received an empty protobuf (`size = 0`), which the Android
Client interprets as `INVALID`/0/0 and cannot play.

0.4.0 starts capture first and waits for the negotiated `AudioFormat` before
binding the listener and accepting clients. If capture cannot start (no
endpoint, backend unavailable, unsupported format), startup fails with a
readable error instead of serving an empty format forever. The published
format is immutable for the server session, so `GET_FORMAT` cannot observe a
torn or mid-renegotiation value.

Consequences: the frozen client sees a valid `AudioFormat` on its first
`GET_FORMAT`. A server that would previously stay alive with no audio (for
example an unknown endpoint) now fails to start; `as-cmd` exits 1 and the GUI
shows the error. This is a deliberate safety improvement, not a protocol
change.

## Not deviations from v0.3.4

The following are intentional **fixes** that preserve the protocol and the
documented behavior, and are covered by tests rather than this ADR's deviation
list:

- Linux `s8` capture works (`U8` maps to `ENCODING_PCM_8BIT`); v0.3.4 exited
  on that path.
- `-l` without a running audio backend prints a readable error instead of
  segfaulting.
- Stop is cancelable and does not hang; Ctrl-C exits 0.
- Ports outside 0–65535 and negative channels/sample rates are rejected
  instead of wrapping or crashing.
- `configVersion` migration is non-destructive and reports its result.

## Consequences for release

- Server-only release: the Android Client stays at v0.3.4 and no APK is built
  or attached for 0.4.0.
- The Wire Protocol and `protos/client.proto` remain unchanged.
- Real v0.3.4 Android Client acceptance and long-run soak remain release-gate
  checks; scripted Wire Protocol replay and local clean-build verification are
  the automated gates.

## References

- [ADR-0001: rewrite baseline](0001-server-rewrite-baseline.md)
- [ADR-0002: Rust + Tauri v2 stack](0002-server-rewrite-stack.md)
- [Wire Protocol](../protocol.md)
- [as-cmd contract](../server-rewrite/as-cmd.md)
- [Configuration and log locations](../server-rewrite/linux.md)
