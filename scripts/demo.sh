#!/usr/bin/env bash
# End-to-end demo on a fresh local 4-validator devnet.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"   # rust-toolchain.toml pins 1.98.1
cargo build -q -p aether-node
A=target/debug/aether
say() { printf '\n\033[1;36m== %s\033[0m\n' "$*"; }

# A directory of its own, so keys left by other devnet or DKG runs are never picked up.
export AETHER_DEVNET_DIR=${AETHER_DEVNET_DIR:-/tmp/aether-demo}
scripts/devnet.sh stop >/dev/null 2>&1 || true
rm -rf "${AETHER_DEVNET_DIR:?}"
say "Starting 4 validators (Commonware simplex BFT, revm, EIP-7864 state)"
scripts/devnet.sh start 4
for _ in $(seq 120); do
  $A status --rpc http://127.0.0.1:8548 2>/dev/null | grep -q '"height": [3-9]' && break
  sleep 0.5
done
$A status --rpc http://127.0.0.1:8548 2>/dev/null | grep -q '"height": [3-9]' || { tail -5 "$AETHER_DEVNET_DIR"/node*.log; echo "validators did not start"; exit 1; }
$A status

BOB=0x00000000000000000000000000000000000b0b00
say "Transfer 12345 wei from dev account 1 (P-256 key) via validator 1"
$A send --rpc http://127.0.0.1:8545 --from-dev 1 --to $BOB --value 12345 --wait

say "Read Bob's balance from validator 3 and verify the Merkle proof locally"
$A balance $BOB --rpc http://127.0.0.1:8547

say "Deploy a counter contract via validator 2"
C=$($A deploy --rpc http://127.0.0.1:8546 --from-dev 2 --code 600a600c600039600a6000f360005460010160005500 | tee /dev/stderr | awk '/^contract:/{print $2}')

say "Call it 3 times via validator 4"
for _ in 1 2 3; do $A call --rpc http://127.0.0.1:8548 --from-dev 3 --to "$C" --wait | tail -1; done

say "Read counter storage via validator 1 with proof"
$A storage "$C" 0 --rpc http://127.0.0.1:8545

say "Every validator has the same block hash and state root"
H=$($A status --rpc http://127.0.0.1:8545 | awk -F': ' '/"height"/{gsub(/,/,"",$2);print $2}')
for p in 8545 8546 8547 8548; do
  curl -s -X POST localhost:$p -H 'content-type: application/json' \
    -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"aether_getBlock\",\"params\":[$H]}" |
    python3 -c "import sys,json;b=json.load(sys.stdin)['result'];print('validator rpc :$p  height',b['height'],' hash',b['hash'][:18],' root',b['state_root'][:18])"
done

say "Recent blocks"
$A blocks 8
say "Devnet keeps running. Stop it with: AETHER_DEVNET_DIR=$AETHER_DEVNET_DIR scripts/devnet.sh stop"
