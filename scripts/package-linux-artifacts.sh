#!/usr/bin/env bash
# Package the Linux release artifacts without building them.
#
# Inputs (produced by `npx tauri build --bundles deb,appimage` and
# `cargo build --release -p as-cmd` in the Cargo workspace):
#   <bundle-dir>/deb/<productName>_<version>_<arch>.deb
#   <bundle-dir>/appimage/<productName>_<version>_<arch>.AppImage
#   <target-dir>/as-cmd
#   <root>/LICENSE
#   <root>/README.md
#   <root>/gui/src-tauri/tauri.conf.json   (productName and version)
#
# Outputs (<out-dir> defaults to <root>/dist-linux):
#   <productName>_<version>_<arch>.deb
#   <productName>_<version>_<arch>.deb.sha256
#   <productName>_<version>_<arch>.AppImage
#   <productName>_<version>_<arch>.AppImage.sha256
#   audio-share-server-cmd-linux.tar.gz   (top-level audio-share-server-cmd/)
#   audio-share-server-cmd-linux.tar.gz.sha256
#
# The CLI archive keeps the v0.3.4 layout: a top-level
# `audio-share-server-cmd` directory containing `as-cmd` plus the license and
# readme. With a Cargo workspace the build outputs live under
# `<root>/target/release`, not `gui/src-tauri/target/release`.
#
# Usage:
#   scripts/package-linux-artifacts.sh [--root DIR] [--bundle-dir DIR]
#                                      [--target-dir DIR] [--out-dir DIR]
#                                      [--arch ARCH]
#
# Run `scripts/package-linux-artifacts.test.sh` to test this script.

set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=lib.sh
source "$SCRIPT_DIR/lib.sh"
ROOT=$(cd "$SCRIPT_DIR/.." && pwd)
ROOT_ARG=
BUNDLE_ARG=
TARGET_ARG=
OUT_ARG=
ARCH_ARG=

CLI_BINARY=as-cmd
CLI_ARCHIVE=audio-share-server-cmd-linux.tar.gz
CLI_ARCHIVE_DIR=audio-share-server-cmd

usage() {
    cat <<'EOF'
Usage: scripts/package-linux-artifacts.sh [options]

Packages Linux bundle outputs into a flat artifact directory.

Options:
  --root DIR        Repository root (default: this script's parent)
  --bundle-dir DIR  Tauri bundle root (default: <root>/target/release/bundle)
  --target-dir DIR  Cargo release output (default: <root>/target/release)
  --out-dir DIR     Artifact output (default: <root>/dist-linux)
  --arch ARCH       Bundle architecture (default: dpkg's, else amd64)
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
    --bundle-dir)
        [[ $# -ge 2 ]] || fail "--bundle-dir needs a value"
        BUNDLE_ARG=$2
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
    --arch)
        [[ $# -ge 2 ]] || fail "--arch needs a value"
        ARCH_ARG=$2
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
BUNDLE_DIR=$(resolve_path "${BUNDLE_ARG:-target/release/bundle}")
TARGET_DIR=$(resolve_path "${TARGET_ARG:-target/release}")
OUT_DIR=$(resolve_path "${OUT_ARG:-dist-linux}")

default_arch() {
    if command -v dpkg >/dev/null 2>&1; then
        dpkg --print-architecture
        return
    fi
    case "$(uname -m)" in
    x86_64 | amd64) echo amd64 ;;
    aarch64 | arm64) echo arm64 ;;
    *) uname -m ;;
    esac
}

ARCH=${ARCH_ARG:-$(default_arch)}

CONFIG="$ROOT/gui/src-tauri/tauri.conf.json"

require_file "$CONFIG"
PRODUCT=$(sed -n 's/.*"productName"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$CONFIG" | head -n 1)
VERSION=$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$CONFIG" | head -n 1)
[[ -n "$PRODUCT" ]] || fail "no productName found in $CONFIG"
[[ -n "$VERSION" ]] || fail "no version found in $CONFIG"

DEB_NAME="${PRODUCT}_${VERSION}_${ARCH}.deb"
APPIMAGE_NAME="${PRODUCT}_${VERSION}_${ARCH}.AppImage"

require_file "$BUNDLE_DIR/deb/$DEB_NAME"
require_file "$BUNDLE_DIR/appimage/$APPIMAGE_NAME"
require_file "$TARGET_DIR/$CLI_BINARY"
require_file "$ROOT/LICENSE"
require_file "$ROOT/README.md"

mkdir -p "$OUT_DIR"
OUT_DIR=$(cd "$OUT_DIR" && pwd)

# Bundles already carry predictable names; copy them to the flat artifact
# directory unchanged so release notes name exactly what the build produced.
cp -f "$BUNDLE_DIR/deb/$DEB_NAME" "$OUT_DIR/$DEB_NAME"
write_sha256 "$OUT_DIR/$DEB_NAME"
cp -f "$BUNDLE_DIR/appimage/$APPIMAGE_NAME" "$OUT_DIR/$APPIMAGE_NAME"
write_sha256 "$OUT_DIR/$APPIMAGE_NAME"

# CLI archive. Stage the v0.3.4 layout directly in the output directory; remove
# any stale staging directory first so re-runs cannot duplicate entries.
STAGE_DIR="$OUT_DIR/$CLI_ARCHIVE_DIR"
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR"
cp -f "$TARGET_DIR/$CLI_BINARY" "$STAGE_DIR/$CLI_BINARY"
chmod 0755 "$STAGE_DIR/$CLI_BINARY"
cp -f "$ROOT/LICENSE" "$STAGE_DIR/LICENSE"
cp -f "$ROOT/README.md" "$STAGE_DIR/README.md"

rm -f "$OUT_DIR/$CLI_ARCHIVE"
tar -C "$OUT_DIR" -czf "$OUT_DIR/$CLI_ARCHIVE" "$CLI_ARCHIVE_DIR"
rm -rf "$STAGE_DIR"
write_sha256 "$OUT_DIR/$CLI_ARCHIVE"

printf 'packaged: %s\n' \
    "$OUT_DIR/$DEB_NAME" \
    "$OUT_DIR/$DEB_NAME.sha256" \
    "$OUT_DIR/$APPIMAGE_NAME" \
    "$OUT_DIR/$APPIMAGE_NAME.sha256" \
    "$OUT_DIR/$CLI_ARCHIVE" \
    "$OUT_DIR/$CLI_ARCHIVE.sha256"
