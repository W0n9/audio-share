#!/usr/bin/env bash
# Acceptance matrix for the v0.3.4 Wire Protocol against the real new Server.
#
# Drives `as-cmd` with the real Linux PipeWire capture backend inside a
# disposable headless PipeWire + WirePlumber session with a null `Audio/Sink`
# and a looped 440 Hz tone. Every combination is measured through the real
# protocol with `protocol-harness probe`; requested and *actually negotiated*
# formats are recorded separately.
#
# Usage:
#   scripts/acceptance-matrix.sh [options]
#
# Options:
#   --out DIR         Results directory (default: /tmp/audio-share-acceptance/<UTC>)
#   --duration SECS   Probe duration per combination (default 4; >= 4 observes
#                     one 3-second server heartbeat)
#   --ab-old PATH     Also run the A/B subset against the old as-cmd 0.3.4
#                     binary at PATH (f32/s8/s16/s24/s32, 48 kHz stereo).
#   --debug           Use target/debug binaries instead of release.
#   --skip-build      Do not run cargo build first.
#   -h, --help        Show this help.
#
# Requires: cargo, pipewire, wireplumber, pw-cli, pw-play, pw-dump,
# dbus-run-session, python3, jq. No sudo and no user services are touched.
#
# Results:
#   matrix.tsv       one row per encoding/channel/rate combination
#   scenarios.tsv    multi-client, reconnect, heartbeat, stop, start/stop rows
#   ab.tsv           old-as-cmd vs new rows (with --ab-old)
#   cases/*.json     raw probe reports
#   logs/*.out       server stdout/stderr per case
#
# Exit status: 0 when every combination and scenario passed (a server that
# honestly rejects a combination is recorded as not-supported, not silently
# passed); 1 when any probe check failed.

set -uo pipefail

SCRIPT_PATH=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")
ROOT=$(cd "$(dirname "$SCRIPT_PATH")/.." && pwd)

usage() {
    sed -n '2,36p' "$SCRIPT_PATH" | sed 's/^# \{0,1\}//'
}

