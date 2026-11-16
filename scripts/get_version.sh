#!/usr/bin/env bash
# Read the repository version.
#
#   -n  version name, exactly as written in VERSION (e.g. 0.5.0-beta)
#   -c  numeric version code used by release tooling/changelog lookup
#
# Version-code scheme (stable, prerelease-aware):
#
#   code = MAJOR * 10_000_000 + MINOR * 100_000 + PATCH * 1_000 + PRERELEASE
#
#   PRERELEASE:
#     stable      999
#     alpha.N     100 + N
#     beta.N      200 + N
#     rc.N        300 + N
#     (N defaults to 0 and must be 0..99)
#
# This orders prereleases before the corresponding stable version and never
# collides with it:
#
#   0.5.0-alpha   500_100
#   0.5.0-beta    500_200
#   0.5.0-rc      500_300
#   0.5.0         500_999
#
# Bounds: MAJOR <= 214, MINOR <= 99, PATCH <= 99, so the result fits a signed
# 32-bit integer. This replaced the old `major*1_000_000 + minor*1_000 + patch`
# formula, under which `0.5.0-beta` and `0.5.0` both produced 5000.
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

usage() {
    echo "usage: $0 -n|-c" >&2
    exit 1
}

[[ $# -eq 1 ]] || usage

version=$(tr -d '[:space:]' < "$ROOT/VERSION")
[[ -n "$version" ]] || {
    echo "error: VERSION is empty" >&2
    exit 1
}

case "$1" in
-n)
    printf '%s\n' "$version"
    ;;
-c)
    core=${version%%-*}
    core=${core%%+*}
    IFS=. read -r major minor patch <<< "$core"
    for part in "$major" "$minor" "$patch"; do
        [[ "$part" =~ ^[0-9]+$ ]] || {
            echo "error: VERSION '$version' is not MAJOR.MINOR.PATCH[-prerelease]" >&2
            exit 1
        }
    done
    ((major <= 214)) || {
        echo "error: major version $major exceeds the version-code range (max 214)" >&2
        exit 1
    }
    ((minor <= 99 && patch <= 99)) || {
        echo "error: minor/patch must be 0..99 for the version-code scheme" >&2
        exit 1
    }

    prerelease=
    if [[ "$version" == *-* ]]; then
        prerelease=${version#*-}
        prerelease=${prerelease%%+*}
    fi

    if [[ -z "$prerelease" ]]; then
        pre_code=999
    else
        label=${prerelease%%.*}
        number=0
        if [[ "$prerelease" == *.* ]]; then
            number=${prerelease#*.}
        fi
        [[ "$number" =~ ^[0-9]+$ ]] || {
            echo "error: prerelease '$prerelease' has a non-numeric suffix" >&2
            exit 1
        }
        ((number <= 99)) || {
            echo "error: prerelease number $number must be 0..99" >&2
            exit 1
        }
        case "$label" in
        alpha) rank=100 ;;
        beta) rank=200 ;;
        rc) rank=300 ;;
        *)
            echo "error: unsupported prerelease label '$label' (use alpha, beta, or rc)" >&2
            exit 1
            ;;
        esac
        pre_code=$((rank + number))
    fi

    code=$((major * 10000000 + minor * 100000 + patch * 1000 + pre_code))
    ((code <= 2147483647)) || {
        echo "error: version code $code does not fit a signed 32-bit integer" >&2
        exit 1
    }
    printf '%s\n' "$code"
    ;;
*)
    usage
    ;;
esac
