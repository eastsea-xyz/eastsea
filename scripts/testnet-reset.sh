#!/usr/bin/env bash
# Start this Mac's testnet over with a new genesis (a protocol change the old
# chain cannot replay). Nothing is deleted: the old chain's data moves to
# $AETHER_TESTNET.stale-<time>. Each validator keeps its keys (same node ids),
# validator 1 keeps the faucet key and gets the voting-node registrar key.
#   scripts/testnet-reset.sh
# Then: the validators run under launchd (`aether run`), and the app's
# apps/wallet/Resources/network.json is the new one (ship it in a release).
set -euo pipefail
cd "$(dirname "$0")/.."
T=${AETHER_TESTNET:-$HOME/aether-testnet}
# A new genesis gets a new chain id: transactions signed for the old chain
# (same keys, same nonces) must not replay on the new one.
prev=$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['chain_id'])" "$T/network.json" 2>/dev/null || echo 7777)
CHAIN=${CHAIN_ID:-$((prev + 1))}
[ "$CHAIN" != "$prev" ] || { echo "CHAIN_ID must differ from the old chain ($prev)"; exit 1; }
A="$T/bin/aether"
export PATH="$HOME/.cargo/bin:$PATH"
N=$(ls -d "$T"/[0-9]* 2>/dev/null | wc -l | tr -d ' ')
[ "$N" -ge 4 ] || { echo "need at least 4 validator dirs in $T"; exit 1; }

cargo build -q --release -p aether-node
scripts/testnet-launchagent.sh uninstall >/dev/null 2>&1 || true
scripts/testnet.sh stop >/dev/null 2>&1 || true

aside="$T.stale-$(date +%s)"
mkdir -p "$aside"
for i in $(seq 1 "$N"); do
  mkdir -p "$aside/$i"
  for f in "$T/$i"/* "$T/$i"/.[!.]*; do
    [ -e "$f" ] || continue
    case "$(basename "$f")" in validator.key|validator.pub.json|faucet.key|registrar.key|node-account.key) ;; *) mv "$f" "$aside/$i/" ;; esac
  done
done
for f in "$T"/*.log "$T"/*.pid "$T"/*.json; do [ -e "$f" ] && mv "$f" "$aside/"; done
echo "old chain data moved to $aside"

cp target/release/aether "$A"
faucet=$("$A" faucet-key --data "$T/1" | awk '/faucet address/ {print $3}')
registrar=$("$A" registrar-key --data "$T/1" | awk '/registrar key/ {print $3}')
pubs=()
for i in $(seq 1 "$N"); do pubs+=("$T/$i/validator.pub.json"); done
"$A" network --chain-id "$CHAIN" --faucet "$faucet" --registrar "$registrar" "${pubs[@]}" > "$T/genesis.json"

# Key ceremony over loopback (all validators are on this Mac).
ports=(); for i in $(seq 1 "$N"); do ports+=($((9200 + i))); done
pids=()
for i in $(seq 1 "$N"); do
  peers=""
  for j in $(seq 1 "$N"); do [ "$j" = "$i" ] || peers+="${peers:+,}$j@127.0.0.1:${ports[$((j - 1))]}"; done
  "$A" dkg --network "$T/genesis.json" --port "${ports[$((i - 1))]}" --data "$T/$i" --peers "$peers" --offline > "$T/dkg$i.log" 2>&1 &
  pids+=($!)
done
for p in "${pids[@]}"; do wait "$p"; done
cp "$T/1/network.json" "$T/network.json"
cp "$T/network.json" apps/wallet/Resources/network.json
echo "new genesis: chain $CHAIN, $N validators, faucet $faucet, registrar $registrar"
python3 -c "import json; print('committee identity', json.load(open('$T/network.json'))['identity'][:32] + '…')"

scripts/testnet-launchagent.sh install
