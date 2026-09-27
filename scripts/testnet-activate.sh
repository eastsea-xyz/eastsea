#!/usr/bin/env bash
# Activate a protocol version on this Mac's testnet: the running committee signs
# the upgrade with its key shares (3 of 4), every validator gets the signed file,
# the next proposer puts it on chain, and it activates one voting-node epoch
# (plus a margin) later. Run scripts/testnet-upgrade.sh first so every validator
# already runs a binary that knows the protocol.
#   scripts/testnet-activate.sh <protocol> [notes]
set -euo pipefail
cd "$(dirname "$0")/.."
T=${AETHER_TESTNET:-$HOME/aether-testnet}
A="$T/bin/aether"
P=${1:?usage: $0 <protocol> [notes]}
NOTES=${2:-}
NET="$T/1/network.json"
S=$(mktemp -d)
trap 'rm -rf "$S"' EXIT

rpc() { curl -s -m 3 localhost:8601 -H 'content-type: application/json' -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":[]}"; }
height=$(rpc aether_status | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["height"])')
chain=$(python3 -c "import json; print(json.load(open('$NET'))['chain_id'])")
epoch=$(rpc aether_rotation | python3 -c 'import json,sys; r=json.load(sys.stdin)["result"]; print(r.get("epoch_blocks", 3600))' 2>/dev/null || echo 3600)
at=$((height + epoch + 120))
python3 -c "import json,sys; json.dump({'chain_id': $chain, 'protocol': $P, 'activate_at': $at, 'releases': [], 'notes': sys.argv[1]}, open('$S/upgrade.json','w'))" "$NOTES"

parts=()
for i in 1 2 3; do
  "$A" upgrade-sign --data "$T/$i" --network "$NET" "$S/upgrade.json" > "$S/partial$i.json"
  parts+=("$S/partial$i.json")
done
"$A" upgrade-combine --network "$NET" "${parts[@]}" > "$S/signed.json"
"$A" upgrade-verify --network "$NET" "$S/signed.json"
for d in "$T"/[0-9]*; do
  mkdir -p "$d/upgrades"
  cp "$S/signed.json" "$d/upgrades/protocol-$P.json"
done
echo "protocol $P signed; activates at block $at (now $height, chain $chain)"
