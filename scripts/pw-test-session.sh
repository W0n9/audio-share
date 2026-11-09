#!/usr/bin/env bash
# Run a command inside a disposable headless PipeWire session with a null sink.
#
# Usage:
#   scripts/pw-test-session.sh [options] -- command [args...]
#
# Options:
#   --tone-wav FILE     Loop-play FILE into the sink (use scripts/gen-tone-wav.py)
#   --priority N        priority.session of the null sink (default 1000)
#   --sink-name NAME    node.name of the null sink (default $PW_TEST_SINK or TestSink)
#   -h, --help          Show this help
#
# The session starts a private pipewire + wireplumber instance (private
# XDG_RUNTIME_DIR, dbus-run-session), creates an Audio/Sink, and exports:
#   PW_TEST_ENDPOINT_SERIAL  object.serial of the sink (as-cmd -e / --endpoint value)
#   PW_TEST_SINK_NAME        sink node name
#
# Requires: pipewire, wireplumber, pw-cli, pw-dump, dbus-run-session. When
# --tone-wav is used, pw-play must be installed.
#
# Everything is cleaned up on exit, including on error. No sudo or user-wide
# services are touched.

set -euo pipefail

usage() {
    sed -n '2,23p' "$0" | sed 's/^# \{0,1\}//'
}

tone_wav=""
priority=1000
sink_name="${PW_TEST_SINK:-TestSink}"
while [ $# -gt 0 ]; do
    case "$1" in
        --tone-wav)
            tone_wav=${2:?--tone-wav needs a file}
            shift 2
            ;;
        --priority)
            priority=${2:?--priority needs a number}
            shift 2
            ;;
        --sink-name)
            sink_name=${2:?--sink-name needs a name}
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        --)
            shift
            break
            ;;
        *)
            break
            ;;
    esac
done

if [ $# -eq 0 ]; then
    usage >&2
    exit 2
fi
if [ -n "$tone_wav" ] && [ ! -f "$tone_wav" ]; then
    echo "tone WAV not found: $tone_wav" >&2
    exit 2
fi

tmp=$(mktemp -d "${TMPDIR:-/tmp}/audio-share-pw.XXXXXX")
cleanup() {
    rm -rf "$tmp"
}
trap cleanup EXIT INT TERM

runtime="$tmp/runtime"
mkdir -p "$runtime"

cat > "$tmp/find-sink.py" <<'PY'
import json
import os
import sys

wanted = os.environ.get("PW_TEST_SINK", "TestSink")
for obj in json.load(sys.stdin):
    props = obj.get("info", {}).get("props", {})
    if (
        obj.get("type", "").endswith("Node")
        and props.get("media.class") == "Audio/Sink"
        and props.get("node.name") == wanted
    ):
        print(props.get("object.serial", ""))
        break
PY

cat > "$tmp/session.sh" <<'SESSION'
#!/usr/bin/env bash
set -euo pipefail

export XDG_RUNTIME_DIR="$PW_TEST_RUNTIME_DIR"
export PIPEWIRE_RUNTIME_DIR="$PW_TEST_RUNTIME_DIR"

pipewire >"$PW_TEST_TMP/pipewire.log" 2>&1 &
pw_pid=$!
wp_pid=""
tone_pid=""

cleanup_session() {
    if [ -n "$tone_pid" ]; then
        kill "$tone_pid" 2>/dev/null || true
    fi
    if [ -n "$wp_pid" ]; then
        kill "$wp_pid" 2>/dev/null || true
    fi
    if [ -n "$pw_pid" ]; then
        kill "$pw_pid" 2>/dev/null || true
    fi
    wait 2>/dev/null || true
}
trap cleanup_session EXIT INT TERM

for _ in $(seq 1 100); do
    [ -S "$XDG_RUNTIME_DIR/pipewire-0" ] && break
    sleep 0.05
done
if [ ! -S "$XDG_RUNTIME_DIR/pipewire-0" ]; then
    echo "pw-test-session: pipewire did not create its socket" >&2
    tail -20 "$PW_TEST_TMP/pipewire.log" >&2 || true
    exit 1
fi

wireplumber >"$PW_TEST_TMP/wireplumber.log" 2>&1 &
wp_pid=$!
for _ in $(seq 1 100); do
    pw-cli info 0 >/dev/null 2>&1 && break
    sleep 0.05
done

# audio.position must be bracketed SPA-JSON: FL,FR would be parsed as a key.
pw-cli create-node adapter \
    "{ factory.name=support.null-audio-sink node.name=${PW_TEST_SINK} node.description=${PW_TEST_SINK} media.class=Audio/Sink object.linger=1 audio.position=[FL,FR] priority.session=${PW_TEST_PRIORITY} }" \
    >"$PW_TEST_TMP/pw-cli.log" 2>&1

sink_serial=""
for _ in $(seq 1 100); do
    sink_serial=$(pw-dump 2>/dev/null | python3 -I "$PW_TEST_TMP/find-sink.py" || true)
    if [ -n "$sink_serial" ]; then
        break
    fi
    sleep 0.1
done
if [ -z "$sink_serial" ]; then
    echo "pw-test-session: null sink ${PW_TEST_SINK} did not appear" >&2
    pw-cli ls Node >&2 || true
    exit 1
fi
export PW_TEST_ENDPOINT_SERIAL="$sink_serial"
export PW_TEST_SINK_NAME="$PW_TEST_SINK"

if [ -n "$PW_TEST_TONE_WAV" ]; then
    (
        while :; do
            pw-play --target "$PW_TEST_SINK" "$PW_TEST_TONE_WAV" >/dev/null 2>&1 || sleep 0.2
        done
    ) &
    tone_pid=$!
    # Let the playback stream link before the command starts.
    sleep 0.5
fi

"$@"
SESSION
chmod +x "$tmp/session.sh"

export PW_TEST_RUNTIME_DIR="$runtime"
export PW_TEST_TMP="$tmp"
export PW_TEST_SINK="$sink_name"
export PW_TEST_PRIORITY="$priority"
export PW_TEST_TONE_WAV="$tone_wav"

dbus-run-session -- bash "$tmp/session.sh" "$@"
