#!/usr/bin/env bash
# Hotplug recovery check for the Linux PipeWire capture backend.
#
# Creates a null `Audio/Sink`, captures a 440 Hz tone, destroys the sink while
# capturing, recreates it, and verifies the documented recovery model:
#
#   * the server process stays alive after its capture target disappears;
#   * it does NOT resume automatically (the rewrite has no capture supervisor:
#     the capture channel ends while the server keeps the old AudioFormat);
#   * operator Stop/Start (a fresh server process, or the GUI Start button)
#     with the recreated sink restores audio.
#
# Sleep/resume is not executed by this script: the host has no controllable
# suspend path (no systemd sleep target, no privilege to suspend the machine).
# See docs/server-rewrite/hotplug-sleep.md for the manual procedure.
#
# Usage:
#   scripts/hotplug-recovery.sh [--debug] [--skip-build] [--out DIR]
#
# Requires the same headless PipeWire tooling as acceptance-matrix.sh plus jq.

set -uo pipefail

SCRIPT_PATH=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")
ROOT=$(cd "$(dirname "$SCRIPT_PATH")/.." && pwd)

usage() {
    sed -n '2,21p' "$SCRIPT_PATH" | sed 's/^# \{0,1\}//'
}

inner_main() {
    local out=${HOTPLUG_OUT:?}
    local as_cmd=${HOTPLUG_AS_CMD:?}
    local harness=${HOTPLUG_HARNESS:?}
    local sink_name=${PW_TEST_SINK_NAME:?}
    local serial=${PW_TEST_ENDPOINT_SERIAL:?}
    local port server_pid server_exit=0
    local baseline=0 destroyed=0 recreated=0 operator=0 observed_no_auto=0
    local new_serial=""

    new_port() {
        python3 -I -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()'
    }

    sink_registry_id() {
        pw-dump 2>/dev/null | SINK_NAME="$sink_name" python3 -I -c '
import json, os, sys
wanted = os.environ["SINK_NAME"]
for obj in json.load(sys.stdin):
    props = obj.get("info", {}).get("props", {})
    if props.get("media.class") == "Audio/Sink" and props.get("node.name") == wanted:
        print(obj.get("id"))
        break
'
    }

    sink_serial() {
        pw-dump 2>/dev/null | SINK_NAME="$sink_name" python3 -I -c '
import json, os, sys
wanted = os.environ["SINK_NAME"]
for obj in json.load(sys.stdin):
    props = obj.get("info", {}).get("props", {})
    if props.get("media.class") == "Audio/Sink" and props.get("node.name") == wanted:
        print(props.get("object.serial", ""))
        break
'
    }

    wait_for_sink() {
        local deadline=$((SECONDS + 10))
        while [ "$SECONDS" -lt "$deadline" ]; do
            if [ -n "$(sink_serial)" ]; then
                return 0
            fi
            sleep 0.1
        done
        return 1
    }

    wait_for_sink_gone() {
        local deadline=$((SECONDS + 10))
        while [ "$SECONDS" -lt "$deadline" ]; do
            if [ -z "$(sink_serial)" ]; then
                return 0
            fi
            sleep 0.1
        done
        return 1
    }

    start_server() {
        local endpoint=$1 label=$2
        port=$(new_port)
        export AUDIO_SHARE_CONFIG="$out/config-$label.ini"
        "$as_cmd" --bind="127.0.0.1:$port" -e "$endpoint" \
            --encoding=f32 --channels=2 --sample-rate=48000 \
            >"$out/server-$label.out" 2>&1 &
        server_pid=$!
        local deadline=$((SECONDS + 15))
        while [ "$SECONDS" -lt "$deadline" ]; do
            if (echo >"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
                return 0
            fi
            kill -0 "$server_pid" 2>/dev/null || return 1
            sleep 0.05
        done
        return 1
    }

    stop_server() {
        if kill -0 "$server_pid" 2>/dev/null; then
            kill -INT "$server_pid" 2>/dev/null || true
        fi
        local deadline=$((SECONDS + 15))
        while kill -0 "$server_pid" 2>/dev/null && [ "$SECONDS" -lt "$deadline" ]; do
            sleep 0.05
        done
        if kill -0 "$server_pid" 2>/dev/null; then
            kill -KILL "$server_pid" 2>/dev/null || true
        fi
        wait "$server_pid" 2>/dev/null
        server_exit=$?
    }

    run_probe() {
        local label=$1 tone=$2 min=$3
        local json="$out/probe-$label.json"
        "$harness" probe --connect "127.0.0.1:$port" --duration-secs 2 \
            --tone-hz "$tone" --min-datagrams "$min" --label "$label" \
            --json-out "$json" >/dev/null 2>"$out/probe-$label.err"
        echo "$json"
    }

    echo "hotplug: sink=$sink_name serial=$serial" >&2

    # 1. Baseline: tone flows while the original sink exists.
    if start_server "$serial" baseline && run_probe baseline 440 1 >/dev/null \
        && jq -e '.ok == true and .datagrams > 0 and .tone.present == true' \
            "$out/probe-baseline.json" >/dev/null 2>&1; then
        baseline=1
        echo "pass baseline capture"
    else
        echo "fail baseline capture" >&2
    fi

    # 2. Destroy the capture target while the server is running.
    registry_id=$(sink_registry_id)
    if [ -n "$registry_id" ] && pw-cli destroy "$registry_id" >/dev/null 2>&1 \
        && wait_for_sink_gone; then
        destroyed=1
        echo "pass sink destroyed (registry id $registry_id)"
    else
        echo "fail sink destroy" >&2
    fi

    # 3. The server must stay alive but stop producing PCM.
    if [ "$destroyed" -eq 1 ] && kill -0 "$server_pid" 2>/dev/null; then
        sleep 1
        run_probe after-destroy 0 0 >/dev/null
        if jq -e '.ok == true and .datagrams == 0' "$out/probe-after-destroy.json" >/dev/null 2>&1; then
            echo "pass server stayed alive, no PCM after target loss"
        else
            echo "fail server behavior after target loss" >&2
        fi
    else
        echo "fail server died or sink was not destroyed" >&2
    fi

    # 4. Recreate a sink with the same name/priority; the old server still
    #    targets the old object.serial, so it must not recover by itself.
    pw-cli create-node adapter \
        "{ factory.name=support.null-audio-sink node.name=${sink_name} node.description=${sink_name} media.class=Audio/Sink object.linger=1 audio.position=[FL,FR] priority.session=1000 }" \
        >/dev/null 2>&1
    if wait_for_sink; then
        new_serial=$(sink_serial)
        recreated=1
        echo "pass sink recreated (new serial $new_serial)"
    else
        echo "fail sink recreate" >&2
    fi
    if [ "$recreated" -eq 1 ] && kill -0 "$server_pid" 2>/dev/null; then
        sleep 1
        run_probe after-recreate 0 0 >/dev/null
        if jq -e '.ok == true and .datagrams == 0' "$out/probe-after-recreate.json" >/dev/null 2>&1; then
            observed_no_auto=1
            echo "pass no automatic recovery (documented: no capture supervisor)"
        else
            echo "fail unexpected automatic recovery or probe failure" >&2
        fi
    fi
    stop_server
    if [ "$server_exit" -ne 0 ]; then
        echo "fail server exit after target loss was $server_exit" >&2
    fi

    # 5. Operator Stop/Start: start a fresh server on the recreated sink.
    if [ "$recreated" -eq 1 ] && [ -n "$new_serial" ] \
        && start_server "$new_serial" operator && run_probe operator 440 1 >/dev/null \
        && jq -e '.ok == true and .datagrams > 0 and .tone.present == true' \
            "$out/probe-operator.json" >/dev/null 2>&1; then
        operator=1
        echo "pass operator Stop/Start recovered capture"
    else
        echo "fail operator Stop/Start recovery" >&2
    fi
    stop_server

    local pass=1
    if [ "$baseline" -ne 1 ] || [ "$destroyed" -ne 1 ] || [ "$recreated" -ne 1 ] \
        || [ "$observed_no_auto" -ne 1 ] || [ "$operator" -ne 1 ] \
        || [ "$server_exit" -ne 0 ]; then
        pass=0
    fi
    python3 -I - "$out" "$pass" "$baseline" "$destroyed" "$recreated" \
        "$observed_no_auto" "$operator" "$serial" "$new_serial" "$server_exit" <<'PY'
import json
import sys

out, passed, baseline, destroyed, recreated, no_auto, operator, old_serial, new_serial, exit_status = sys.argv[1:]
summary = {
    "pass": passed == "1",
    "baseline_tone": baseline == "1",
    "sink_destroyed": destroyed == "1",
    "sink_recreated": recreated == "1",
    "no_automatic_capture_recovery": no_auto == "1",
    "operator_stop_start_recovered": operator == "1",
    "original_serial": old_serial,
    "new_serial": new_serial,
    "server_exit_after_target_loss": int(exit_status),
    "automatic_recovery_implemented": False,
    "sleep_resume_executed": False,
}
with open(out + "/hotplug-summary.json", "w") as handle:
    json.dump(summary, handle, indent=2)
    handle.write("\n")
print("hotplug: " + ("PASS" if summary["pass"] else "FAIL"))
PY
    local status=$?
    echo "hotplug: results in $out"
    [ "$pass" -eq 1 ] && [ "$status" -eq 0 ] && return 0
    return 1
}

# --------------------------------------------------------------- outer mode
out=""
debug=0
skip_build=0
while [ $# -gt 0 ]; do
    case "$1" in
        --inner)
            inner_main
            exit $?
            ;;
        --out)
            out=${2:?--out needs a directory}
            shift 2
            ;;
        --debug)
            debug=1
            shift
            ;;
        --skip-build)
            skip_build=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