# ---------------------------------------------------------------- inner mode
inner_main() {
    local rate=$1
    local out=${MATRIX_OUT:?}
    local as_cmd=${MATRIX_AS_CMD:?}
    local harness=${MATRIX_HARNESS:?}
    local duration=${MATRIX_DURATION:?}
    local run_scenarios=${MATRIX_RUN_SCENARIOS:-0}
    local ab_old=${MATRIX_AB_OLD:-}
    local serial=${PW_TEST_ENDPOINT_SERIAL:?}
    local tsv="$out/matrix.tsv"
    local scenarios="$out/scenarios.tsv"
    local ab_tsv="$out/ab.tsv"
    local failures="$out/failures.txt"
    local port server_pid server_exit server_log

    new_port() {
        python3 -I -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()'
    }

    wait_tcp() {
        local deadline=$((SECONDS + 15))
        while [ "$SECONDS" -lt "$deadline" ]; do
            if (echo >"/dev/tcp/127.0.0.1/$port") 2>/dev/null; then
                return 0
            fi
            if ! kill -0 "$server_pid" 2>/dev/null; then
                return 1
            fi
            sleep 0.05
        done
        return 1
    }

    start_server() {
        # args: encoding channels rate label [binary]
        local encoding=$1 channels=$2 case_rate=$3 label=$4
        local binary=${5:-$as_cmd}
        port=$(new_port)
        server_log="$out/logs/server-$label.out"
        "$binary" --bind="127.0.0.1:$port" -e "$serial" \
            --encoding="$encoding" --channels="$channels" --sample-rate="$case_rate" \
            >"$server_log" 2>&1 &
        server_pid=$!
        wait_tcp
    }

    stop_server() {
        server_exit=""
        if kill -0 "$server_pid" 2>/dev/null; then
            kill -INT "$server_pid" 2>/dev/null || true
        fi
        local deadline=$((SECONDS + 5))
        while kill -0 "$server_pid" 2>/dev/null && [ "$SECONDS" -lt "$deadline" ]; do
            sleep 0.05
        done
        # The old 0.3.4 binary ignores SIGINT; TERM is its normal shutdown.
        if kill -0 "$server_pid" 2>/dev/null; then
            kill -TERM "$server_pid" 2>/dev/null || true
            deadline=$((SECONDS + 5))
            while kill -0 "$server_pid" 2>/dev/null && [ "$SECONDS" -lt "$deadline" ]; do
                sleep 0.05
            done
        fi
        if kill -0 "$server_pid" 2>/dev/null; then
            kill -KILL "$server_pid" 2>/dev/null || true
        fi
        wait "$server_pid" 2>/dev/null
        server_exit=$?
    }

    server_tail() {
        tail -n 2 "$server_log" 2>/dev/null | tr '\n' '|' | tr '\t' ' '
    }

    # Append one matrix row extracted from a probe JSON report.
    record_row() {
        # args: kind requested status detail json
        local kind=$1 requested=$2 status=$3 detail=$4 json=$5
        local actual_encoding=- actual_channels=- actual_rate=- block_align=-
        local datagrams=0 pcm_bytes=0 max_datagram=0 full=0 partial=0
        local tone_hz=- tone_amp=- tone_present=- heartbeats=0 misses=0 sessions=0
        if [ -s "$json" ] && jq -e . "$json" >/dev/null 2>&1; then
            actual_encoding=$(jq -r '.format.encoding // "-"' "$json")
            actual_channels=$(jq -r '.format.channels // "-"' "$json")
            actual_rate=$(jq -r '.format.sample_rate // "-"' "$json")
            block_align=$(jq -r '.format.block_align // "-"' "$json")
            datagrams=$(jq -r '.datagrams // 0' "$json")
            pcm_bytes=$(jq -r '.pcm_bytes // 0' "$json")
            max_datagram=$(jq -r '.max_datagram_bytes // 0' "$json")
            full=$(jq -r '.full_datagrams // 0' "$json")
            partial=$(jq -r '.partial_datagrams // 0' "$json")
            if jq -e '.tone != null' "$json" >/dev/null 2>&1; then
                tone_hz=$(jq -r '.tone.detected_hz' "$json")
                tone_amp=$(jq -r '.tone.amplitude' "$json")
                tone_present=$(jq -r '.tone.present' "$json")
            fi
            heartbeats=$(jq -r '.heartbeat_count // 0' "$json")
            misses=$(jq -r '.heartbeat_misses // 0' "$json")
            sessions=$(jq -r '.sessions // 0' "$json")
        fi
        printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
            "$kind" "$requested" "$actual_encoding" "$actual_channels" "$actual_rate" \
            "$block_align" "$datagrams" "$pcm_bytes" "$max_datagram" "$full" "$partial" \
            "$tone_hz" "$tone_amp" "$tone_present" "$heartbeats" "$misses" "$sessions" \
            "$status" "$detail" >>"$tsv"
    }

    record_failure() {
        printf '%s\n' "$1" >>"$failures"
    }

    record_scenario() {
        # args: name status detail
        printf '%s\t%s\t%s\n' "$1" "$2" "$3" >>"$scenarios"
    }

    # One encoding/channel/rate combination.
    run_matrix_case() {
        local encoding=$1 channels=$2 case_rate=$3
        local label="$encoding-${channels}ch-${case_rate}"
        local requested="$encoding/${channels}ch/${case_rate}"
        local json="$out/cases/$label.json"
        local probe_log="$out/logs/probe-$label.out"
        rm -f "$json"
        if ! start_server "$encoding" "$channels" "$case_rate" "$label"; then
            stop_server
            record_row "matrix" "$requested" "not-supported" \
                "server_exit=${server_exit:-?} $(server_tail)" "$json"
            echo "NOT-SUPPORTED $label server_exit=${server_exit:-?}"
            return
        fi
        "$harness" probe --connect "127.0.0.1:$port" --duration-secs "$duration" \
            --tone-hz 440 --label "$label" --json-out "$json" \
            >/dev/null 2>"$probe_log"
        local probe_status=$?
        stop_server
        local actual_encoding=- actual_rate=- actual_channels=-
        if [ -s "$json" ]; then
            actual_encoding=$(jq -r '.format.encoding // "-"' "$json")
            actual_channels=$(jq -r '.format.channels // "-"' "$json")
            actual_rate=$(jq -r '.format.sample_rate // "-"' "$json")
        fi
        local status detail
        if [ "$probe_status" -ne 0 ] || [ "${server_exit:-1}" -ne 0 ]; then
            status=fail
            detail="probe_exit=$probe_status server_exit=${server_exit:-?} $(server_tail) $(tail -n 1 "$probe_log" 2>/dev/null)"
            record_failure "$label"
        elif [ "$actual_encoding" != "$encoding" ] \
            || [ "$actual_channels" != "$channels" ] \
            || [ "$actual_rate" != "$case_rate" ]; then
            status=pass-changed
            detail="requested=${requested} actual=${actual_encoding}/${actual_channels}ch/${actual_rate}"
        else
            status=pass
            detail=""
        fi
        record_row "matrix" "$requested" "$status" "$detail" "$json"
        echo "$status $label $( [ -n "$detail" ] && echo "($detail)" )"
    }

    # Three simultaneous clients on one server.
    run_multi_client() {
        local label=multi-client json="$out/cases/multi-client.json"
        local names=(multi-1 multi-2 multi-3) pids=() statuses=()
        if ! start_server f32 2 48000 "$label"; then
            stop_server
            record_scenario "$label" not-supported "server_exit=${server_exit:-?} $(server_tail)"
            record_failure "$label"
            return
        fi
        for index in 0 1 2; do
            "$harness" probe --connect "127.0.0.1:$port" --duration-secs 3 --tone-hz 440 \
                --label "${names[$index]}" --json-out "$out/cases/${names[$index]}.json" \
                >/dev/null 2>"$out/logs/${names[$index]}.err" &
            pids+=("$!")
        done
        local all_ok=1 index=0
        for pid in "${pids[@]}"; do
            wait "$pid"
            statuses+=("$?")
            index=$((index + 1))
        done
        stop_server
        index=0
        while [ "$index" -lt 3 ]; do
            if [ "${statuses[$index]}" -ne 0 ] \
                || ! jq -e '.ok == true and .datagrams > 0 and .tone.present == true' \
                    "$out/cases/${names[$index]}.json" >/dev/null 2>&1; then
                all_ok=0
            fi
            index=$((index + 1))
        done
        if [ "${server_exit:-1}" -ne 0 ]; then
            all_ok=0
        fi
        local datagrams
        datagrams=$(jq -s 'map(.datagrams) | add' \
            "$out/cases/multi-1.json" "$out/cases/multi-2.json" "$out/cases/multi-3.json" 2>/dev/null || echo 0)
        cp "$out/cases/multi-1.json" "$json" 2>/dev/null || true
        if [ "$all_ok" -eq 1 ]; then
            record_scenario "$label" pass "3 clients, total datagrams=$datagrams"
            echo "pass $label (3 clients)"
        else
            record_scenario "$label" fail "client exits=${statuses[*]} server_exit=${server_exit:-?}"
            record_failure "$label"
            echo "fail $label"
        fi
    }

    # One client disconnect/reconnect cycle within a single probe process.
    run_reconnect() {
        local label=reconnect json="$out/cases/reconnect.json"
        if ! start_server f32 2 48000 "$label"; then
            stop_server
            record_scenario "$label" not-supported "server_exit=${server_exit:-?} $(server_tail)"
            record_failure "$label"
            return
        fi
        "$harness" probe --connect "127.0.0.1:$port" --duration-secs 5 \
            --reconnect-every-secs 1 --reconnect-delay-ms 200 --tone-hz 440 \
            --label "$label" --json-out "$json" >/dev/null 2>"$out/logs/$label.err"
        local probe_status=$?
        stop_server
        if [ "$probe_status" -eq 0 ] && [ "${server_exit:-1}" -eq 0 ] \
            && jq -e '.ok == true and .reconnects >= 2 and (.start_play_ids | length) >= 3 and (.start_play_ids as $ids | all(range(1; $ids | length); $ids[.] > $ids[.-1]))' \
                "$json" >/dev/null 2>&1; then
            record_scenario "$label" pass "$(jq -c '{reconnects,start_play_ids,datagrams}' "$json")"
            echo "pass $label"
        else
            record_scenario "$label" fail "probe_exit=$probe_status server_exit=${server_exit:-?} $(jq -c '{ok,reconnects,start_play_ids,errors}' "$json" 2>/dev/null)"
            record_failure "$label"
            echo "fail $label"
        fi
    }

    # A reply heartbeat pumped for longer than the 5-second timeout.
    run_heartbeat_active() {
        local label=heartbeat-active json="$out/cases/heartbeat-active.json"
        if ! start_server f32 2 48000 "$label"; then
            stop_server
            record_scenario "$label" not-supported "server_exit=${server_exit:-?} $(server_tail)"
            record_failure "$label"
            return
        fi
        "$harness" probe --connect "127.0.0.1:$port" --duration-secs 8 --tone-hz 440 \
            --label "$label" --json-out "$json" >/dev/null 2>"$out/logs/$label.err"
        local probe_status=$?
        stop_server
        if [ "$probe_status" -eq 0 ] && [ "${server_exit:-1}" -eq 0 ] \
            && jq -e '.ok == true and .heartbeat_count >= 2 and .heartbeat_misses == 0 and .server_closed == false' \
                "$json" >/dev/null 2>&1; then
            record_scenario "$label" pass "$(jq -c '{heartbeat_count,heartbeat_gaps_ms,heartbeat_misses}' "$json")"
            echo "pass $label"
        else
            record_scenario "$label" fail "probe_exit=$probe_status $(jq -c '{ok,heartbeat_count,heartbeat_misses,server_closed,errors}' "$json" 2>/dev/null)"
            record_failure "$label"
            echo "fail $label"
        fi
    }

    # A silent client must be closed on the first 3-second tick after 5 s.
    run_heartbeat_timeout() {
        local label=heartbeat-timeout json="$out/cases/heartbeat-timeout.json"
        if ! start_server f32 2 48000 "$label"; then
            stop_server
            record_scenario "$label" not-supported "server_exit=${server_exit:-?} $(server_tail)"
            record_failure "$label"
            return
        fi
        "$harness" probe --connect "127.0.0.1:$port" --duration-secs 12 \
            --no-heartbeat-reply --expect-server-close --tone-hz 0 --min-datagrams 0 \
            --label "$label" --json-out "$json" >/dev/null 2>"$out/logs/$label.err"
        local probe_status=$?
        stop_server
        local closed_after
        closed_after=$(jq -r '.closed_after_ms // 0' "$json" 2>/dev/null || echo 0)
        if [ "$probe_status" -eq 0 ] && [ "${server_exit:-1}" -eq 0 ] \
            && jq -e '.ok == true and .server_closed == true and .closed_after_ms >= 4500 and .closed_after_ms <= 9500' \
                "$json" >/dev/null 2>&1; then
            record_scenario "$label" pass "closed_after_ms=$closed_after"
            echo "pass $label (closed after ${closed_after} ms)"
        else
            record_scenario "$label" fail "probe_exit=$probe_status closed_after_ms=$closed_after $(jq -c '{ok,errors}' "$json" 2>/dev/null)"
            record_failure "$label"
            echo "fail $label (closed_after_ms=$closed_after)"
        fi
    }

    # SIGINT to the server must close the active TCP session and exit 0.
    run_server_stop() {
        local label=server-stop json="$out/cases/server-stop.json"
        local ready="$out/cases/server-stop.ready"
        rm -f "$ready" "$json"
        if ! start_server f32 2 48000 "$label"; then
            stop_server
            record_scenario "$label" not-supported "server_exit=${server_exit:-?} $(server_tail)"
            record_failure "$label"
            return
        fi
        "$harness" probe --connect "127.0.0.1:$port" --duration-secs 20 \
            --expect-server-close --tone-hz 0 --label "$label" \
            --ready-file "$ready" --json-out "$json" >/dev/null 2>"$out/logs/$label.err" &
        local probe_pid=$!
        local deadline=$((SECONDS + 10))
        while [ ! -f "$ready" ] && [ "$SECONDS" -lt "$deadline" ]; do
            sleep 0.05
        done
        if [ ! -f "$ready" ]; then
            kill -KILL "$probe_pid" 2>/dev/null || true
            stop_server
            record_scenario "$label" fail "probe did not reach START_PLAY/UDP bind"
            record_failure "$label"
            echo "fail $label (probe not ready)"
            return
        fi
        kill -INT "$server_pid" 2>/dev/null || true
        wait "$probe_pid"
        local probe_status=$?
        stop_server
        local closed_after
        closed_after=$(jq -r '.closed_after_ms // 0' "$json" 2>/dev/null || echo 0)
        if [ "$probe_status" -eq 0 ] && [ "${server_exit:-1}" -eq 0 ] \
            && jq -e '.ok == true and .server_closed == true and .closed_after_ms <= 5000' \
                "$json" >/dev/null 2>&1; then
            record_scenario "$label" pass "server_exit=0 closed_after_ms=$closed_after"
            echo "pass $label (closed after ${closed_after} ms)"
        else
            record_scenario "$label" fail "probe_exit=$probe_status server_exit=${server_exit:-?} closed_after_ms=$closed_after"
            record_failure "$label"
            echo "fail $label"
        fi
    }

    # Repeated Start -> stream -> Stop cycles on fresh server processes.
    run_start_stop_cycles() {
        local label=start-stop-cycles
        local cycle=1 all_ok=1
        while [ "$cycle" -le 3 ]; do
            local json="$out/cases/start-stop-$cycle.json"
            if ! start_server f32 2 48000 "$label-$cycle"; then
                stop_server
                all_ok=0
                break
            fi
            "$harness" probe --connect "127.0.0.1:$port" --duration-secs 1 --tone-hz 0 \
                --label "$label-$cycle" --json-out "$json" >/dev/null 2>"$out/logs/$label-$cycle.err"
            local probe_status=$?
            stop_server
            if [ "$probe_status" -ne 0 ] || [ "${server_exit:-1}" -ne 0 ]; then
                all_ok=0
            fi
            cycle=$((cycle + 1))
        done
        if [ "$all_ok" -eq 1 ]; then
            record_scenario "$label" pass "3 start/stop cycles, all probes and servers exited 0"
            echo "pass $label"
        else
            record_scenario "$label" fail "a cycle failed; see logs"
            record_failure "$label"
            echo "fail $label"
        fi
    }

    record_ab_row() {
        # args: server requested status detail json
        local server=$1 requested=$2 status=$3 detail=$4 json=$5
        local actual_encoding=- actual_channels=- actual_rate=- block_align=-
        local datagrams=0 pcm_bytes=0 max_datagram=0 full=0 partial=0
        local tone_hz=- tone_amp=- tone_present=-
        if [ -s "$json" ] && jq -e . "$json" >/dev/null 2>&1; then
            actual_encoding=$(jq -r '.format.encoding // "-"' "$json")
            actual_channels=$(jq -r '.format.channels // "-"' "$json")
            actual_rate=$(jq -r '.format.sample_rate // "-"' "$json")
            block_align=$(jq -r '.format.block_align // "-"' "$json")
            datagrams=$(jq -r '.datagrams // 0' "$json")
            pcm_bytes=$(jq -r '.pcm_bytes // 0' "$json")
            max_datagram=$(jq -r '.max_datagram_bytes // 0' "$json")
            full=$(jq -r '.full_datagrams // 0' "$json")
            partial=$(jq -r '.partial_datagrams // 0' "$json")
            if jq -e '.tone != null' "$json" >/dev/null 2>&1; then
                tone_hz=$(jq -r '.tone.detected_hz' "$json")
                tone_amp=$(jq -r '.tone.amplitude' "$json")
                tone_present=$(jq -r '.tone.present' "$json")
            fi
        fi
        printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
            "$server" "$requested" "$actual_encoding" "$actual_channels" "$actual_rate" \
            "$block_align" "$datagrams" "$pcm_bytes" "$max_datagram" "$full" "$partial" \
            "$tone_hz" "$tone_amp" "$tone_present" "$status" "$detail" >>"$ab_tsv"
        case "$status" in
            pass|not-supported|pass-changed) ;;
            *) record_failure "ab-$server-$requested" ;;
        esac
        echo "$status ab-$server-$requested ${detail:+($detail)}"
    }

    run_ab_case() {
        local server=$1 binary=$2 encoding=$3
        local label="ab-$server-$encoding"
        local json="$out/cases/$label.json"
        rm -f "$json"
        if ! start_server "$encoding" 2 48000 "$label" "$binary"; then
            stop_server
            # The old 0.3.4 Linux path exits 1 for s8; record the rejection.
            record_ab_row "$server" "$encoding/2ch/48000" not-supported \
                "server_exit=${server_exit:-?}" "$json"
            return
        fi
        "$harness" probe --connect "127.0.0.1:$port" --duration-secs 2 --tone-hz 440 \
            --label "$label" --json-out "$json" >/dev/null 2>"$out/logs/$label.err"
        local probe_status=$?
        stop_server
        local actual_encoding=- actual_channels=- actual_rate=-
        if [ -s "$json" ]; then
            actual_encoding=$(jq -r '.format.encoding // "-"' "$json")
            actual_channels=$(jq -r '.format.channels // "-"' "$json")
            actual_rate=$(jq -r '.format.sample_rate // "-"' "$json")
        fi
        if [ "$probe_status" -eq 0 ]; then
            if [ "$actual_encoding" = "$encoding" ] && [ "$actual_channels" = 2 ] && [ "$actual_rate" = 48000 ]; then
                record_ab_row "$server" "$encoding/2ch/48000" pass "" "$json"
            else
                record_ab_row "$server" "$encoding/2ch/48000" pass-changed \
                    "actual=$actual_encoding/${actual_channels}ch/$actual_rate" "$json"
            fi
        else
            # The old binary exits on SIGINT with 130; only the probe matters.
            record_ab_row "$server" "$encoding/2ch/48000" fail \
                "probe_exit=$probe_status $(tail -n 1 "$out/logs/$label.err" 2>/dev/null)" "$json"
        fi
    }

    # ------------------------------------------------------------- main loop
    if [ "$run_scenarios" = 1 ]; then
        printf 'name\tstatus\tdetail\n' >"$scenarios"
        printf 'server\trequested\tactual_encoding\tactual_channels\tactual_rate\tblock_align\tdatagrams\tpcm_bytes\tmax_datagram\tfull\tpartial\ttone_hz\ttone_amp\ttone_present\tstatus\tdetail\n' >"$ab_tsv"
    fi

    for encoding in f32 s8 s16 s24 s32; do
        for channels in 1 2; do
            run_matrix_case "$encoding" "$channels" "$rate"
        done
    done

    if [ "$run_scenarios" = 1 ]; then
        run_multi_client
        run_reconnect
        run_heartbeat_active
        run_heartbeat_timeout
        run_server_stop
        run_start_stop_cycles
        if [ -n "$ab_old" ]; then
            if [ ! -x "$ab_old" ]; then
                echo "ab-old binary is not executable: $ab_old" >&2
                record_failure "ab-old-missing"
            else
                for encoding in f32 s8 s16 s24 s32; do
                    run_ab_case old "$ab_old" "$encoding"
                    run_ab_case new "$as_cmd" "$encoding"
                done
            fi
        fi
    fi
}

