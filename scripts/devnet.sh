#!/usr/bin/env bash
# Start a local N-validator Aether devnet (default 4).
#   scripts/devnet.sh start [N]   scripts/devnet.sh stop
# Validators link over iroh (Mainline DHT + hole punching/relays) by default;
# AETHER_TRANSPORT=tcp uses plain loopback TCP instead (offline).
# AETHER_LOCAL="1 2 3" runs only those validators here (others run elsewhere,
# e.g. `aether node --index 4 --validators 4 ...` on another machine).
set -euo pipefail
cd "$(dirname "$0")/.."
BIN=${AETHER_BIN:-target/debug/aether}
DIR=${AETHER_DEVNET_DIR:-/tmp/aether-devnet}
N=${2:-4}

case "${1:-start}" in
  start)
    rm -rf "$DIR" && mkdir -p "$DIR"
    for i in ${AETHER_LOCAL:-$(seq 1 "$N")}; do
      peers=""
      if [ "${AETHER_TRANSPORT:-iroh}" = tcp ]; then
        peers="--peers $(for j in $(seq 1 "$N"); do [ "$j" != "$i" ] && printf '%s@127.0.0.1:%s,' "$j" $((9000 + j)); done | sed 's/,$//')"
      fi
      "$BIN" node --index "$i" --validators "$N" --port $((9000 + i)) --rpc-port $((8544 + i)) \
        --data "$DIR/$i" $peers > "$DIR/node$i.log" 2>&1 &
      echo $! > "$DIR/node$i.pid"
      echo "validator $i  p2p 127.0.0.1:$((9000 + i))  rpc http://127.0.0.1:$((8544 + i))  log $DIR/node$i.log"
    done ;;
  stop)
    for f in "$DIR"/node*.pid; do [ -f "$f" ] && kill "$(cat "$f")" 2>/dev/null || true; done
    echo "stopped" ;;
esac
