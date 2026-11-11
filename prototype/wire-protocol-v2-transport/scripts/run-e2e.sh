#!/usr/bin/env bash
# wire-protocol-v2 transport prototype — desktop end-to-end smoke run (ticket 01).
#
# Starts the prototype server on an ephemeral loopback port, runs the desktop
# client through the happy path and every negative case, prints PASS/FAIL per
# case, and appends everything (with a header recording date, exact command,
# and versions) to evidence/desktop-e2e.log.
#
# Usage: scripts/run-e2e.sh [client-extra-args...]
set -u

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
SERVER_DIR="$ROOT/server"
BIN_DIR="$SERVER_DIR/target/release"
SERVER_BIN="$BIN_DIR/proto-server"
CLIENT_BIN="$BIN_DIR/proto-client"
EVIDENCE_DIR="$ROOT/evidence"
LOG="$EVIDENCE_DIR/desktop-e2e.log"
WORK="$(mktemp -d /tmp/proto-e2e.XXXXXX)"
SERVER_PID=""
SERVER2_PID=""
HOLD_PID=""

cleanup() {
    [[ -n "$HOLD_PID" ]] && kill "$HOLD_PID" 2>/dev/null
    for pid in "$SERVER_PID" "$SERVER2_PID"; do
        if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
            kill -INT "$pid" 2>/dev/null
            wait "$pid" 2>/dev/null
        fi
    done
    rm -rf "$WORK"
}
trap cleanup EXIT

mkdir -p "$EVIDENCE_DIR"

{
echo "=================================================================="
echo "=== wire-protocol-v2 transport prototype — desktop e2e run"
echo "=== date (UTC):  $(date -u '+%Y-%m-%dT%H:%M:%SZ')"
echo "=== command:     scripts/run-e2e.sh $*"
echo "=== host:        $(uname -srmo)"
echo "=== toolchain:   $(rustc --version 2>/dev/null || echo 'rustc not found'); $(cargo --version 2>/dev/null || echo 'cargo not found')"
if [[ -f "$SERVER_DIR/Cargo.lock" ]]; then
    CRATES=""
    for crate in quinn quinn-proto rustls ring rcgen tokio; do
        ver=$(grep -A1 "^name = \"$crate\"$" "$SERVER_DIR/Cargo.lock" | grep '^version' | head -1 | cut -d'"' -f2)
        [[ -n "$ver" ]] && CRATES="$CRATES $crate=$ver"
    done
    echo "=== crates:     $CRATES (from server/Cargo.lock)"
fi
echo "=================================================================="
} | tee -a "$LOG"

# Everything below is printed to stdout and appended to the evidence log.
exec > >(tee -a "$LOG") 2>&1

cd "$SERVER_DIR" || exit 1
echo "--- building release bins"
cargo build --release --quiet || { echo "FAIL: build"; exit 1; }

failures=0

# pass_case <name> <client-output> <rc>
check() {
    local name="$1" out="$2" rc="$3"
    shift 3
    local ok=true
    for pattern in "$@"; do
        if ! grep -q -- "$pattern" <<<"$out"; then
            echo "   FAIL: output lacks '$pattern'"
            ok=false
        fi
    done
    if [[ "$rc" -ne 0 ]]; then
        echo "   FAIL: client exited $rc"
        ok=false
    fi
    if $ok; then
        echo "PASS: $name"
    else
        echo "FAIL: $name"
        echo "$out" | sed 's/^/   | /'
        failures=$((failures + 1))
    fi
}

# --- start the server on an ephemeral port -------------------------------------
echo "--- starting server"
"$SERVER_BIN" --bind 127.0.0.1:0 --spki-out "$WORK/spki.hex" --idle-timeout-secs 60 \
    > "$WORK/server.log" 2>&1 &
SERVER_PID=$!

ready=""
for _ in $(seq 1 50); do
    if grep -q "PROTO_SERVER_READY" "$WORK/server.log" 2>/dev/null; then
        ready=1
        break
    fi
    sleep 0.1
done
if [[ -z "$ready" ]]; then
    echo "FAIL: server never became ready"; cat "$WORK/server.log"; exit 1
fi
ADDR=$(grep -o 'PROTO_SERVER_READY.*' "$WORK/server.log" | head -1 | sed -E 's/^.*addr=([^ ]+).*/\1/')
SPKI=$(head -1 "$WORK/spki.hex")
echo "--- server ready at $ADDR, spki ${SPKI:0:16}..."