# --------------------------------------------------------------- outer mode
out=""
debug=0
skip_build=0
duration=4
ab_old=""
while [ $# -gt 0 ]; do
    case "$1" in
        --inner)
            inner_main "${2:?--inner needs a sample rate}"
            exit 0
            ;;
        --out)
            out=${2:?--out needs a directory}
            shift 2
            ;;
        --duration)
            duration=${2:?--duration needs seconds}
            shift 2
            ;;
        --ab-old)
            ab_old=${2:?--ab-old needs a path}
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
    out="/tmp/audio-share-acceptance/$(date -u +%Y%m%dT%H%M%SZ)"
fi
mkdir -p "$out/logs" "$out/cases"
: >"$out/failures.txt"
printf 'kind\trequested\tactual_encoding\tactual_channels\tactual_rate\tblock_align\tdatagrams\tpcm_bytes\tmax_datagram\tfull\tpartial\ttone_hz\ttone_amp\ttone_present\theartbeats\theartbeat_misses\tsessions\tstatus\tdetail\n' >"$out/matrix.tsv"

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

tone48="$out/tone-48000.wav"
tone44="$out/tone-44100.wav"
python3 "$ROOT/scripts/gen-tone-wav.py" "$tone48" --frequency 440 \
    --sample-rate 48000 --channels 2 --duration 30 --amplitude 0.25 >/dev/null
