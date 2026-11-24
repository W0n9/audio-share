#!/usr/bin/env bash
# Apply the version in VERSION to the Rust workspace and Tauri GUI.
#
# Android is intentionally not touched: its version (currently 0.6.3 /
# versionCode 6) is owned by android-client-v2/app/build.gradle.kts so the v2
# APK can coexist with the released v1 app.
#
# Prerelease versions such as `0.5.0-beta` are supported. The numeric version
# code comes from scripts/get_version.sh -c, whose prerelease-aware scheme
# keeps `0.5.0-beta` distinct from `0.5.0` (500200 vs 500999).
#
# Updated files:
#   Cargo.toml                              [workspace.package] version
#   Cargo.lock                              workspace package entries
#   gui/package.json                        npm package version
#   gui/package-lock.json                   npm lockfile root versions
#   gui/src-tauri/Cargo.toml                GUI crate version
#   gui/src-tauri/tauri.conf.json           Tauri bundle version
#
# The legacy CMake/C++ version files were removed with the old Server
# implementation in the 0.4.0 cutover.
set -euo pipefail

pushd "$(dirname "$0")/.." >/dev/null

version_name=$(bash ./scripts/get_version.sh -n)
version_code=$(bash ./scripts/get_version.sh -c)

echo VERSION: version_name="$version_name" version_code="$version_code"

# Cargo workspace version; every workspace crate inherits it.
sed -Ebi "s|^version = \"[^\"]*\"|version = \"$version_name\"|" Cargo.toml

# Cargo.lock records each workspace package version explicitly. Update only
# those entries (third-party pins stay untouched) so `cargo --locked` remains
# usable after a version bump. Read the member list from the root manifest:
# tools/rust-android-interop is a separate workspace, so a tools/* glob would
# incorrectly demand an entry for it in this workspace's Cargo.lock.
workspace_manifests=()
while IFS= read -r member; do
    workspace_manifests+=("$member/Cargo.toml")
done < <(
    awk '/^members = \[/,/^\]/' Cargo.toml |
        sed -n 's/^[[:space:]]*"\([^"]*\)",\{0,1\}$/\1/p'
)
[[ ${#workspace_manifests[@]} -gt 0 ]] || {
    echo "error: could not read workspace members from Cargo.toml" >&2
    exit 1
}
workspace_packages=()
for manifest in "${workspace_manifests[@]}"; do
    package=$(sed -n 's/^name = "\(.*\)"$/\1/p' "$manifest" | head -n 1)
    [[ -n "$package" ]] && workspace_packages+=("$package")
done

update_lock_versions() {
    local lock=$1
    local tmp package
    tmp=$(mktemp)
    cp "$lock" "$tmp"
    for package in "${workspace_packages[@]}"; do
        awk -v pkg="$package" -v ver="$version_name" '
            $0 == "[[package]]" { in_package = 0 }
            $0 == "name = \"" pkg "\"" { in_package = 1 }
            in_package && $0 ~ /^version = "/ {
                sub(/^version = "[^"]*"/, "version = \"" ver "\"")
                in_package = 0
            }
            { print }
        ' "$tmp" > "$tmp.next"
        mv "$tmp.next" "$tmp"
    done
    cat "$tmp" > "$lock"
    rm -f "$tmp"
}

update_lock_versions Cargo.lock

# The cross-language interop harness is a separate workspace, but its lockfile
# path-depends on these workspace crates. Keep those entries in sync so
# `cargo --locked` still works there; packages it does not contain are skipped.
interop_lock=tools/rust-android-interop/Cargo.lock
if [[ -f "$interop_lock" ]]; then
    update_lock_versions "$interop_lock"
fi

# GUI package metadata and lockfile.
sed -Ebi "0,/^  \"version\": \"[^\"]*\"/s//  \"version\": \"$version_name\"/" gui/package.json
sed -Ebi "0,/^  \"version\": \"[^\"]*\"/s//  \"version\": \"$version_name\"/" gui/package-lock.json
sed -Ebi "0,/^      \"version\": \"[^\"]*\"/s//      \"version\": \"$version_name\"/" gui/package-lock.json

# GUI crate manifest and Tauri bundle config.
sed -Ebi "0,/^version = \"[^\"]*\"/s//version = \"$version_name\"/" gui/src-tauri/Cargo.toml
sed -Ebi "0,/^  \"version\": \"[^\"]*\"/s//  \"version\": \"$version_name\"/" gui/src-tauri/tauri.conf.json

# Fail loudly if any expected location still disagrees with VERSION.
check_version() {
    local file=$1 pattern=$2
    grep -Eq "$pattern" "$file" || {
        echo "error: $file does not contain version $version_name" >&2
        exit 1
    }
}

check_version Cargo.toml "^version = \"$version_name\"$"
check_version gui/package.json "^  \"version\": \"$version_name\""
check_version gui/package-lock.json "^      \"version\": \"$version_name\""
check_version gui/src-tauri/Cargo.toml "^version = \"$version_name\"$"
check_version gui/src-tauri/tauri.conf.json "^  \"version\": \"$version_name\""
for package in "${workspace_packages[@]}"; do
    grep -A1 "^name = \"$package\"$" Cargo.lock | grep -q "^version = \"$version_name\"$" || {
        echo "error: Cargo.lock entry for $package does not contain version $version_name" >&2
        exit 1
    }
    if [[ -f "$interop_lock" ]] && grep -q "^name = \"$package\"$" "$interop_lock"; then
        grep -A1 "^name = \"$package\"$" "$interop_lock" | grep -q "^version = \"$version_name\"$" || {
            echo "error: $interop_lock entry for $package does not contain version $version_name" >&2
            exit 1
        }
    fi
done

popd >/dev/null
