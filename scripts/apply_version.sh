#!/usr/bin/env bash
# Apply the version in VERSION to the Rust workspace and Tauri GUI.
#
# Android is intentionally not touched: the v0.4.0 release is Server-only and
# the Android Client remains at its released v0.3.4 version/APK.
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
# usable after a version bump.
workspace_manifests=(crates/*/Cargo.toml tools/*/Cargo.toml gui/src-tauri/Cargo.toml)
workspace_packages=()
for manifest in "${workspace_manifests[@]}"; do
    package=$(sed -n 's/^name = "\(.*\)"$/\1/p' "$manifest" | head -n 1)
    [[ -n "$package" ]] && workspace_packages+=("$package")
done

cargo_lock_tmp=$(mktemp)
cp Cargo.lock "$cargo_lock_tmp"
for package in "${workspace_packages[@]}"; do
    awk -v pkg="$package" -v ver="$version_name" '
        $0 == "[[package]]" { in_package = 0 }
        $0 == "name = \"" pkg "\"" { in_package = 1 }
        in_package && $0 ~ /^version = "/ {
            sub(/^version = "[^"]*"/, "version = \"" ver "\"")
            in_package = 0
        }
        { print }
    ' "$cargo_lock_tmp" > "$cargo_lock_tmp.next"
    mv "$cargo_lock_tmp.next" "$cargo_lock_tmp"
done
cat "$cargo_lock_tmp" > Cargo.lock
rm -f "$cargo_lock_tmp"

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
done

popd >/dev/null
