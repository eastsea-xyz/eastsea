#!/usr/bin/env bash
# Run this Mac's testnet validators (keys and chain data in $AETHER_TESTNET, default ~/aether-testnet).
#   scripts/testnet.sh start | stop | status
# Unlike devnet.sh, `start` never wipes chain data: the chain resumes where it stopped.
# Validator 1 also serves the faucet (its key: <dir>/1/faucet.key).
# While a validator runs, `caffeinate -s -w <pid>` keeps the Mac awake on power.
set -euo pipefail
T=${AETHER_TESTNET:-$HOME/aether-testnet}
A="$T/bin/aether"
N=$(ls -d "$T"/[0-9]* 2>/dev/null | wc -l | tr -d ' ')
rpc() { curl -s -m 3 -X POST -H 'content-type: application/json' --data '{"jsonrpc":"2.0","id":1,"method":"aether_status","params":[]}' "http://127.0.0.1:$((8600 + $1))"; }
case "${1:-status}" in
  start)
    for i in $(seq 1 "$N"); do
      if [ -f "$T/node$i.pid" ] && kill -0 "$(cat "$T/node$i.pid")" 2>/dev/null; then echo "validator $i already running"; continue; fi
      args=(run --network "$T/$i/network.json" --port $((9100 + i)) --rpc-port $((8600 + i)) --data "$T/$i")
      [ -f "$T/$i/faucet.key" ] && args+=("--node-arg=--faucet-key=$T/$i/faucet.key")
      nohup "$A" "${args[@]}" >> "$T/node$i.log" 2>&1 &
      echo $! > "$T/node$i.pid"
      # No system sleep on power while this validator runs (ends with it).
      nohup caffeinate -s -w "$!" >/dev/null 2>&1 &
      echo "validator $i  rpc http://127.0.0.1:$((8600 + i))  log $T/node$i.log"
    done ;;
  stop)
    for i in $(seq 1 "$N"); do [ -f "$T/node$i.pid" ] && kill "$(cat "$T/node$i.pid")" 2>/dev/null || true; done
    echo "stopped" ;;
  status)
    for i in $(seq 1 "$N"); do
      h=$(rpc "$i" | python3 -c "import sys,json; print(json.load(sys.stdin)['result']['height'])" 2>/dev/null || echo down)
      echo "validator $i  height $h"
    done ;;
esac
