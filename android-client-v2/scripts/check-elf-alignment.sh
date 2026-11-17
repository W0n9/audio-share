#!/usr/bin/env bash
# Assert 16 KB page-size compatibility for Android release artifacts.
#
# For every ELF shared object (*.so):
#   - every PT_LOAD segment must be aligned to at least 16384 bytes (2**14)
#     and to a power of two;
#   - the end of GNU_RELRO (VirtAddr + MemSiz) must be 0x4000-aligned, unless
#     there is no writable data after RELRO.
# For every APK (zip):
#   - `zipalign -c -P 16 4` must pass, i.e. uncompressed shared libraries are
#     stored 16 KB-aligned.
#
# Non-interactive: exits non-zero on the first failing file after checking all
# inputs, and prints a final `ALIGNED` line only when every check passed.
#
# Usage:
#   check-elf-alignment.sh [options] FILE...
#
#   FILE...                 .so and/or .apk paths (type detected by magic)
#   --ndk-dir DIR           NDK root (default: $ANDROID_NDK_HOME, else
#                           $ANDROID_HOME/ndk/27.2.12479018, else the newest
#                           NDK under $ANDROID_HOME/ndk)
#   --zipalign PATH         zipalign binary (default: build-tools/36.0.0/
#                           zipalign, else the newest build-tools, else PATH)
#
# Example (after a release build):
#   android-client-v2/scripts/check-elf-alignment.sh \
#     android-client-v2/app/build/generated/jniLibs/arm64-v8a/libaudio_share_client_v2_native.so \
#     android-client-v2/app/build/generated/jniLibs/x86_64/libaudio_share_client_v2_native.so \
#     android-client-v2/app/build/outputs/apk/release/app-release.apk

set -euo pipefail

PAGE_SIZE=0x4000
PAGE_SIZE_DEC=$((16#4000))

fail() {
    echo "error: $*" >&2
    exit 1
}

usage() {
    sed -n '2,40p' "$0" | sed 's/^# \{0,1\}//'
}

hex_to_dec() {
    local value=$1
    case "$value" in
    0x* | 0X*) printf '%d\n' "$((16#${value#0[xX]}))" ;;
    *) printf '%d\n' "$((10#$value))" ;;
    esac
}

default_ndk_dir() {
    if [[ -n ${ANDROID_NDK_HOME:-} ]]; then
        printf '%s\n' "$ANDROID_NDK_HOME"
        return
    fi
    if [[ -n ${ANDROID_HOME:-} ]]; then
        if [[ -d $ANDROID_HOME/ndk/27.2.12479018 ]]; then
            printf '%s\n' "$ANDROID_HOME/ndk/27.2.12479018"
            return
        fi
        local candidate
        candidate=$(find "$ANDROID_HOME/ndk" -mindepth 1 -maxdepth 1 -type d 2>/dev/null |
            sort -V | tail -1)
        [[ -n $candidate ]] && printf '%s\n' "$candidate"
        return
    fi
    if [[ -n ${ANDROID_NDK_ROOT:-} ]]; then
        printf '%s\n' "$ANDROID_NDK_ROOT"
    fi
}

default_zipalign() {
    if [[ -n ${ANDROID_HOME:-} ]]; then
        if [[ -x $ANDROID_HOME/build-tools/36.0.0/zipalign ]]; then
            printf '%s\n' "$ANDROID_HOME/build-tools/36.0.0/zipalign"
            return
        fi
        local candidate
        candidate=$(find "$ANDROID_HOME/build-tools" -mindepth 1 -maxdepth 1 -type d 2>/dev/null |
            sort -V | while read -r dir; do
                [[ -x $dir/zipalign ]] && printf '%s\n' "$dir/zipalign"
            done | tail -1)
        [[ -n $candidate ]] && printf '%s\n' "$candidate"
        return
    fi
    command -v zipalign 2>/dev/null || true
}

NDK_DIR=""
ZIPALIGN=""
FILES=()

while [[ $# -gt 0 ]]; do
    case "$1" in
    --ndk-dir)
        [[ $# -ge 2 ]] || fail "--ndk-dir needs a value"
        NDK_DIR=$2
        shift 2
        ;;
    --zipalign)
        [[ $# -ge 2 ]] || fail "--zipalign needs a value"
        ZIPALIGN=$2
        shift 2
        ;;
    -h | --help)
        usage
        exit 0
        ;;
    --)
        shift
        FILES+=("$@")
        break
        ;;
    -*)
        fail "unknown option: $1"
        ;;
    *)
        FILES+=("$1")
        shift
        ;;
    esac
done