echo "--- CASE 1: happy path (auth, subscribe, media, unsubscribe, close)"
out=$("$CLIENT_BIN" --addr "$ADDR" --pin "$SPKI" --duration-ms 800 --min-datagrams 80 "$@" 2>&1)
rc=$?
echo "$out" | sed 's/^/   | /'
check "happy path" "$out" "$rc" \
    "mode=happy ok=true" "gaps=0" "duplicates=0" "pattern_mismatches=0" \
    "timestamp_mismatches=0" "oversized=0" "misaligned=0" "close_reason=Normal"

echo "--- CASE 2: wrong ALPN (audio-share/v9) fails handshake before app data"
out=$("$CLIENT_BIN" --addr "$ADDR" --pin "$SPKI" --wrong-alpn 2>&1)
rc=$?
echo "$out" | sed 's/^/   | /'
check "wrong alpn" "$out" "$rc" "ok=true" "expected_failure=wrong-alpn"

echo "--- CASE 3: wrong pinned SPKI fails verification"
out=$("$CLIENT_BIN" --addr "$ADDR" --pin "$SPKI" --wrong-pin 2>&1)
rc=$?
echo "$out" | sed 's/^/   | /'
check "wrong pin" "$out" "$rc" "ok=true" "expected_failure=wrong-pin"

echo "--- CASE 4: wrong credential gets AUTH_ERR(BAD_CREDENTIAL) + CLOSE(AUTH_FAILED)"
out=$("$CLIENT_BIN" --addr "$ADDR" --pin "$SPKI" --wrong-credential 2>&1)
rc=$?
echo "$out" | sed 's/^/   | /'
check "wrong credential" "$out" "$rc" "ok=true" \
    "expected_failure=auth-failure" "auth_err=BadCredential" "close_after=Some(AuthFailed)"

echo "--- CASE 5: SUBSCRIBE without AUTH gets AUTH_ERR(NOT_AUTHENTICATED) + CLOSE(PROTOCOL_ERROR)"
out=$("$CLIENT_BIN" --addr "$ADDR" --pin "$SPKI" --mode no-auth-subscribe 2>&1)
rc=$?
echo "$out" | sed 's/^/   | /'
check "subscribe without auth" "$out" "$rc" "ok=true" \
    "auth_err=NotAuthenticated" "close_reason=ProtocolError"

echo "--- CASE 6: unknown msg_type gets CLOSE(PROTOCOL_ERROR)"
out=$("$CLIENT_BIN" --addr "$ADDR" --pin "$SPKI" --mode bad-msg-type 2>&1)
rc=$?
echo "$out" | sed 's/^/   | /'
check "unknown msg type" "$out" "$rc" "ok=true" "close_reason=ProtocolError"

echo "--- CASE 7: slow reader (pauses 500 ms mid-stream, control stays alive)"
out=$("$CLIENT_BIN" --addr "$ADDR" --pin "$SPKI" --duration-ms 900 --min-datagrams 60 \
    --stop-reading-ms 500 2>&1)
rc=$?
echo "$out" | sed 's/^/   | /'
check "slow reader" "$out" "$rc" "mode=happy ok=true" "close_reason=Normal"

echo "--- CASE 8: server stop delivers CLOSE(SERVER_STOP) to a subscribed client"
"$SERVER_BIN" --bind 127.0.0.1:0 --spki-out "$WORK/spki2.hex" > "$WORK/server2.log" 2>&1 &
SERVER2_PID=$!
ready2=""
for _ in $(seq 1 50); do
    if grep -q "PROTO_SERVER_READY" "$WORK/server2.log" 2>/dev/null; then ready2=1; break; fi
    sleep 0.1
done
if [[ -z "$ready2" ]]; then
    echo "FAIL: second server never became ready"; failures=$((failures + 1))
else
    ADDR2=$(grep -o 'PROTO_SERVER_READY.*' "$WORK/server2.log" | head -1 | sed -E 's/^.*addr=([^ ]+).*/\1/')
    "$CLIENT_BIN" --addr "$ADDR2" --pin-file "$WORK/spki2.hex" --mode hold \
        > "$WORK/hold.out" 2>&1 &
    HOLD_PID=$!
    sleep 0.6
    kill -INT "$SERVER2_PID"
    SERVER2_PID=""
    wait "$HOLD_PID"; hold_rc=$?
    HOLD_PID=""
    out=$(cat "$WORK/hold.out")
    echo "$out" | sed 's/^/   | /'
    check "server stop" "$out" "$hold_rc" \
        "mode=hold ok=true" "close_reason=ServerStop" "transport_code=1" "transport_text=server-stop"
fi

echo "=================================================================="
if [[ "$failures" -eq 0 ]]; then
    echo "RESULT: all 8 cases PASS"
else
    echo "RESULT: $failures case(s) FAILED"
fi
echo "=================================================================="
exit $((failures > 0))
