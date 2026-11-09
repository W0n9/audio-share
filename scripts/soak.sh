#!/usr/bin/env bash
# Short/long soak for the new Server over the real Wire Protocol.
#
# Runs the release `as-cmd` with the real PipeWire capture backend inside a
# disposable headless PipeWire + WirePlumber session with a null `Audio/Sink`
# and a looped 440 Hz tone, plus N scripted clients. Samples server/client
# RSS and CPU, aggregates heartbeat misses, reconnects, PCM throughput,
# datagram-gap proxies for glitches, and the process exit status.
#
# Usage:
#   scripts/soak.sh [options]
#
# Options:
#   --duration SPEC   Soak duration: 8h (default), 30m, 90s, or plain seconds.
#   --clients N       Number of scripted clients (default 2).
#   --reconnect-every SECS
#                     Each client reconnects after SECS of streaming
#                     (default 1800; 0 disables reconnects).
#   --sample-seconds N
#                     RSS/CPU sampling interval (default 5).
#   --rotate-mb N     Log rotation threshold in MiB (default 16).
#   --out DIR         Results directory (default: /tmp/audio-share-soak/<UTC>).
#   --debug           Use target/debug binaries (not for acceptance claims).
#   --skip-build      Do not run cargo build --release first.
#   -h, --help        Show this help.
#
# Results (under --out):
#   soak-summary.json  machine-readable verdict and counters
#   monitor.csv        RSS/CPU samples
#   server-stream.log* rotated server stdout/stderr
#   server.log*        the as-cmd log file (rotation by rename; see docs)
#   client-N.json      per-client probe report
#   client-N.out/.err  per-client stdout/stderr
#
# The script exits nonzero on a server crash, a non-zero server exit, any
# client probe failure, or any heartbeat miss. A gap between PCM datagrams is
# recorded as a glitch proxy (50/100/250 ms buckets), not hidden.
#
# Requires: cargo, pipewire, wireplumber, pw-cli, pw-play, pw-dump,
# dbus-run-session, python3, jq. No sudo and no user services are touched.

set -uo pipefail

SCRIPT_PATH=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")
ROOT=$(cd "$(dirname "$SCRIPT_PATH")/.." && pwd)

usage() {
    sed -n '2,41p' "$SCRIPT_PATH" | sed 's/^# \{0,1\}//'
}

parse_duration() {
    local spec=$1
    case "$spec" in
        *h) echo $(( ${spec%h} * 3600 )) ;;
        *m) echo $(( ${spec%m} * 60 )) ;;
        *s) echo "${spec%s}" ;;
        *[!0-9]*) echo "invalid duration: $spec" >&2; return 1 ;;
        *) echo "$spec" ;;
    esac
}

