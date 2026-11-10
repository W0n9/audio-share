#!/usr/bin/env bash
# Self-test for package-linux-artifacts.sh.
#
# Builds a fake Cargo/bundle tree, runs the packager, and asserts the exact
# release artifact names, the v0.3.4 CLI archive layout, and checksum files.
# Runs on the Linux host and on the ubuntu-24.04 CI runner.
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=lib.sh
source "$SCRIPT_DIR/lib.sh"
PACKAGER="$SCRIPT_DIR/package-linux-artifacts.sh"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

list_archive() {
    tar -tzf "$1" | sed 's:/$::'
}

# --- fake repository, bundle, and build outputs -----------------------------
ROOT="$TMP/root"
mkdir -p "$ROOT/gui/src-tauri" "$ROOT/target/release" \
    "$TMP/bundle/deb" "$TMP/bundle/appimage"
cat >"$ROOT/gui/src-tauri/tauri.conf.json" <<'EOF'
{
  "productName": "AudioShareServer",
  "version": "9.9.9"
}
EOF
printf 'fake-license\n' >"$ROOT/LICENSE"
printf 'fake-readme\n' >"$ROOT/README.md"
printf 'fake-cli-binary\n' >"$ROOT/target/release/as-cmd"
chmod +x "$ROOT/target/release/as-cmd"
printf 'fake-deb\n' >"$TMP/bundle/deb/AudioShareServer_9.9.9_amd64.deb"
printf 'fake-appimage\n' >"$TMP/bundle/appimage/AudioShareServer_9.9.9_amd64.AppImage"

# --- package ----------------------------------------------------------------
bash "$PACKAGER" \
    --root "$ROOT" \
    --bundle-dir "$TMP/bundle" \
    --target-dir "$ROOT/target/release" \
    --out-dir "$TMP/dist-linux" \
    --arch amd64 >/dev/null

DEB="$TMP/dist-linux/AudioShareServer_9.9.9_amd64.deb"
DEB_SUM="$TMP/dist-linux/AudioShareServer_9.9.9_amd64.deb.sha256"
APPIMAGE="$TMP/dist-linux/AudioShareServer_9.9.9_amd64.AppImage"
APPIMAGE_SUM="$TMP/dist-linux/AudioShareServer_9.9.9_amd64.AppImage.sha256"
CLI="$TMP/dist-linux/audio-share-server-cmd-linux.tar.gz"
CLI_SUM="$TMP/dist-linux/audio-share-server-cmd-linux.tar.gz.sha256"

for file in "$DEB" "$DEB_SUM" "$APPIMAGE" "$APPIMAGE_SUM" "$CLI" "$CLI_SUM"; do
    assert_file "$file"
done

[[ "$(cat "$DEB")" == "fake-deb" ]] || fail "deb content was not preserved"
[[ "$(cat "$APPIMAGE")" == "fake-appimage" ]] || fail "AppImage content was not preserved"

DEB_SUM_CONTENT=$(cat "$DEB_SUM")
APPIMAGE_SUM_CONTENT=$(cat "$APPIMAGE_SUM")
CLI_SUM_CONTENT=$(cat "$CLI_SUM")
[[ "$DEB_SUM_CONTENT" =~ ^[0-9a-f]{64}\ \ AudioShareServer_9\.9\.9_amd64\.deb$ ]] ||
    fail "unexpected deb checksum: $DEB_SUM_CONTENT"
[[ "$APPIMAGE_SUM_CONTENT" =~ ^[0-9a-f]{64}\ \ AudioShareServer_9\.9\.9_amd64\.AppImage$ ]] ||
    fail "unexpected AppImage checksum: $APPIMAGE_SUM_CONTENT"
[[ "$CLI_SUM_CONTENT" =~ ^[0-9a-f]{64}\ \ audio-share-server-cmd-linux\.tar\.gz$ ]] ||
    fail "unexpected CLI checksum: $CLI_SUM_CONTENT"
[[ "${DEB_SUM_CONTENT%% *}" == "$(sha256_hex "$TMP/bundle/deb/AudioShareServer_9.9.9_amd64.deb")" ]] ||
    fail "deb checksum does not match the input bundle"
[[ "${APPIMAGE_SUM_CONTENT%% *}" == "$(sha256_hex "$TMP/bundle/appimage/AudioShareServer_9.9.9_amd64.AppImage")" ]] ||
    fail "AppImage checksum does not match the input bundle"
[[ "${CLI_SUM_CONTENT%% *}" == "$(sha256_hex "$CLI")" ]] ||
    fail "CLI checksum does not match the archive"

# The CLI archive keeps the v0.3.4 layout: one top-level directory with the
# Linux `as-cmd`, LICENSE, and README.md.
ENTRIES=$(list_archive "$CLI")
for entry in \
    audio-share-server-cmd/as-cmd \
    audio-share-server-cmd/LICENSE \
    audio-share-server-cmd/README.md; do
    grep -qx "$entry" <<<"$ENTRIES" || fail "archive entry missing: $entry"
done
[[ "$(grep -c '^audio-share-server-cmd/as-cmd$' <<<"$ENTRIES")" -eq 1 ]] ||
    fail "archive has a duplicated as-cmd entry"

# The staged directory must not leak into the output directory.
[[ ! -e "$TMP/dist-linux/audio-share-server-cmd" ]] ||
    fail "staging directory was left in the output directory"

# Re-running must be idempotent (no duplicate archive entries / stale staging).
bash "$PACKAGER" \
    --root "$ROOT" \
    --bundle-dir "$TMP/bundle" \
    --target-dir "$ROOT/target/release" \
    --out-dir "$TMP/dist-linux" \
    --arch amd64 >/dev/null
[[ "$(list_archive "$CLI" | grep -cx 'audio-share-server-cmd/as-cmd')" -eq 1 ]] ||
    fail "re-running produced duplicate archive entries"

# A missing deb must fail with a non-zero status and name the expected artifact.
mkdir -p "$TMP/empty-bundle/deb" "$TMP/empty-bundle/appimage"
cp "$TMP/bundle/appimage/AudioShareServer_9.9.9_amd64.AppImage" \
    "$TMP/empty-bundle/appimage/"
if bash "$PACKAGER" --root "$ROOT" --bundle-dir "$TMP/empty-bundle" \
    --target-dir "$ROOT/target/release" --out-dir "$TMP/no-output" \
    --arch amd64 >"$TMP/error.log" 2>&1; then
    fail "packager accepted a bundle directory without the deb"
fi
grep -q 'AudioShareServer_9.9.9_amd64.deb' "$TMP/error.log" ||
    fail "missing-deb error did not name the expected deb"

echo "package-linux-artifacts self-test: PASS"
