# 07: `as-cmd` parity

**What to build:** `as-cmd` 保留全部现有 flags 与语义：`-b/--bind`、`-l/--list-endpoint`、`--list-encoding`、`-e/--endpoint`、`--encoding`、`--channels`、`--sample-rate`、`-V/--verbose`、`-v/--version`、`-h/--help`；默认地址选择、`server.log` 与退出码保持可判断。Windows 与 Linux 都要列出真实 Endpoint。

**Blocked by:** 05 (Windows real capture path), 06 (Linux real capture path)

**Status:** resolved

- [x] 所有 flags 可用；帮助文本列出示例与默认值。
- [x] `-l` 在 Windows 与 Linux 列出真实 Endpoint 并标记默认。
- [x] `--list-encoding` 列出 5 种 encoding。
- [x] 默认 bind 地址选择与旧版一致。
- [x] `server.log` 继续写入；错误信息可读；退出码可判断。
- [x] 进程级断言覆盖 help、version、list 与 exit code。

## Comments

2026-11-09 — branch `ticket/07-as-cmd-parity` (worktree `audio-share-wt/07`).

Implemented:

- Full clap CLI with v0.3.4 flags/semantics: `-h/--help`,
  `-l/--list-endpoint`, `-b/--bind [[host][:port]]` (bare → default address,
  default port 65530; accepts bare, `-b value` and `--bind=value`),
  `-e/--endpoint <id|default>`, `--encoding
  <default|f32|s8|s16|s24|s32>`, `--list-encoding`, `--channels`,
  `--sample-rate`, `-V/--verbose`, `-v/--version` (custom; clap's default
  version flag disabled), plus `--fake-capture`. Help includes the old
  examples (with the machine's default address) and all defaults.
- Dispatch order matches v0.3.4: help > version > list-endpoint >
  list-encoding > bind. No action → full help on stderr, exit 0.
- Bind parsing reuses `core::server::default_host()` (first private IPv4 via
  `Ipv4Addr::is_private()`, else first non-loopback IPv4), rejects non-numeric
  and >65535 ports with readable errors, and treats an empty host as the
  default address.
- `-l` uses `core::capture::platform_backend()`, prints
  `\t{* }id: {:<4} name: {}` and `total: N`; with no PipeWire daemon it exits 1
  with a readable error instead of v0.3.4's SIGSEGV.
- New `audio-share-core::paths` module: `AUDIO_SHARE_CONFIG` → exe-dir
  `config.ini` when writable (probe write) → `$XDG_CONFIG_HOME/audio-share-server/`
  (default `~/.config/...`); `server.log` shares the resolved directory.
  Ticket 09 can reuse it for config persistence.
- Logging: stdout + `server.log` (truncated on launch), `info` default and
  `trace` with `-V`, v0.3.4-style `[YYYY-MM-DD HH:MM:SS.mmm] [info] message`
  lines (UTC). Old `as-cmd` only logged to stdout; the file sink is an
  intentional spec requirement.
- Ctrl-C stops capture and the network side and exits 0; start/bind/capture
  failures exit 1 with readable stderr. `server.log` also records fatal errors.
- Docs: `docs/server-rewrite/as-cmd.md` (flags, defaults, examples, exit codes,
  log/config paths, intentional differences, platform notes).

Commands and results (Ubuntu 24.04.5, Rust 1.98.1):

- `cargo test -p as-cmd` — 5 unit + 12 CLI process + 1 harness replay + 1 live
  server process passed; the headless PipeWire test is ignored by default.
- `cargo test -p as-cmd --test cli_pipewire -- --ignored --nocapture` — 1
  passed in ~4 s inside a disposable headless PipeWire/WirePlumber session:
  real `-l` found `TestSink` with the `*` default marker and `total: 1`; the
  protocol harness replayed the v0.3.4 fixture through the real capture
  backend; a real-backend server stopped with exit 0 on SIGINT.
- `cargo test --workspace` — all non-ignored tests pass (as-cmd 19; core 29
  unit + 17 protocol + 15 server + 17 wire + 6 paths + 5 synthetic; harness
  suites unchanged).
- `cargo fmt --all -- --check` — clean; `cargo clippy -p as-cmd -p
  audio-share-core --all-targets -- -D warnings` — clean;
  `cargo check --target x86_64-pc-windows-msvc -p as-cmd` — pass.
- Process tests verify `--help`/`-h` (examples, flags, defaults, exit 0),
  `--version`/`-v` (`0.4.0`, homepage), exact `--list-encoding` output, no-args
  help-to-stderr exit 0, unknown flag/encoding, bad/overflowing/negative ports,
  negative channels/sample rate, `-l` without a daemon (exit 1, no crash), live
  start + fixture replay + SIGINT exit 0 + session close, `server.log`
  creation/truncation/`[info]`/`[trace]` lines, and stdout mirroring.

Intentional deviations (also documented in `as-cmd.md`):

- Unknown `--encoding` is rejected at parse time (exit 1) instead of failing
  late/platform-dependently; ports are validated instead of wrapping modulo
  65536; `-b value` and empty hosts are accepted (old ignored/errored); extra
  colons and negative channels/rates are rejected; unknown endpoints now fail
  at startup with an actionable error instead of serving an empty format;
  Ctrl-C exits 0; `server.log` is new; version is `0.4.0`.
- Default host now follows RFC 1918 strictly: `172.16–172.31` are private and
  `192.169–192.175` are not (v0.3.4 matched only `172.16.x.x` and also matched
  `192.169–192.175`). This can change the default address on such networks.

Caveats:

- Windows endpoint listing/capture is implemented through the shared WASAPI
  backend and compile-checked with `cargo check --target
  x86_64-pc-windows-msvc -p as-cmd`; no Windows runtime was available here, so
  real Windows `-l`/capture remains manual verification.
- Real v0.3.4 Android APK acceptance and long-running A/B/soak remain ticket 15.
- `as-cmd` does not read `config.ini`; the path module only locates `server.log`
  (ticket 09 will use it for persistence).

## Comments

2026-11-09 — fix pass on branch `fix/code-review`: the default bind address was
restored to v0.3.4's exact mask comparison. `core::server::select_default_address`
now applies `(address & mask) == mask` with masks `0x0a000000`, `0xac100000`,
and `0xc0a80000` (loopback still filtered first), replacing the
`Ipv4Addr::is_private()` RFC 1918 check. That check had narrowed/widened the
default address on some networks, which contradicted ticket 07's "默认 bind
地址选择与旧版一致" requirement. The behavior is unit-tested with the loose
matches (for example `11.x`, `173.16`–`173.31`, `192.169`–`192.175`) and
recorded in ADR-0003 §6 and `docs/server-rewrite/as-cmd.md`. The `-b value`
acceptance and parse-time unknown-`--encoding` rejection remain intentional
deviations (ADR-0003 §4–5).
