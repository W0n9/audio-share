#!/usr/bin/env bash
# wire-protocol-v2 transport prototype — network impairment helper (ticket 01).
#
# All subcommands require root: run with sudo. Each subcommand is idempotent:
# safe to re-run, and every destructive action has a matching clear/remove.
# Interface defaults to `lo` for quick local runs — note netem on `lo` affects
# ALL loopback traffic on the host; prefer the veth pair (`veth-create`) for
# anything longer than a quick check.
#
# Subcommands:
#   loss <percent> [iface]              e.g. loss 5
#   reorder <gap> <delay-ms> <jitter-ms> [iface]
#                                       e.g. reorder 3 20 10   (every 3rd packet
#                                       delayed by 20ms ±10ms → reordering)
#   duplicate <percent> [iface]         e.g. duplicate 2
#   jitter <delay-ms> <jitter-ms> [iface]  e.g. jitter 10 5  (constant delay ± jitter)
#   rate <rate> [iface]                 e.g. rate 1mbit  (token bucket)
#   matrix [iface]                      loss 5% + reorder + duplicate + jitter in one qdisc
#   show [iface]                        tc qdisc show dev IFACE
#   clear [iface]                       remove the root qdisc from IFACE
#   veth-create [mtu] [prefix]          veth pair `prefix-a`/`prefix-b` with MTU,
#                                       peer moved into netns `prefix-ns`
#                                       (default mtu 1280, prefix "proto")
#   veth-remove [prefix]                remove the pair and its netns
#   help
#
# Tool: `tc qdisc replace dev IFACE root netem ...` — `replace` overwrites any
# existing root qdisc, making the add/modify cases idempotent. `clear` uses
# `tc qdisc del dev IFACE root || true` so it succeeds on an unimpaired iface.
#
# Notes:
# - netem delay/loss apply at the qdisc layer; QUIC sees them as RTT increase
#   and datagram loss without TCP-style retransmission pressure.
# - For the small-MTU gate use `veth-create 1280` and bind the server to
#   10.77.0.1 (IPv4) — the path MTU is then controlled by the veth MTU.
#   Re-enable DPLPMTUD (`--mtud` on both bins) to watch payload resizing react
#   to path-limit changes; the prototype default disables MTUD.
set -u

usage() { grep '^#   ' "$0" | sed 's/^#   //'; exit 1; }
need_root() {
    if [[ "$(id -u)" -ne 0 ]]; then
        echo "impairment.sh: must run as root (use sudo)" >&2
        exit 1
    fi
}

