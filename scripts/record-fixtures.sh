#!/usr/bin/env bash
# Re-record the real Linux v0.3.4 fixture sets from the released `as-cmd`.
#
# Usage:
#   AS_CMD_034=/path/to/as-cmd-0.3.4-linux scripts/record-fixtures.sh
#
# The script starts a disposable headless PipeWire session with a null sink,
# loops a generated sine tone into it, and runs protocol-harness `record`
# against the old binary. It writes:
#   fixtures/protocol/v0.3.4/linux/f32-48k-stereo
#   fixtures/protocol/v0.3.4/linux/s16-44k1-stereo
#
# No sudo, no host PipeWire session, and no absolute paths inside fixtures.

set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
as_cmd=${AS_CMD_034:-}

if [ -z "$as_cmd" ]; then
    echo "Set AS_CMD_034 to the released v0.3.4 Linux as-cmd binary." >&2
    echo "Example: AS_CMD_034=\"\$HOME/bin/as-cmd-0.3.4-linux\" $0" >&2
    exit 2
fi
if [ ! -x "$as_cmd" ]; then
    echo "as-cmd is not executable: $as_cmd" >&2
    exit 2
fi

cargo build -p protocol-harness --manifest-path "$root/Cargo.toml"
harness="$root/target/debug/protocol-harness"
fixtures="$root/fixtures/protocol/v0.3.4/linux"

tmp=$(mktemp -d "${TMPDIR:-/tmp}/audio-share-record.XXXXXX")
cleanup() {
    rm -rf "$tmp"
}
trap cleanup EXIT INT TERM

python3 -I "$root/scripts/gen-tone-wav.py" --duration 60 "$tmp/tone.wav" >/dev/null

AS_CMD="$as_cmd" HARNESS="$harness" FIXTURES="$fixtures" \
    "$root/scripts/pw-test-session.sh" --tone-wav "$tmp/tone.wav" -- \
    bash -c '
        set -euo pipefail

        record_case() {
            local out=$1
            shift
            echo "recording $out"
            "$HARNESS" record \
                --server-cmd "$AS_CMD" \
                --server-arg=--bind=127.0.0.1:{port} \
                --server-arg=-e \
                --server-arg="$PW_TEST_ENDPOINT_SERIAL" \
                "$@" \
                --out "$out" \
                --provenance recorded-v0.3.4-linux \
                --platform linux \
                --server-label "as-cmd 0.3.4 (linux, headless PipeWire null sink)" \
                --heartbeats 3 \
                --datagrams 32 \
                --timeout-secs 25
        }

        record_case "$FIXTURES/f32-48k-stereo" \
            --server-arg=--encoding=f32 \
            --server-arg=--channels=2 \
            --server-arg=--sample-rate=48000
        record_case "$FIXTURES/s16-44k1-stereo" \
            --server-arg=--encoding=s16 \
            --server-arg=--channels=2 \
            --server-arg=--sample-rate=44100
    '

echo "recorded Linux fixtures under $fixtures"