if [ -z "$out" ]; then
    out="/tmp/audio-share-hotplug/$(date -u +%Y%m%dT%H%M%SZ)"
fi
mkdir -p "$out"

if [ "$debug" -eq 1 ]; then
    profile=debug
else
    profile=release
fi
as_cmd="$ROOT/target/$profile/as-cmd"
harness="$ROOT/target/$profile/protocol-harness"
if [ "$skip_build" -eq 0 ]; then
    echo "building $profile binaries..." >&2
    if [ "$profile" = release ]; then
        cargo build --manifest-path "$ROOT/Cargo.toml" --release -p as-cmd -p protocol-harness >&2
    else
        cargo build --manifest-path "$ROOT/Cargo.toml" -p as-cmd -p protocol-harness >&2
    fi
fi
if [ ! -x "$as_cmd" ] || [ ! -x "$harness" ]; then
    echo "missing binaries: $as_cmd and/or $harness (run without --skip-build)" >&2
    exit 2
fi

tone="$out/tone-48000.wav"
python3 "$ROOT/scripts/gen-tone-wav.py" "$tone" --frequency 440 \
    --sample-rate 48000 --channels 2 --duration 30 --amplitude 0.25 >/dev/null

export HOTPLUG_OUT="$out" HOTPLUG_AS_CMD="$as_cmd" HOTPLUG_HARNESS="$harness"
"$ROOT/scripts/pw-test-session.sh" --tone-wav "$tone" -- "$SCRIPT_PATH" --inner
status=$?
if [ -f "$out/hotplug-summary.json" ]; then
    echo
    jq . "$out/hotplug-summary.json"
fi
exit "$status"