SUB="${1:-help}"
shift 2>/dev/null || true
case "$SUB" in
    help|-h|--help)
        usage
        ;;
    loss)
        need_root; PCT="${1:?usage: loss <percent> [iface]}"; shift; IFACE="${1:-lo}"
        # Idempotent: replace overwrites any existing root qdisc.
        tc qdisc replace dev "$IFACE" root netem loss "$PCT%"
        echo "netem loss $PCT% on $IFACE"
        tc qdisc show dev "$IFACE"
        ;;
    reorder)
        need_root; GAP="${1:?usage: reorder <gap> <delay-ms> <jitter-ms> [iface]}"; shift
        DELAY="${1:?}"; shift; JITTER="${1:?}"; shift; IFACE="${1:-lo}"
        # 25% of packets get DELAY±JITTER, taken from every GAPth slot —
        # with jitter < delay this reorders without dropping.
        tc qdisc replace dev "$IFACE" root netem delay "$DELAY"ms "$JITTER"ms reorder 25% gap "$GAP"
        echo "netem reorder 25% gap=$GAP delay=${DELAY}ms jitter=${JITTER}ms on $IFACE"
        tc qdisc show dev "$IFACE"
        ;;
    duplicate)
        need_root; PCT="${1:?usage: duplicate <percent> [iface]}"; shift; IFACE="${1:-lo}"
        tc qdisc replace dev "$IFACE" root netem duplicate "$PCT%"
        echo "netem duplicate $PCT% on $IFACE"
        tc qdisc show dev "$IFACE"
        ;;
    jitter)
        need_root; DELAY="${1:?usage: jitter <delay-ms> <jitter-ms> [iface]}"; shift
        JITTER="${1:?}"; shift; IFACE="${1:-lo}"
        tc qdisc replace dev "$IFACE" root netem delay "$DELAY"ms "$JITTER"ms distribution normal
        echo "netem delay ${DELAY}ms ±${JITTER}ms on $IFACE"
        tc qdisc show dev "$IFACE"
        ;;
    rate)
        need_root; RATE="${1:?usage: rate <rate> [iface] e.g. rate 1mbit}"; shift; IFACE="${1:-lo}"
        # tbf for the rate cap; netem does not rate-limit by itself.
        tc qdisc replace dev "$IFACE" root tbf rate "$RATE" burst 32kbit latency 200ms
        echo "tbf rate $RATE on $IFACE"
        tc qdisc show dev "$IFACE"
        ;;
    matrix)
        need_root; IFACE="${1:-lo}"
        # One combined profile: 5% loss, 25% of packets reordered (gap 4),
        # 2% duplication, 15ms±8ms jitter.
        tc qdisc replace dev "$IFACE" root netem delay 15ms 8ms reorder 25% gap 4 loss 5% duplicate 2%
        echo "combined impairment matrix on $IFACE"
        tc qdisc show dev "$IFACE"
        ;;
    show)
        IFACE="${1:-lo}"
        tc qdisc show dev "$IFACE"
        ;;
    clear)
        need_root; IFACE="${1:-lo}"
        # Idempotent: succeeds even when no qdisc is attached.
        tc qdisc del dev "$IFACE" root 2>/dev/null || true
        echo "root qdisc removed from $IFACE (if any)"
        tc qdisc show dev "$IFACE"
        ;;
    veth-create)
        need_root; MTU="${1:-1280}"; shift 2>/dev/null || true; PREFIX="${1:-proto}"
        NS="$PREFIX-ns"; A="$PREFIX-a"; B="$PREFIX-b"
        # Idempotent: create only what is missing.
        if ! ip netns list | grep -qw "$NS"; then
            ip netns add "$NS"
        fi
        if ! ip link show "$A" &>/dev/null; then
            ip link add "$A" type veth peer name "$B"
            ip link set "$B" netns "$NS"
            ip addr add 10.77.0.1/24 dev "$A"
            ip link set "$A" mtu "$MTU" up
        else
            # Still enforce the requested MTU on re-runs.
            ip link set "$A" mtu "$MTU"
        fi
        ip netns exec "$NS" ip addr add 10.77.0.2/24 dev "$B" 2>/dev/null || true
        ip netns exec "$NS" ip link set "$B" mtu "$MTU" up 2>/dev/null || true
        ip netns exec "$NS" ip link set lo up
        echo "veth $A (host, 10.77.0.1/24, MTU $MTU) <-> $B in netns $NS (10.77.0.2/24, MTU $MTU)"
        echo "run the server inside the namespace for a genuinely small-MTU path:"
        echo "  sudo ip netns exec $NS ./proto-server --bind 10.77.0.2:65531"
        echo "  ./proto-client --addr 10.77.0.2:65531 --pin ... "
        ;;
    veth-remove)
        need_root; PREFIX="${1:-proto}"
        NS="$PREFIX-ns"; A="$PREFIX-a"
        # Idempotent: deleting the netns removes its end; the host end may need
        # its own removal.
        ip netns del "$NS" 2>/dev/null || true
        ip link del "$A" 2>/dev/null || true
        echo "veth pair and netns removed"
        ;;
    *)
        echo "unknown subcommand: $SUB" >&2
        usage
        ;;
esac
