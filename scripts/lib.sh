#!/usr/bin/env bash
# Shared helpers for the repository's release-packaging scripts.
#
# Source this file; do not execute it. `resolve_path` resolves relative paths
# against the caller's $ROOT, which must be set before the first call.

# Print an error and exit non-zero.
fail() {
    echo "error: $*" >&2
    exit 1
}

# Resolve $1 against $ROOT unless it is already absolute.
#
# Git Bash on Windows hands out paths such as `D:\a\repo` (the form
# `$GITHUB_WORKSPACE` takes), so native drive-letter and UNC paths must pass
# through untouched instead of being joined onto $ROOT.
resolve_path() {
    case "$1" in
    /* | //* | [A-Za-z]:*) printf '%s\n' "$1" ;;
    *) printf '%s\n' "$ROOT/$1" ;;
    esac
}

# Fail if the file does not exist.
require_file() {
    [[ -f "$1" ]] || fail "missing build input: $1"
}

# Fail if the file does not exist (self-test spelling and message).
assert_file() {
    [[ -f "$1" ]] || fail "missing file: $1"
}

# Print the SHA-256 of $1 using whichever tool is available.
sha256_hex() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    elif command -v openssl >/dev/null 2>&1; then
        openssl dgst -sha256 "$1" | awk '{print $NF}'
    else
        fail "no SHA-256 tool found (sha256sum, shasum, or openssl)"
    fi
}

# Write the `<hex>  <basename>` checksum sidecar next to $1.
write_sha256() {
    local file=$1
    printf '%s  %s\n' "$(sha256_hex "$file")" "$(basename "$file")" >"$file.sha256"
}