# ---------------------------------------------------------------- inner mode
inner_main() {
    local out=${SOAK_OUT:?}
    local duration=${SOAK_DURATION:?}
    local clients=${SOAK_CLIENTS:?}
    local reconnect_every=${SOAK_RECONNECT_EVERY:-1800}
    local sample_seconds=${SOAK_SAMPLE_SECONDS:-5}
    local rotate_bytes=${SOAK_ROTATE_BYTES:-16777216}
    local as_cmd=${SOAK_AS_CMD:?}
    local harness=${SOAK_HARNESS:?}
    local serial=${PW_TEST_ENDPOINT_SERIAL:?}
    local port server_pid server_status="" server_crashed=0
    local server_log="$out/server.log"
    local stream_log="$out/server-stream.log"
    local monitor="$out/monitor.csv"
    local client_pids=()
    local start_epoch had_client_failure=0

    new_port() {
        python3 -I -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()'
    }

    # Consumer for a process-substitution pipe: append lines and rotate.
    rotate_stream() {
        local file=$1 limit=$2 size=0 line rotated
        while IFS= read -r line; do
            printf '%s\n' "$line" >>"$file"
            size=$((size + ${#line} + 1))
            if [ "$size" -ge "$limit" ]; then
                rotated="$file.$(date -u +%Y%m%dT%H%M%SZ).rotated"
                mv "$file" "$rotated" 2>/dev/null || true
                size=0
            fi
        done
    }

    rotate_if_large() {
        local file=$1 limit=$2
        [ -f "$file" ] || return 0
        local size
        size=$(stat -c%s "$file" 2>/dev/null || echo 0)
        if [ "$size" -ge "$limit" ]; then
            mv "$file" "$file.$(date -u +%Y%m%dT%H%M%SZ).rotated" 2>/dev/null || true
        fi
    }

    wait_tcp() {
        local deadline=$((SECONDS + 20))
        while [ "$SECONDS" -lt "$deadline" ]; do
            if (echo >"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
                return 0
            fi
            kill -0 "$server_pid" 2>/dev/null || return 1
            sleep 0.05
        done
        return 1
    }

    pid_alive() {
        kill -0 "$1" 2>/dev/null
    }

    stop_pid() {
        local pid=$1 deadline=$((SECONDS + 15))
        pid_alive "$pid" || return 0
        kill -INT "$pid" 2>/dev/null || true
        while pid_alive "$pid" && [ "$SECONDS" -lt "$deadline" ]; do
            sleep 0.1
        done
        if pid_alive "$pid"; then
            kill -KILL "$pid" 2>/dev/null || true
        fi
        wait "$pid" 2>/dev/null
    }

    export AUDIO_SHARE_CONFIG="$out/config.ini"
    port=$(new_port)
    start_epoch=$(date +%s)
    echo "soak: server=$as_cmd clients=$clients duration=${duration}s port=$port" >&2

    "$as_cmd" --bind="127.0.0.1:$port" -e "$serial" \
        --encoding=f32 --channels=2 --sample-rate=48000 \
        > >(rotate_stream "$stream_log" "$rotate_bytes") 2>&1 &
    server_pid=$!
    if ! wait_tcp; then
        wait "$server_pid" 2>/dev/null
        server_status=$?
        echo "soak: server did not start (exit $server_status); see $stream_log" >&2
        printf '{"pass":false,"server_crashed":true,"server_exit":%s,"error":"server did not start"}\n' \
            "$server_status" >"$out/soak-summary.json"
        return 1
    fi

    printf 'timestamp,elapsed_s,server_rss_kb,server_cpu_pct,client_rss_kb,server_alive,clients_alive\n' \
        >"$monitor"

    local index=0
    while [ "$index" -lt "$clients" ]; do
        "$harness" probe --connect "127.0.0.1:$port" --duration-secs "$duration" \
            --timeout-secs $((duration + 30)) \
            --reconnect-every-secs "$reconnect_every" --reconnect-delay-ms 3000 \
            --heartbeat-interval-ms 3000 --tone-hz 440 --label "soak-$index" \
            --json-out "$out/client-$index.json" \
            >"$out/client-$index.out" 2>"$out/client-$index.err" &
        client_pids+=("$!")
        index=$((index + 1))
    done

    # Monitor until the duration elapses, the server dies, or a client dies.
    while :; do
        local now elapsed server_rss server_cpu client_rss=0 alive=0
        now=$(date +%s)
        elapsed=$((now - start_epoch))
        if pid_alive "$server_pid"; then
            read -r server_rss server_cpu < <(ps -o rss=,pcpu= -p "$server_pid" 2>/dev/null | tr -s ' ')
            server_rss=${server_rss:-0}
            server_cpu=${server_cpu:-0}
        else
            server_rss=0
            server_cpu=0
        fi
        for pid in "${client_pids[@]}"; do
            if pid_alive "$pid"; then
                alive=$((alive + 1))
                local rss
                rss=$(ps -o rss= -p "$pid" 2>/dev/null | tr -d ' ')
                client_rss=$((client_rss + ${rss:-0}))
            fi
        done
        printf '%s,%s,%s,%s,%s,%s,%s\n' \
            "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$elapsed" "$server_rss" "$server_cpu" \
            "$client_rss" "$(pid_alive "$server_pid" && echo 1 || echo 0)" "$alive" \
            >>"$monitor"
        rotate_if_large "$monitor" "$rotate_bytes"
        rotate_if_large "$server_log" "$rotate_bytes"

        if ! pid_alive "$server_pid"; then
            server_crashed=1
            break
        fi
        # Check the duration before client liveness: clients finish exactly at
        # the duration and must not be mistaken for an early client failure.
        if [ "$elapsed" -ge "$duration" ]; then
            break
        fi
        if [ "$alive" -lt "$clients" ]; then
            had_client_failure=1
            break
        fi
        sleep "$sample_seconds"
    done

    # If the server died early, stop the clients instead of waiting out the soak.
    if [ "$server_crashed" -eq 1 ] || [ "$had_client_failure" -eq 1 ]; then
        for pid in "${client_pids[@]}"; do
            kill -KILL "$pid" 2>/dev/null || true
        done
    fi
    for pid in "${client_pids[@]}"; do
        wait "$pid" 2>/dev/null
    done

    if pid_alive "$server_pid"; then
        kill -INT "$server_pid" 2>/dev/null || true
        local deadline=$((SECONDS + 20))
        while pid_alive "$server_pid" && [ "$SECONDS" -lt "$deadline" ]; do
            sleep 0.1
        done
        if pid_alive "$server_pid"; then
            kill -KILL "$server_pid" 2>/dev/null || true
            server_crashed=1
        fi
    else
        server_crashed=1
    fi
    wait "$server_pid" 2>/dev/null
    server_status=$?
    # as-cmd exits 0 on SIGINT; a non-zero status is a crash/failure.
    if [ "$server_status" -ne 0 ]; then
        server_crashed=1
    fi

    local actual_elapsed=$(( $(date +%s) - start_epoch ))
    SOAK_ACTUAL_ELAPSED=$actual_elapsed SOAK_SERVER_STATUS=$server_status \
        SOAK_SERVER_CRASHED=$server_crashed SOAK_HAD_CLIENT_FAILURE=$had_client_failure \
        python3 -I - "$out" "$duration" "$clients" "$reconnect_every" "$actual_elapsed" <<'PY'
import glob
import json
import os
import statistics
import sys

out, requested, clients, reconnect_every, actual = sys.argv[1:]
requested = int(requested)
clients = int(clients)
actual = int(actual)
server_status = int(os.environ["SOAK_SERVER_STATUS"])
server_crashed = os.environ["SOAK_SERVER_CRASHED"] == "1"
had_client_failure = os.environ["SOAK_HAD_CLIENT_FAILURE"] == "1"

rss = []
cpu = []
with open(os.path.join(out, "monitor.csv")) as handle:
    header = handle.readline()
    for line in handle:
        parts = line.strip().split(",")
        if len(parts) < 7:
            continue
        try:
            rss.append(int(parts[2]))
            cpu.append(float(parts[3]))
        except ValueError:
            continue

reports = []
client_errors = []
for path in sorted(glob.glob(os.path.join(out, "client-*.json"))):
    try:
        with open(path) as handle:
            reports.append(json.load(handle))
    except (OSError, json.JSONDecodeError) as error:
        client_errors.append(f"{os.path.basename(path)}: {error}")

def total(key):
    return sum(int(report.get(key) or 0) for report in reports)

pcm_bytes = total("pcm_bytes")
datagrams = total("datagrams")
sessions = total("sessions")
reconnects = total("reconnects")
heartbeat_count = total("heartbeat_count")
heartbeat_misses = total("heartbeat_misses")
gaps50 = total("datagram_gaps_over_50ms")
gaps100 = total("datagram_gaps_over_100ms")
gaps250 = total("datagram_gaps_over_250ms")
max_gap = max((int(report.get("max_datagram_gap_ms") or 0) for report in reports), default=0)
client_ok = sum(1 for report in reports if report.get("ok") is True)
tone_present = all(
    (report.get("tone") or {}).get("present") is True for report in reports
) if reports else False

# Per-client throughput against sample_rate * block_align * elapsed time.
ratios = []
for report in reports:
    fmt = report.get("format") or {}
    rate = int(fmt.get("sample_rate") or 0)
    align = int(fmt.get("block_align") or 0)
    elapsed_ms = int(report.get("duration_elapsed_ms") or 0)
    if rate and align and elapsed_ms:
        expected = rate * align * (elapsed_ms / 1000.0)
        ratios.append(round(float(report.get("pcm_bytes") or 0) / expected, 4))
throughput_min = min(ratios) if ratios else None

summary = {
    "requested_duration_secs": requested,
    "actual_duration_secs": actual,
    "clients_requested": clients,
    "clients_reporting": len(reports),
    "clients_ok": client_ok,
    "client_process_failure": had_client_failure,
    "reconnect_every_secs": int(reconnect_every),
    "server_exit": server_status,
    "server_crashed": server_crashed,
    "server_rss_kb": {
        "min": min(rss) if rss else None,
        "max": max(rss) if rss else None,
        "mean": round(statistics.fmean(rss), 1) if rss else None,
    },
    "server_cpu_pct": {
        "max": max(cpu) if cpu else None,
        "mean": round(statistics.fmean(cpu), 3) if cpu else None,
    },
    "samples": len(rss),
    "pcm_bytes": pcm_bytes,
    "datagrams": datagrams,
    "sessions": sessions,
    "reconnects": reconnects,
    "heartbeat_count": heartbeat_count,
    "heartbeat_misses": heartbeat_misses,
    "glitch_gaps_over_50ms": gaps50,
    "glitch_gaps_over_100ms": gaps100,
    "glitch_gaps_over_250ms": gaps250,
    "max_datagram_gap_ms": max_gap,
    "throughput_ratio_min": throughput_min,
    "tone_present_all": tone_present,
    "client_errors": client_errors,
}
summary["pass"] = (
    not server_crashed
    and server_status == 0
    and client_ok == clients
    and heartbeat_misses == 0
    and tone_present
)

with open(os.path.join(out, "soak-summary.json"), "w") as handle:
    json.dump(summary, handle, indent=2)
    handle.write("\n")

print(
    "soak: {result} duration={actual}s clients={ok}/{clients} "
    "server_exit={exit} rss_max={rss}KB cpu_mean={cpu}% "
    "pcm={pcm}B datagrams={datagrams} heartbeats={hb} misses={misses} "
    "reconnects={reconnects} gaps>250ms={gaps} max_gap={max_gap}ms".format(
        result="PASS" if summary["pass"] else "FAIL",
        actual=actual,
        ok=client_ok,
        clients=clients,
        exit=server_status,
        rss=summary["server_rss_kb"]["max"],
        cpu=summary["server_cpu_pct"]["mean"],
        pcm=pcm_bytes,
        datagrams=datagrams,
        hb=heartbeat_count,
        misses=heartbeat_misses,
        reconnects=reconnects,
        gaps=gaps250,
        max_gap=max_gap,
    )
)
sys.exit(0 if summary["pass"] else 1)
PY
    local summary_status=$?
    echo "soak: results in $out"
    return "$summary_status"
}

# --------------------------------------------------------------- outer mode
out=""
duration_spec="8h"
clients=2
reconnect_every=1800
sample_seconds=5
rotate_mb=16
debug=0
skip_build=0
while [ $# -gt 0 ]; do
    case "$1" in
        --inner)
            inner_main
            exit $?
            ;;
        --duration)
            duration_spec=${2:?--duration needs a value}
            shift 2
            ;;
        --clients)
            clients=${2:?--clients needs a number}
            shift 2
            ;;
        --reconnect-every)
            reconnect_every=${2:?--reconnect-every needs seconds}
            shift 2
            ;;
        --sample-seconds)
            sample_seconds=${2:?--sample-seconds needs a number}
            shift 2
            ;;
        --rotate-mb)
            rotate_mb=${2:?--rotate-mb needs a number}
            shift 2
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

duration=$(parse_duration "$duration_spec") || exit 2
if [ "$duration" -le 0 ] || [ "$clients" -le 0 ] || [ "$sample_seconds" -le 0 ]; then
    echo "duration, clients, and sample seconds must be positive" >&2
    exit 2
fi
if [ -z "$out" ]; then
    out="/tmp/audio-share-soak/$(date -u +%Y%m%dT%H%M%SZ)"
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

export SOAK_OUT="$out" SOAK_DURATION="$duration" SOAK_CLIENTS="$clients" \
    SOAK_RECONNECT_EVERY="$reconnect_every" SOAK_SAMPLE_SECONDS="$sample_seconds" \
    SOAK_ROTATE_BYTES=$((rotate_mb * 1024 * 1024)) \
    SOAK_AS_CMD="$as_cmd" SOAK_HARNESS="$harness"

echo "soak output: $out" >&2
echo "requested duration: ${duration}s, clients: $clients, reconnect every: ${reconnect_every}s" >&2
"$ROOT/scripts/pw-test-session.sh" --tone-wav "$tone" -- "$SCRIPT_PATH" --inner
status=$?
if [ -f "$out/soak-summary.json" ]; then
    echo
    jq . "$out/soak-summary.json"
fi
exit "$status"
