#!/usr/bin/env bash
# Roll a new node release onto this Mac's testnet validators without stopping
# the chain: build (with the pinned proving program), install the node and its
# proving sidecar, then restart one validator at a time and wait until it is
# following again before touching the next (the other three keep quorum).
#   scripts/testnet-upgrade.sh
# Chain data and keys stay. Protocol changes then activate on chain by a
# committee-signed upgrade (scripts/testnet-activate.sh), not here.
set -euo pipefail
cd "$(dirname "$0")/.."
T=${AETHER_TESTNET:-$HOME/aether-testnet}
export PATH="$HOME/.cargo/bin:$PATH"
# Release binary: same bytes wherever the checkout lives (gap G5).
. scripts/repro-env.sh
aether_repro_rustflags
aether_repro_link_flags
N=$(ls -d "$T"/[0-9]* 2>/dev/null | wc -l | tr -d ' ')
[ "$N" -ge 4 ] || { echo "need at least 4 validator dirs in $T"; exit 1; }

AETHER_PROVER_PROGRAM=$(scripts/prover-program.sh)
export AETHER_PROVER_PROGRAM
cargo build -q --release --locked -p aether-node
# Same bytes as a rebuild elsewhere: the linker's UUID follows the build
# directory, so rewrite it from the code before the binary is installed.
aether_repro_fix_uuid target/release/aether
echo "proving program $AETHER_PROVER_PROGRAM"

height() { curl -s -m 2 "localhost:$((8600 + $1))" -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"aether_status","params":[]}' | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["height"])' 2>/dev/null || echo 0; }

# The new binaries go in beside the old ones; each validator picks them up on restart.
cp target/release/aether "$T/bin/aether.new" && mv -f "$T/bin/aether.new" "$T/bin/aether"
cp apps/prover/target/release/aether-prover "$T/bin/aether-prover.new" && mv -f "$T/bin/aether-prover.new" "$T/bin/aether-prover"
codesign --force --options runtime --timestamp --sign "${SIGN_IDENTITY:-Developer ID Application: Pipln (45WU468FZE)}" "$T/bin/aether" "$T/bin/aether-prover" >/dev/null

for i in $(seq 1 "$N"); do
  before=$(height 1)
  [ "$i" = 1 ] && before=$(height 2)
  launchctl kickstart -k "gui/$(id -u)/com.pipln.aether.testnet.v$i"
  # Back once it serves again and has moved past the height it restarted at.
  for _ in $(seq 1 120); do
    sleep 2
    h=$(height "$i")
    [ "$h" -gt "$before" ] && break
  done
  echo "validator $i: height $(height "$i")"
  [ "$(height "$i")" -gt "$before" ] || { echo "validator $i did not come back; stopping the rollout"; exit 1; }
done
echo "rolled out to $N validators"
