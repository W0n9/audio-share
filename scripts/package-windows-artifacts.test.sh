#!/usr/bin/env bash
# Self-test for package-windows-artifacts.sh.
#
# Builds a fake Cargo target tree, runs the packager, and asserts the exact
# artifact names, archive layout, and checksum files. Runs on Linux locally and
# on the windows-latest runner through Git Bash/7-Zip.
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=lib.sh
source "$SCRIPT_DIR/lib.sh"
PACKAGER="$SCRIPT_DIR/package-windows-artifacts.sh"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

# --- path resolution --------------------------------------------------------
# $GITHUB_WORKSPACE reaches Git Bash as a native Windows path; it must not be
# joined onto ROOT (that produced /d/a/.../D:\a\... on CI).
(
    ROOT=/posix/root
    [[ "$(resolve_path 'D:\a\audio-share')" == 'D:\a\audio-share' ]] ||
        fail "drive-letter path must stay absolute"
    [[ "$(resolve_path 'D:/a/audio-share')" == 'D:/a/audio-share' ]] ||
        fail "forward-slash drive path must stay absolute"
    [[ "$(resolve_path '//server/share')" == '//server/share' ]] ||
        fail "UNC path must stay absolute"
    [[ "$(resolve_path 'relative/dir')" == '/posix/root/relative/dir' ]] ||
        fail "relative path must resolve against ROOT"
)

list_archive() {
    if command -v unzip >/dev/null 2>&1; then
        unzip -Z1 "$1" | tr '\\' '/'
    elif command -v 7z >/dev/null 2>&1; then
        # 7z -ba columns end with the path; artifact paths contain no spaces.
        7z l -ba "$1" | awk '{print $NF}' | tr '\\' '/'
    else
        fail "neither unzip nor 7z is available to inspect the archive"
    fi
}

# --- fake build outputs -----------------------------------------------------
mkdir -p "$TMP/target/release"
printf 'fake-gui-binary\n' >"$TMP/target/release/AudioShareServer.exe"
printf 'fake-cli-binary\n' >"$TMP/target/release/as-cmd.exe"
printf 'fake-license\n' >"$TMP/LICENSE"
printf 'fake-readme\n' >"$TMP/README.md"

# --- package ----------------------------------------------------------------
bash "$PACKAGER" \
    --root "$TMP" \
    --target-dir "$TMP/target/release" \
    --out-dir "$TMP/dist-windows" >/dev/null

GUI="$TMP/dist-windows/AudioShareServer.exe"
GUI_SUM="$TMP/dist-windows/AudioShareServer.exe.sha256"
ZIP="$TMP/dist-windows/audio-share-server-cmd-windows.zip"
ZIP_SUM="$TMP/dist-windows/audio-share-server-cmd-windows.zip.sha256"

assert_file "$GUI"
assert_file "$GUI_SUM"
assert_file "$ZIP"
assert_file "$ZIP_SUM"

[[ "$(cat "$GUI")" == "fake-gui-binary" ]] || fail "GUI binary content was not preserved"
GUI_SUM_CONTENT=$(cat "$GUI_SUM")
ZIP_SUM_CONTENT=$(cat "$ZIP_SUM")
[[ "$GUI_SUM_CONTENT" =~ ^[0-9a-f]{64}\ \ AudioShareServer\.exe$ ]] ||
    fail "unexpected GUI checksum: $GUI_SUM_CONTENT"
[[ "$ZIP_SUM_CONTENT" =~ ^[0-9a-f]{64}\ \ audio-share-server-cmd-windows\.zip$ ]] ||
    fail "unexpected archive checksum: $ZIP_SUM_CONTENT"
[[ "${GUI_SUM_CONTENT%% *}" == "$(sha256_hex "$TMP/target/release/AudioShareServer.exe")" ]] ||
    fail "GUI checksum does not match the input binary"
[[ "${ZIP_SUM_CONTENT%% *}" == "$(sha256_hex "$ZIP")" ]] ||
    fail "archive checksum does not match the archive"

ENTRIES=$(list_archive "$ZIP")
for entry in \
    audio-share-server-cmd/as-cmd.exe \
    audio-share-server-cmd/LICENSE \
    audio-share-server-cmd/README.md; do
    grep -qx "$entry" <<<"$ENTRIES" || fail "archive entry missing: $entry"
done

# The staged directory must not leak into the output directory.
[[ ! -e "$TMP/dist-windows/audio-share-server-cmd" ]] ||
    fail "staging directory was left in the output directory"

# Re-running must be idempotent (no duplicate zip entries / stale staging).
bash "$PACKAGER" \
    --root "$TMP" \
    --target-dir "$TMP/target/release" \
    --out-dir "$TMP/dist-windows" >/dev/null
[[ $(list_archive "$ZIP" | grep -cx 'audio-share-server-cmd/as-cmd.exe') -eq 1 ]] ||
    fail "re-running produced duplicate archive entries"

# A missing GUI binary must fail with a non-zero status and a clear message.
mkdir -p "$TMP/empty-target"
if bash "$PACKAGER" --root "$TMP" --target-dir "$TMP/empty-target" \
    --out-dir "$TMP/no-output" >"$TMP/error.log" 2>&1; then
    fail "packager accepted a target directory without AudioShareServer.exe"
fi
grep -q 'AudioShareServer.exe' "$TMP/error.log" ||
    fail "missing-binary error did not name AudioShareServer.exe"

echo "package-windows-artifacts self-test: PASS"