[[ ${#FILES[@]} -gt 0 ]] || fail "no input files (use --help)"
[[ -n $NDK_DIR ]] || NDK_DIR=$(default_ndk_dir)
[[ -n $NDK_DIR ]] || fail "cannot locate the Android NDK; set ANDROID_HOME or pass --ndk-dir"

READELF="$NDK_DIR/toolchains/llvm/prebuilt/linux-x86_64/bin/llvm-readelf"
[[ -x $READELF ]] || fail "llvm-readelf not found at $READELF"

check_elf() {
    local file=$1
    local output line type off vaddr paddr filesz memsz rest
    local -a load_align=() load_flags=() load_end=()
    local load_count=0 writable_after_relro=0 relro_found=0 relro_end="" relro_vaddr="" relro_memsz=""
    local status=0

    output=$("$READELF" -Wl "$file") || fail "llvm-readelf failed on $file"
    while IFS= read -r line; do
        # LOAD and GNU_RELRO rows both place VirtAddr in $3 and MemSiz in $6;
        # the alignment is the last column (hex). LOAD flags may span two
        # columns ("R E"), so they are everything between $6 and the align.
        read -r type off vaddr paddr filesz memsz rest <<<"$line"
        case "$type" in
        LOAD)
            load_count=$((load_count + 1))
            load_align+=("$(awk '{print $NF}' <<<"$line")")
            load_flags+=("$(awk '{f = ""; for (i = 7; i < NF; i++) f = f $i; print f}' <<<"$line")")
            load_end+=("$((vaddr + memsz))")
            ;;
        GNU_RELRO)
            relro_found=1
            relro_vaddr=$vaddr
            relro_memsz=$memsz
            relro_end=$((vaddr + memsz))
            ;;
        esac
    done < <(grep -E '^[[:space:]]*(LOAD|GNU_RELRO)[[:space:]]' <<<"$output")

    ((load_count > 0)) || {
        echo "FAIL: $file has no LOAD segments (parse failure)"
        return 1
    }

    local i align_dec
    for ((i = 0; i < load_count; i++)); do
        align_dec=$(hex_to_dec "${load_align[$i]}")
        if ((align_dec < PAGE_SIZE_DEC || (align_dec & (align_dec - 1)) != 0)); then
            echo "FAIL: $file LOAD segment $((i + 1)) align ${load_align[$i]} (expected >= $PAGE_SIZE)"
            status=1
        else
            echo "PASS: $file LOAD segment $((i + 1)) align ${load_align[$i]}"
        fi
        if [[ -n $relro_end && ${load_flags[$i]} == *W* ]] && ((load_end[i] > relro_end)); then
            writable_after_relro=1
        fi
    done

    if ((relro_found == 0)); then
        if ((writable_after_relro == 1)); then
            echo "FAIL: $file has writable data but no GNU_RELRO"
            status=1
        else
            echo "PASS: $file has no GNU_RELRO and no writable LOAD segments"
        fi
    elif ((relro_end % PAGE_SIZE_DEC == 0)); then
        echo "PASS: $file GNU_RELRO end $relro_vaddr+$relro_memsz = $(printf '0x%x' "$relro_end") is $PAGE_SIZE-aligned"
    elif ((writable_after_relro == 0)); then
        echo "PASS: $file GNU_RELRO end $(printf '0x%x' "$relro_end") is not $PAGE_SIZE-aligned but there is no writable data after RELRO"
    else
        echo "FAIL: $file GNU_RELRO end $(printf '0x%x' "$relro_end") is not $PAGE_SIZE-aligned"
        status=1
    fi

    return "$status"
}

check_apk() {
    local file=$1
    [[ -x $ZIPALIGN ]] || fail "zipalign not found (looked for $ZIPALIGN); pass --zipalign"
    echo "== zipalign -c -P 16 4 $file"
    if "$ZIPALIGN" -c -P 16 4 "$file"; then
        echo "PASS: $file zipalign -c -P 16 4"
    else
        echo "FAIL: $file zipalign -c -P 16 4"
        return 1
    fi
}

[[ -n $ZIPALIGN ]] || ZIPALIGN=$(default_zipalign)

overall=0
for file in "${FILES[@]}"; do
    [[ -f $file ]] || fail "missing input file: $file"
    magic=$(head -c 4 "$file" | od -An -tx1 | tr -d ' \n')
    case "$magic" in
    7f454c46)
        echo "== ELF: $file"
        check_elf "$file" || overall=1
        ;;
    504b0304 | 504b0506 | 504b0708)
        echo "== APK: $file"
        check_apk "$file" || overall=1
        ;;
    *)
        fail "unrecognised file type (expected ELF or APK): $file"
        ;;
    esac
done

if ((overall == 0)); then
    echo "ALIGNED: ${#FILES[@]} file(s) are 16 KB page-size compatible"
else
    echo "NOT ALIGNED: at least one check failed" >&2
    exit 1
fi
