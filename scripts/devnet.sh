#!/usr/bin/env bash
# Start a local N-validator Aether devnet (default 4).
#   scripts/devnet.sh start [N]   scripts/devnet.sh stop
set -euo pipefail
cd "$(dirname "$0")/.."
BIN=${AETHER_BIN:-target/debug/aether}
DIR=${AETHER_DEVNET_DIR:-/tmp/aether-devnet}
N=${2:-4}

case "${1:-start}" in
  start)
    rm -rf "$DIR" && mkdir -p "$DIR"
    for i in $(seq 1 "$N"); do
      boot=""
      [ "$i" -gt 1 ] && boot="--bootstrap 1@127.0.0.1:9001"
      "$BIN" node --index "$i" --validators "$N" --port $((9000 + i)) --rpc-port $((8544 + i)) \
        --data "$DIR/$i" $boot > "$DIR/node$i.log" 2>&1 &
      echo $! > "$DIR/node$i.pid"
      echo "validator $i  p2p 127.0.0.1:$((9000 + i))  rpc http://127.0.0.1:$((8544 + i))  log $DIR/node$i.log"
    done ;;
  stop)
    for f in "$DIR"/node*.pid; do [ -f "$f" ] && kill "$(cat "$f")" 2>/dev/null || true; done
    echo "stopped" ;;
esac
