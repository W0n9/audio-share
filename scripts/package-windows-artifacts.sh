#!/usr/bin/env bash
# Package the Windows release artifacts without building them.
#
# Inputs (produced by `npx tauri build --no-bundle` and
# `cargo build --release -p as-cmd` in the Cargo workspace):
#   <target-dir>/AudioShareServer.exe
#   <target-dir>/as-cmd.exe
#   <root>/LICENSE
#   <root>/README.md
#
# Outputs (<out-dir> defaults to <root>/dist-windows):
#   AudioShareServer.exe
#   AudioShareServer.exe.sha256
#   audio-share-server-cmd-windows.zip   (top-level audio-share-server-cmd/)
#   audio-share-server-cmd-windows.zip.sha256
#
# The archive keeps the v0.3.4 layout and names: a top-level
# `audio-share-server-cmd` directory containing `as-cmd.exe` plus the license
# and readme. With a Cargo workspace the build outputs live under
# `<root>/target/release`, not `gui/src-tauri/target/release`.
#
# Usage:
#   scripts/package-windows-artifacts.sh [--root DIR] [--target-dir DIR]
#                                        [--out-dir DIR]
#
# Run `scripts/package-windows-artifacts.test.sh` to test this script.

set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=lib.sh
source "$SCRIPT_DIR/lib.sh"
ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
ROOT_ARG=
TARGET_ARG=
OUT_ARG=

GUI_BINARY=AudioShareServer.exe
CLI_BINARY=as-cmd.exe
CLI_ARCHIVE=audio-share-server-cmd-windows.zip
CLI_ARCHIVE_DIR=audio-share-server-cmd

usage() {
    cat <<'EOF'
Usage: scripts/package-windows-artifacts.sh [options]

Packages Windows build outputs into a flat artifact directory.

Options:
  --root DIR        Repository root (default: this script's parent)
  --target-dir DIR  Cargo release output (default: <root>/target/release)
  --out-dir DIR     Artifact output (default: <root>/dist-windows)
  -h, --help        Show this help

Relative paths are resolved against --root.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
    --root)
        [[ $# -ge 2 ]] || fail "--root needs a value"
        ROOT_ARG=$2
        shift 2
        ;;
    --target-dir)
        [[ $# -ge 2 ]] || fail "--target-dir needs a value"
        TARGET_ARG=$2
        shift 2
        ;;
    --out-dir)
        [[ $# -ge 2 ]] || fail "--out-dir needs a value"
        OUT_ARG=$2
        shift 2
        ;;
    -h | --help)
        usage
        exit 0
        ;;
    *)
        usage >&2
        fail "unknown argument: $1"
        ;;
    esac
done

[[ -n "$ROOT_ARG" ]] && ROOT=$(resolve_path "$ROOT_ARG")
TARGET_DIR=$(resolve_path "${TARGET_ARG:-target/release}")
OUT_DIR=$(resolve_path "${OUT_ARG:-dist-windows}")

create_zip() {
    local directory=$1
    local archive=$2
    local parent name archive_name
    parent=$(dirname "$directory")
    name=$(basename "$directory")
    archive_name=$(basename "$archive")
    rm -f "$archive"
    # Use the archive's basename so a native Windows 7z/zip never has to parse
    # a POSIX-style absolute path from Git Bash.
    if command -v 7z >/dev/null 2>&1; then
        (cd "$parent" && 7z a -tzip "$archive_name" "$name" >/dev/null)
    elif command -v zip >/dev/null 2>&1; then
        (cd "$parent" && zip -qr "$archive_name" "$name")
    else
        fail "no zip tool found (7z or zip)"
    fi
}

require_file "$TARGET_DIR/$GUI_BINARY"
require_file "$TARGET_DIR/$CLI_BINARY"
require_file "$ROOT/LICENSE"
require_file "$ROOT/README.md"

mkdir -p "$OUT_DIR"
OUT_DIR=$(cd "$OUT_DIR" && pwd)

# Portable GUI exe: already named AudioShareServer.exe by the Cargo [[bin]];
# copy it to the flat artifact directory under the preserved name.
cp -f "$TARGET_DIR/$GUI_BINARY" "$OUT_DIR/$GUI_BINARY"
write_sha256 "$OUT_DIR/$GUI_BINARY"

# CLI archive. Stage the v0.3.4 layout directly in the output directory so a
# native Windows 7z/zip receives only relative names from Git Bash; remove any
# stale staging directory first so re-runs cannot duplicate entries.
STAGE_DIR="$OUT_DIR/$CLI_ARCHIVE_DIR"
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR"
cp -f "$TARGET_DIR/$CLI_BINARY" "$STAGE_DIR/$CLI_BINARY"
cp -f "$ROOT/LICENSE" "$STAGE_DIR/LICENSE"
cp -f "$ROOT/README.md" "$STAGE_DIR/README.md"

create_zip "$STAGE_DIR" "$OUT_DIR/$CLI_ARCHIVE"
rm -rf "$STAGE_DIR"
write_sha256 "$OUT_DIR/$CLI_ARCHIVE"

printf 'packaged: %s\n' \
    "$OUT_DIR/$GUI_BINARY" \
    "$OUT_DIR/$GUI_BINARY.sha256" \
    "$OUT_DIR/$CLI_ARCHIVE" \
    "$OUT_DIR/$CLI_ARCHIVE.sha256"
