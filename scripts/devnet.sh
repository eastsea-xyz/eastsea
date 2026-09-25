#!/usr/bin/env bash
# Start a local N-validator Aether devnet (default 4).
#   scripts/devnet.sh start [N]   scripts/devnet.sh stop
#   scripts/devnet.sh dkg [N]     run the committee DKG (writes <dir>/<i>/threshold.json);
#                                 `start` keeps those keys and wipes only chain data.
# Validators link over iroh (Mainline DHT + hole punching/relays) by default;
# AETHER_TRANSPORT=tcp uses plain loopback TCP instead (offline).
# AETHER_LOCAL="1 2 3" runs only those validators here (others run elsewhere,
# e.g. `aether node --index 4 --validators 4 ...` on another machine).
# AETHER_NETWORK=path/network.json: real keys (each <dir>/<i> holds validator.key
# from `aether keygen`); validators find their index from their own key.
set -euo pipefail
cd "$(dirname "$0")/.."
BIN=${AETHER_BIN:-target/debug/aether}
DIR=${AETHER_DEVNET_DIR:-/tmp/aether-devnet}
N=${2:-4}

who() { if [ -n "${AETHER_NETWORK:-}" ]; then echo "--network $AETHER_NETWORK"; else echo "--index $1 --validators $N"; fi; }

# Nodes run from the network.json that dkg/reshare wrote into their dir (it has the identity).
whonode() { if [ -n "${AETHER_NETWORK:-}" ]; then echo "--network $DIR/$1/network.json"; else echo "--index $1 --validators $N"; fi; }

case "${1:-start}" in
  start)
    mkdir -p "$DIR"
    # Keep validator and DKG keys, wipe chain data.
    find "$DIR" -mindepth 1 -not -name threshold.json -not -name network.json -not -name validator.key -not -name validator.pub.json -not -type d -delete 2>/dev/null || true
    find "$DIR" -mindepth 1 -type d -empty -delete 2>/dev/null || true
    for i in ${AETHER_LOCAL:-$(seq 1 "$N")}; do
      peers=""
      if [ "${AETHER_TRANSPORT:-iroh}" = tcp ]; then
        peers="--peers $(for j in $(seq 1 "$N"); do [ "$j" != "$i" ] && printf '%s@127.0.0.1:%s,' "$j" $((9000 + j)); done | sed 's/,$//')"
      fi
      "$BIN" node $(whonode "$i") --port $((9000 + i)) --rpc-port $((8544 + i)) \
        --data "$DIR/$i" $peers > "$DIR/node$i.log" 2>&1 &
      echo $! > "$DIR/node$i.pid"
      echo "validator $i  p2p 127.0.0.1:$((9000 + i))  rpc http://127.0.0.1:$((8544 + i))  log $DIR/node$i.log"
    done ;;
  dkg)
    mkdir -p "$DIR"
    for i in ${AETHER_LOCAL:-$(seq 1 "$N")}; do
      [ -n "${AETHER_NETWORK:-}" ] || rm -rf "$DIR/$i"
      rm -f "$DIR/$i/threshold.json"
      "$BIN" dkg $(who "$i") --port $((9000 + i)) --data "$DIR/$i" > "$DIR/dkg$i.log" 2>&1 &
      echo "dkg validator $i  log $DIR/dkg$i.log"
    done
    wait
    grep -h "committee identity" "$DIR"/dkg*.log | sort | uniq -c ;;
  stop)
    for f in "$DIR"/node*.pid; do [ -f "$f" ] && kill "$(cat "$f")" 2>/dev/null || true; done
    echo "stopped" ;;
esac