python3 "$ROOT/scripts/gen-tone-wav.py" "$tone44" --frequency 440 \
    --sample-rate 44100 --channels 2 --duration 30 --amplitude 0.25 >/dev/null

export MATRIX_OUT="$out" MATRIX_AS_CMD="$as_cmd" MATRIX_HARNESS="$harness" \
    MATRIX_DURATION="$duration" MATRIX_AB_OLD="$ab_old"

echo "acceptance matrix output: $out" >&2
echo "running 48 kHz combinations, scenarios, and A/B..." >&2
MATRIX_RUN_SCENARIOS=1 PW_TONE="$tone48" \
    "$ROOT/scripts/pw-test-session.sh" --tone-wav "$tone48" -- \
    "$SCRIPT_PATH" --inner 48000 || echo "warning: 48 kHz session exited $?" >&2

echo "running 44.1 kHz combinations..." >&2
MATRIX_RUN_SCENARIOS=0 \
    "$ROOT/scripts/pw-test-session.sh" --tone-wav "$tone44" -- \
    "$SCRIPT_PATH" --inner 44100 || echo "warning: 44.1 kHz session exited $?" >&2

# ---------------------------------------------------------------- summary
matrix_total=$(tail -n +2 "$out/matrix.tsv" | wc -l)
pass=$(tail -n +2 "$out/matrix.tsv" | awk -F'\t' '$18=="pass"' | wc -l)
changed=$(tail -n +2 "$out/matrix.tsv" | awk -F'\t' '$18=="pass-changed"' | wc -l)
unsupported=$(tail -n +2 "$out/matrix.tsv" | awk -F'\t' '$18=="not-supported"' | wc -l)
failed=$(tail -n +2 "$out/matrix.tsv" | awk -F'\t' '$18=="fail"' | wc -l)
scenario_pass=0
scenario_fail=0
if [ -s "$out/scenarios.tsv" ]; then
    scenario_pass=$(tail -n +2 "$out/scenarios.tsv" | awk -F'\t' '$2=="pass"' | wc -l)
    scenario_fail=$(tail -n +2 "$out/scenarios.tsv" | awk -F'\t' '$2=="fail"' | wc -l)
fi

echo
echo "matrix: $matrix_total combinations, $pass pass, $changed pass-changed, $unsupported not-supported, $failed fail"
echo "scenarios: $scenario_pass pass, $scenario_fail fail"
if [ -s "$out/failures.txt" ]; then
    echo "failures:"
    sed 's/^/  - /' "$out/failures.txt"
fi
echo "results: $out"

if [ "$failed" -ne 0 ] || [ "$scenario_fail" -ne 0 ] || [ -s "$out/failures.txt" ]; then
    exit 1
fi
exit 0
