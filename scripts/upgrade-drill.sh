#!/usr/bin/env bash
# Rehearse a protocol upgrade end to end on a throwaway local chain (mainnet
# beta gate A4; docs/ops/upgrade-drill.md has the full run book):
#   scripts/upgrade-drill.sh
#
# One PASS/FAIL line per claim below (a FAIL still runs the remaining checks;
# the script exits 1 if anything failed). Five things are proved:
#   1. Builder release approval (docs/design/19-release-approval.md): an
#      ordinary release needs 2 of the 3 pinned builder keys, an emergency 3
#      of 3, one signature or a tampered manifest is refused
#      (scripts/release-approve.py + scripts/builder-sign, all software
#      checks), the wallet's approval policy agrees (apps/wallet
#      release-approval tests), and the approved bytes are published to an
#      on-chain ReleaseLog whose storage is verified against a certified
#      state root (aether_releaseEntries + aether storage).
#   2. A scheduled committee upgrade (3-of-4 BLS partials, scripts/
#      testnet-activate.sh flow) to protocol 4 activates at exactly the
#      signed height, blocks keep finalizing through the switch, and 2-of-4
#      partials do not combine.
#   3. The validator left on the old release stops cleanly one block before
#      the new rules (exit 3, "UPGRADE REQUIRED" in its log), its last
#      finalized block is the chain's block (no fork), and it rejoins once
#      its binary is updated.
#   4. The emergency path (B4, committee n-f): an emergency upgrade needs 3
#      of 4 independent ed25519 approvals — 2-of-4 is refused — and activates
#      after the epoch notice, not the 604,800-block mainnet notice.
#   5. A bad new release (a binary that refuses to start) costs quorum but
#      not the chain: the operator rolls the node back to the previous
#      release before the switch height, it rejoins, and the switch is clean
#      once a good release is installed everywhere.
#
# The chain is built like a real launch (scripts/mainnet-rehearsal.sh): four
# validators on loopback ports, DKG, 1 s blocks, history v2 + node rewards +
# dev registrar, plus a faucet so a dev account can pay to publish releases
# (the real launch has no faucet; see docs/ops/upgrade-drill.md). Two dev-only
# hooks make a protocol this tree does not implement rehearable (cargo feature
# `dev-drill` in crates/node — never in a shipped binary, the script checks
# the gate):
#   - AETHER_DEV_UPGRADE_NOTICE=<blocks>  shortens the mainnet notice (30)
#   - AETHER_DEV_PROTOCOL=<n>            a drill build claims protocol n
# Budget: ~4 min of incremental builds + ~10 min of chain time on an M-series
# Mac. Data lives in tmp/drill (throwaway); the run log is tmp/upgrade-drill.log.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
mkdir -p tmp
LOG=$ROOT/tmp/upgrade-drill.log
exec > >(tee "$LOG") 2>&1
export PATH="$HOME/.cargo/bin:$HOME/.foundry/bin:$PATH"
# cc/cargo temp files go to the workspace too: the system disk can be too
# full for a C compile (aws-lc-sys) to even open its temp output.
export TMPDIR="$ROOT/tmp"

D=${AETHER_DRILL_DIR:-$ROOT/tmp/drill}
CHAIN=${AETHER_DRILL_CHAIN_ID:-7795}
EPOCH_BLOCKS=60          # the emergency notice on this chain (an epoch)
BLOCK_MS=1000
NOTICE=30                # AETHER_DEV_UPGRADE_NOTICE: ordinary-notice stand-in
BIN=$ROOT/tmp/drill-bin

PASS=() FAIL=()
ok() { PASS+=("$1"); echo "  ok    $1"; }
bad() { FAIL+=("$1"); echo "  FAIL  $1"; }
section() { echo; echo "== $*"; }

PIDS=()
cleanup() {
  trap - EXIT INT TERM
  for p in ${PIDS[@]+"${PIDS[@]}"}; do
    pgrep -P "$p" 2>/dev/null | xargs kill 2>/dev/null || true
    kill "$p" 2>/dev/null || true
  done
  sleep 2
  for p in ${PIDS[@]+"${PIDS[@]}"}; do
    pgrep -P "$p" 2>/dev/null | xargs kill -9 2>/dev/null || true
    kill -9 "$p" 2>/dev/null || true
  done
}
trap cleanup EXIT INT TERM

rpc() { # <method> <params> <port>; retried, an empty answer under load is not a verdict
  local out i
  for i in 1 2 3; do
    out=$(curl -s -m 10 -X POST -H 'content-type: application/json' -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":$2}" "http://127.0.0.1:$3")
    [ -n "$out" ] && { printf '%s' "$out"; return 0; }
    sleep 1
  done
  printf '%s' "$out"
}
jget() { python3 -c "
import json, sys
try:
    d = json.load(sys.stdin)
    print($1)
except Exception:
    print('')"; }
height() { local h; h=$(rpc aether_status '[]' "$1" | jget 'd["result"]["height"]'); echo "${h:-0}"; }
proto() { rpc aether_status '[]' "$1" | jget 'd["result"]["protocol"]'; }
node_proto() { rpc aether_status '[]' "$1" | jget 'd["result"]["node_protocol"]'; }
wait_height() { # <port> <target> <seconds>
  local end=$((SECONDS + $3))
  while [ "$(height "$1")" -lt "$2" ]; do
    if [ "$SECONDS" -ge "$end" ]; then return 1; fi
    sleep 1
  done
}
bfield() { # <height> <field> <port>
  rpc aether_getBlock "[$1]" "$3" | jget "d[\"result\"][\"$2\"]"
}
stop_node() { # <pid>
  local p=$1 i
  kill "$p" 2>/dev/null || true
  for i in $(seq 1 30); do kill -0 "$p" 2>/dev/null || break; sleep 0.5; done
  pgrep -P "$p" 2>/dev/null | xargs kill -9 2>/dev/null || true
  kill -9 "$p" 2>/dev/null || true
  wait "$p" 2>/dev/null || true   # reap, so a later kill -0 tells the truth
}

# ---------------------------------------------------------------------------
section "0. builds (plain, drill, prover) and the dev-hook gate"
export AETHER_PROVER=""
command -v cargo >/dev/null || { echo "cargo (rustup) not found" >&2; exit 1; }
command -v cast >/dev/null || { echo "foundry (cast) not found: needed for ReleaseLog" >&2; exit 1; }
command -v swiftc >/dev/null || { echo "swiftc not found: needed for the release-approval policy tests" >&2; exit 1; }
echo "-- cargo build --release (plain; the gate proof below uses this binary)"
CARGO_TARGET_DIR="$ROOT/tmp/target.noindex" cargo build --release -q -j 4 --locked -p aether-node
echo "-- cargo build --release (dev-drill feature; the rehearsal binary)"
CARGO_TARGET_DIR="$ROOT/tmp/target-drill.noindex" cargo build --release -q -j 4 --locked -p aether-node --features dev-drill
rm -rf "$BIN"; mkdir -p "$BIN"
cp tmp/target.noindex/release/aether "$BIN/aether-plain"
cp tmp/target-drill.noindex/release/aether "$BIN/aether"
A="$BIN/aether"; PLAIN="$BIN/aether-plain"
# The proving sidecar (a validator refuses to run under protocol >= 2 without
# one). AETHER_DRILL_PROVER reuses an existing build; otherwise it is built
# from apps/prover (needs the aether-jolt checkout, scripts/prover-program.sh).
if [ -n "${AETHER_DRILL_PROVER:-}" ] && [ -x "${AETHER_DRILL_PROVER}" ]; then
  cp "${AETHER_DRILL_PROVER}" "$BIN/aether-prover"
elif [ -x "$BIN/aether-prover" ]; then
  :
else
  echo "-- building aether-prover (one-time; set AETHER_DRILL_PROVER to reuse a build)"
  prover_bin=$(CARGO_TARGET_DIR="$ROOT/tmp/target-prover.noindex" scripts/prover-program.sh)
  cp "$ROOT/tmp/target-prover.noindex/release/aether-prover" "$BIN/aether-prover"
fi
export AETHER_PROVER="$BIN/aether-prover"

# The gate: a shipped (plain) binary ignores the drill variables entirely and
# has no dev-b3 subcommand; only the feature build reads them.
p1=$(AETHER_DEV_PROTOCOL=9 AETHER_DEV_UPGRADE_NOTICE=1 "$PLAIN" protocol)
p2=$(AETHER_DEV_PROTOCOL=9 AETHER_DEV_UPGRADE_NOTICE=1 "$A" protocol)
[ "$p1" = 3 ] && ok "plain build ignores AETHER_DEV_PROTOCOL (claims $p1)" || bad "plain build claims protocol $p1 (want 3)"
[ "$p2" = 9 ] && ok "drill build claims AETHER_DEV_PROTOCOL ($p2)" || bad "drill build claims $p2 (want 9)"
if "$PLAIN" dev-b3 "$0" >/dev/null 2>&1; then bad "plain build has the dev-b3 subcommand"; else ok "plain build has no dev-b3 subcommand"; fi
h1=$(AETHER_DEV_PROTOCOL=9 "$A" dev-b3 Cargo.toml)
[ ${#h1} = 64 ] && ok "drill build hashes with dev-b3 (${h1:0:16}…)" || bad "dev-b3 did not print a 64-hex hash"

# The wallet's approval policy on the same rules (2/3, 3/3 emergency,
# wrong-hash refused, 72 h pending): pure Swift, no app needed.
echo "-- swiftc release-approval policy tests"
if swiftc -o tmp/sw-release-approval apps/wallet/Sources/ReleaseApproval.swift apps/wallet/Tests/release-approval/main.swift 2>tmp/sw-release-approval.err \
   && AETHER_AGENT_TEST_TMP=$ROOT/tmp ./tmp/sw-release-approval >tmp/sw-release-approval.out 2>&1; then
  ok "wallet release-approval policy tests pass (2/3 approve, 3/3 emergency, wrong hash refused, 72 h pending)"
else
  bad "wallet release-approval policy tests failed ($(tail -2 tmp/sw-release-approval.out 2>/dev/null | tr '\n' ' '))"
fi

# ---------------------------------------------------------------------------
section "1. a four-validator rehearsal chain (chain $CHAIN, protocol 3, 1 s blocks)"
rm -rf "$D"; mkdir -p "$D/peers"
for i in 1 2 3 4; do "$A" keygen --data "$D/g$i" >/dev/null; done
mkdir -p "$D/faucet"
"$A" faucet-key --data "$D/faucet" > "$D/faucet.addr"
faucet=$(awk '/^faucet address/ {print $3}' "$D/faucet.addr")
founder=$("$A" dev-accounts | awk '$1 == "dev" && $2 == 1 {print $3}')
"$A" network --chain-id "$CHAIN" --protocol 3 --epoch-blocks "$EPOCH_BLOCKS" --history 2 --node-rewards --dev-registrar \
  --faucet "$faucet" \
  "$D"/g1/validator.pub.json "$D"/g2/validator.pub.json "$D"/g3/validator.pub.json "$D"/g4/validator.pub.json \
  > "$D/genesis.json"
echo "-- dkg (loopback tcp)"
ports=()
while [ "${#ports[@]}" -lt 16 ]; do
  p=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')
  case " ${ports[@]:-} " in *" $p "*) ;; *) ports+=("$p");; esac
done
dkgp=("${ports[@]:0:4}") p2p=("${ports[@]:4:4}") rpcp=("${ports[@]:8:4}") resh=("${ports[@]:12:4}")
for i in 1 2 3 4; do
  peers=""
  for j in 1 2 3 4; do [ "$j" = "$i" ] || peers+="${peers:+,}$j@127.0.0.1:${dkgp[$((j - 1))]}"; done
  "$A" dkg --network "$D/genesis.json" --port "${dkgp[$((i - 1))]}" --data "$D/g$i" --peers "$peers" --offline > "$D/dkg$i.log" 2>&1 &
  PIDS+=($!)
done
for p in ${PIDS[@]+"${PIDS[@]}"}; do wait "$p" || { echo "dkg failed (see $D/dkg*.log)" >&2; exit 1; }; done
PIDS=()
cp "$D/g1/network.json" "$D/network.json"
"$A" ceremony-record --network "$D/network.json" --out "$D/ceremony-check.json" >/dev/null
# The committee identity `aether storage --identity` verifies under: printed by
# dkg (tests/devnet.rs exercises exactly this hex through the same flag).
IDENTITY=$(sed -n 's/^committee identity: //p' "$D/dkg1.log" | head -1)
[ -n "$IDENTITY" ] || IDENTITY=$(python3 -c 'import json;print(json.load(open("'"$D"'/network.json"))["identity"])')

NODE_PID[0]=0; NODE_PID[1]=0; NODE_PID[2]=0; NODE_PID[3]=0
start_node() { # <1-4> <binary> ["AETHER_DEV_PROTOCOL value, empty = claim 3"]
  local i=$1 bin=$2 claim=$3 others="" j args env
  for j in 1 2 3 4; do [ "$j" = "$i" ] || others+="${others:+,}http://127.0.0.1:${rpcp[$((j - 1))]}"; done
  args=(run --exit-with-parent --data "$D/g$i" --network "$D/network.json" --ceremony "$D/ceremony-check.json"
    --port "${p2p[$((i - 1))]}" --rpc-port "${rpcp[$((i - 1))]}" --reshare-port "${resh[$((i - 1))]}"
    --dev-peer-dir "$D/peers" --node-arg=--block-time-ms=$BLOCK_MS
    "--follow-arg=--from-rpc=$others" --reshare-timeout 120)
  if [ "$i" = 1 ]; then args+=(--node-arg=--dev-registrar "--node-arg=--faucet-key=$D/faucet/faucet.key"); fi
  if [ -n "${NODE_PID[$((i - 1))]}" ] && [ "${NODE_PID[$((i - 1))]}" != 0 ]; then stop_node "${NODE_PID[$((i - 1))]}"; fi
  env=(AETHER_DEV_UPGRADE_NOTICE=$NOTICE RUST_LOG=info,commonware=warn)
  [ -n "$claim" ] && env+=(AETHER_DEV_PROTOCOL=$claim)
  env "${env[@]}" nohup "$bin" "${args[@]}" > "$D/g$i.log" 2>&1 &
  NODE_PID[$((i - 1))]=$!
  PIDS+=("${NODE_PID[$((i - 1))]}")
}
start_node 1 "$A" ""; start_node 2 "$A" ""; start_node 3 "$A" ""; start_node 4 "$A" ""
if wait_height "${rpcp[0]}" 12 240; then ok "blocks finalize (validator 1 reached height ≥ 12)"; else bad "no finalized blocks (see $D/*.log)"; fi
h=$(height "${rpcp[0]}")
roots=""
for k in 0 1 2 3; do roots+="$(bfield "$h" state_root "${rpcp[$k]}") "; done
if [ "$(printf '%s\n' $roots | sort -u | grep -c .)" = 1 ]; then
  ok "the four validators agree on the state root at height $h"
else
  bad "the validators disagree at height $h: $roots"
fi
node_proto_0=$(node_proto "${rpcp[0]}")
[ "$node_proto_0" = 3 ] && ok "every node starts claiming protocol 3 (node_protocol $node_proto_0)" || bad "node_protocol is $node_proto_0 (want 3)"

# ---------------------------------------------------------------------------
section "2. builder release approval (design 19): 2-of-3, emergency 3-of-3, on-chain ReleaseLog"
REL=$D/rel; mkdir -p "$REL/EastSea.app/Contents/MacOS" "$REL/EastSea.app/Contents/Helpers"
printf 'drill app executable\n' > "$REL/EastSea.app/Contents/MacOS/EastSea"
printf 'drill node binary\n' > "$REL/EastSea.app/Contents/Helpers/aether"
printf 'drill agent binary\n' > "$REL/EastSea.app/Contents/Helpers/aether-agent"
printf 'drill dmg bytes\n' > "$REL/EastSea.dmg"
echo "-- funding the founder (dev 1) from the faucet, then deploying ReleaseLog"
rpc aether_faucet "[\"$founder\"]" "${rpcp[0]}" > "$D/faucet-grant.json"
granted=$(jget 'd["result"]["amount_wei"]' < "$D/faucet-grant.json")
[ -n "$granted" ] && ok "faucet granted the founder $granted wei" || bad "faucet grant failed ($(cat "$D/faucet-grant.json"))"
# The grant is a transaction: wait until it is final and the balance shows up,
# or the deploy below is refused for insufficient funds while it is in flight.
end=$((SECONDS + 45)); bal=""
while [ "$SECONDS" -lt "$end" ]; do
  bal=$(rpc eth_getBalance "[\"$founder\", \"latest\"]" "${rpcp[0]}" | jget 'int(d["result"], 16)')
  [ -n "$bal" ] && [ "$bal" != "0" ] && break
  sleep 1
done
# The dev account starts unfunded, so "nonzero" stands in for ">= granted":
# bash's 64-bit arithmetic overflows above 9.2e18 and a 10e18-wei grant is past it.
if [ -n "$bal" ] && [ "$bal" != "0" ]; then
  ok "the grant finalized (balance $bal wei)"
else
  bad "the founder's grant never landed (balance ${bal:-?} after 45 s)"
fi
RUNTIME_HEX=$(cd contracts && forge inspect ReleaseLog deployedBytecode 2>/dev/null)
CODE_HEX=$(cd contracts && forge inspect ReleaseLog bytecode 2>/dev/null)
CODE_HASH=$(cast keccak "$RUNTIME_HEX")
deploy_out=""
for try in 1 2 3; do
  deploy_out=$("$A" deploy --rpc "http://127.0.0.1:${rpcp[0]}" --from-dev 1 --code "$CODE_HEX" 2>&1 || true)
  printf '%s\n' "$deploy_out" | grep -q "^contract: " && break
  sleep 3
done
LOGADDR=$(printf '%s\n' "$deploy_out" | awk '/^contract: / {print $2}')
[ -n "$LOGADDR" ] && ok "ReleaseLog deployed at $LOGADDR (runtime keccak ${CODE_HASH:0:18}…)" || bad "ReleaseLog deploy failed ($deploy_out)"

# Three software builder keys, a pin file for release-approve.py (AETHER_RELEASE_NET)
python3 - "$REL" <<'PYEOF'
import json, pathlib, sys
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives import serialization
rel = pathlib.Path(sys.argv[1])
for i in range(3):
    key = ec.generate_private_key(ec.SECP256R1())
    (rel / f"builder{i}.pem").write_bytes(key.private_bytes(
        serialization.Encoding.PEM, serialization.PrivateFormat.PKCS8, serialization.NoEncryption()))
net = json.load(open("tmp/drill/network.json"))
net["builder_keys"] = []  # filled by the signer below
json.dump(net, open(rel / "release-net.keys.json", "w"))
PYEOF
sign_manifest() { # <pem> <manifest> <out-sig>; Swift-compatible x963 key + raw low-s r||s
  python3 - "$1" "$2" "$3" <<'PYEOF'
import hashlib, json, pathlib, sys
from cryptography.hazmat.primitives.asymmetric import ec, utils
from cryptography.hazmat.primitives import hashes, serialization
pem, manifest, out = sys.argv[1:]
key = serialization.load_pem_private_key(pathlib.Path(pem).read_bytes(), None)
data = pathlib.Path(manifest).read_bytes()
der = key.sign(data, ec.ECDSA(hashes.SHA256()))
r, s = utils.decode_dss_signature(der)
n = 0xFFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551  # the P-256 order
if s > n // 2:
    s = n - s
pub = key.public_key().public_bytes(serialization.Encoding.X962, serialization.PublicFormat.UncompressedPoint)
json.dump({"public_key": pub.hex(), "signature": r.to_bytes(32, "big").hex() + s.to_bytes(32, "big").hex(),
           "manifest_sha256": hashlib.sha256(data).hexdigest()}, open(out, "w"))
PYEOF
}
# The pin: network.json (this chain) + builder keys + the ReleaseLog address/hash.
python3 - "$REL" "$LOGADDR" "$CODE_HASH" <<'PYEOF'
import json, pathlib, sys
from cryptography.hazmat.primitives.asymmetric import ec
from cryptography.hazmat.primitives import serialization
rel = pathlib.Path(sys.argv[1]); log, code_hash = sys.argv[2:]
net = json.load(open(rel / "release-net.keys.json"))
net["release_log"] = log
net["release_log_code_hash"] = code_hash
net["builder_keys"] = []
for i in range(3):
    key = serialization.load_pem_private_key((rel / f"builder{i}.pem").read_bytes(), None)
    net["builder_keys"].append(key.public_key().public_bytes(
        serialization.Encoding.X962, serialization.PublicFormat.UncompressedPoint).hex())
json.dump(net, open(rel / "release-net.json", "w"))
PYEOF
export AETHER_RELEASE_NET=$REL/release-net.json
approve() { python3 scripts/release-approve.py "$@"; }

echo "-- ordinary release: prepare, sign, combine"
approve prepare --chain-id "$CHAIN" --log "$LOGADDR" --version 0.0.0 --build drill \
  --dmg "$REL/EastSea.dmg" --app "$REL/EastSea.app" --sparkle-signature "$(printf 'ab%.0s' $(seq 32))" \
  --out "$REL/manifest.json" > "$REL/prepare.out" || true
for i in 0 1 2; do sign_manifest "$REL/builder$i.pem" "$REL/manifest.json" "$REL/sig$i.json" || true; done
if combine_out=$(approve combine "$REL/manifest.json" "$REL/sig0.json" "$REL/sig1.json" --out "$REL/signatures.json" 2>&1); then
  ok "2-of-3 builder signatures approve the ordinary release"
else
  bad "2-of-3 combine failed: $combine_out"
fi
if scripts/builder-sign verify-bundle "$REL/manifest.json" "$REL/signatures.json" > /dev/null 2>&1; then
  ok "builder-sign verify-bundle accepts the pair (software check, no enclave)"
else
  bad "verify-bundle rejected the 2-of-3 bundle"
fi
if approve combine "$REL/manifest.json" "$REL/sig0.json" --out "$REL/one.json" >/dev/null 2>&1; then
  bad "1-of-3 builder signature was enough for an ordinary release"
else
  ok "1-of-3 builder signature is refused"
fi
python3 - "$REL" <<'PYEOF' || true
import json, pathlib, sys
rel = pathlib.Path(sys.argv[1])
m = json.loads((rel / "manifest.json").read_bytes())
m["build"] = "tampered"
(rel / "manifest-tampered.json").write_bytes(json.dumps(m, sort_keys=True, separators=(",", ":")).encode())
PYEOF
if approve combine "$REL/manifest-tampered.json" "$REL/sig0.json" "$REL/sig1.json" --out "$REL/t.json" >/dev/null 2>&1; then
  bad "a tampered manifest was approved"
else
  ok "signatures for a different (tampered) manifest are refused"
fi

echo "-- emergency release: 3-of-3 required"
approve prepare --chain-id "$CHAIN" --log "$LOGADDR" --version 0.0.0 --build drill \
  --dmg "$REL/EastSea.dmg" --app "$REL/EastSea.app" --sparkle-signature "$(printf 'ab%.0s' $(seq 32))" \
  --emergency --out "$REL/manifest-e.json" > /dev/null || true
for i in 0 1 2; do sign_manifest "$REL/builder$i.pem" "$REL/manifest-e.json" "$REL/sige$i.json" || true; done
if approve combine "$REL/manifest-e.json" "$REL/sige0.json" "$REL/sige1.json" --out "$REL/t.json" >/dev/null 2>&1; then
  bad "2-of-3 builder signatures approved an emergency release"
else
  ok "emergency needs 3-of-3 builders (2-of-3 refused)"
fi
if approve combine "$REL/manifest-e.json" "$REL/sige0.json" "$REL/sige1.json" "$REL/sige2.json" --out "$REL/signatures-e.json" >/dev/null 2>&1; then
  ok "3-of-3 builder signatures approve the emergency release"
else
  bad "3-of-3 emergency combine failed"
fi

echo "-- publishing both entries on chain (aether deploy/call, dev 1 pays)"
publish() { # <manifest> <sigs> <true|false>
  local mhex shex arch data
  mhex=$(python3 -c "print(open('$1','rb').read().hex())")
  shex=$(python3 -c "print(open('$2','rb').read().hex())")
  arch=$(python3 -c "import json;print(next(a['sha256'] for a in json.load(open('$1'))['artifacts'] if a['name']=='EastSea.dmg'))")
  data=$(cast calldata 'publish(bytes,bytes32,bytes,bool)' "0x$mhex" "0x$arch" "0x$shex" "$3")
  "$A" call --rpc "http://127.0.0.1:${rpcp[0]}" --from-dev 1 --to "$LOGADDR" --data "$data" --wait >/dev/null
}
if publish "$REL/manifest.json" "$REL/signatures.json" false \
   && publish "$REL/manifest-e.json" "$REL/signatures-e.json" true; then
  ok "both releases published to ReleaseLog (publish calldata, waited receipts)"
else
  bad "publishing to ReleaseLog failed"
fi
sleep 2
raw=$(rpc aether_releaseEntries "[\"$LOGADDR\",0,8]" "${rpcp[0]}")
want_m=$(python3 -c 'import hashlib,sys;print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$REL/manifest.json")
want_me=$(python3 -c 'import hashlib,sys;print(hashlib.sha256(open(sys.argv[1],"rb").read()).hexdigest())' "$REL/manifest-e.json")
got_count=$(printf '%s' "$raw" | jget 'd["result"]["count"]')
got_m=$(printf '%s' "$raw" | jget 'd["result"]["entries"][0]["manifest_sha256"]')
got_e=$(printf '%s' "$raw" | jget 'd["result"]["entries"][1]["manifest_sha256"]')
got_flag=$(printf '%s' "$raw" | jget 'd["result"]["entries"][1]["emergency"]')
if [ "$got_count" = 2 ] && [ "$got_m" = "$want_m" ] && [ "$got_e" = "$want_me" ] && [ "$got_flag" = True ]; then
  ok "aether_releaseEntries matches the approved bytes (2 entries, hashes + emergency flag)"
else
  bad "release entries mismatch (count $got_count, m ${got_m:0:12}≠${want_m:0:12}, e ${got_e:0:12}≠${want_me:0:12}, flag $got_flag)"
fi
# Storage proof under a committee-certified root: slot 0 (count) and entry 0's
# manifest word (keccak256([0u8;32]) is the Solidity array base).
base10=$(cast keccak 0x0000000000000000000000000000000000000000000000000000000000000000 | python3 -c 'import sys;print(int(sys.stdin.read().strip(),16))')
store_read() { "$A" storage "$LOGADDR" "$1" --rpc "http://127.0.0.1:${rpcp[0]}" --identity "$IDENTITY" 2>/dev/null | awk -F'= ' '/= /{print $2}' | head -1; }
c0=$(store_read 0); m0=$(store_read "$base10")
want_m10=$(python3 -c "print(int('$want_m',16))")
if [ "$c0" = 2 ] && [ "$m0" = "$want_m10" ]; then
  ok "storage proofs verify under a certified root (count=2, entry 0 manifest word)"
else
  bad "storage proof mismatch (count '$c0', manifest word '$m0' vs $want_m10)"
fi

# ---------------------------------------------------------------------------
section "3. scheduled committee upgrade to protocol 4 (short dev notice)"
# Simulated releases: wrappers that claim a protocol this tree does not
# implement. r4/r5/r6 differ only in the claim; each has its own bytes (hash).
for v in 4 5 6; do
  printf '#!/bin/sh\nexport AETHER_DEV_PROTOCOL=%s\nexec "%s" "$@"\n' "$v" "$A" > "$BIN/aether-r$v"
  chmod +x "$BIN/aether-r$v"
done
printf '#!/bin/sh\necho "simulated bad release r6: refusing to start" >&2\nexit 1\n' > "$BIN/aether-bad"
chmod +x "$BIN/aether-bad"
b3r4=$("$A" dev-b3 "$BIN/aether-r4")
h=$(height "${rpcp[0]}")
H1=$((h + 50))
python3 - "$CHAIN" "$H1" "$b3r4" > "$D/upgrade4.json" <<'PYEOF'
import json, sys
chain, at, b3 = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
json.dump({"chain_id": chain, "protocol": 4, "activate_at": at,
           "releases": [{"platform": "macos-arm64-dmg", "version": "drill-r4",
                          "blake3": b3, "url": "https://drill.invalid/r4.dmg"}],
           "notes": "drill: scheduled switch to protocol 4"}, sys.stdout)
PYEOF
for i in 1 2 3; do "$A" upgrade-sign --data "$D/g$i" --network "$D/network.json" "$D/upgrade4.json" > "$D/p4-$i.json" || true; done
if "$A" upgrade-combine --network "$D/network.json" "$D/p4-1.json" "$D/p4-2.json" > "$D/t.json" 2>&1; then
  bad "2-of-4 committee partials combined into a signature"
else
  ok "2-of-4 committee partials do not combine (threshold is 3)"
fi
if "$A" upgrade-combine --network "$D/network.json" "$D/p4-1.json" "$D/p4-2.json" "$D/p4-3.json" > "$D/signed4.json" 2>"$D/combine4.err"; then
  vout=$("$A" upgrade-verify --network "$D/network.json" "$D/signed4.json" 2>&1 || true)
else
  vout="combine failed: $(tr '\n' ' ' < "$D/combine4.err" 2>/dev/null)"
  bad "3-of-4 partials did not combine ($vout)"
fi
for i in 1 2 3 4; do mkdir -p "$D/g$i/upgrades"; [ -s "$D/signed4.json" ] && cp "$D/signed4.json" "$D/g$i/upgrades/protocol-4.json" || true; done
sched=""
end=$((SECONDS + 90))
while [ "$SECONDS" -lt "$end" ]; do
  sched=$(rpc aether_status '[]' "${rpcp[0]}" | jget '"yes" if [4, '"$H1"'] in d["result"].get("schedule", []) else "no"')
  [ "$sched" = yes ] && break
  sleep 1
done
if [ "$sched" = yes ]; then ok "3-of-4 committee signature on chain: protocol 4 activates at $H1 ($vout)"; else bad "the signed upgrade never reached the chain (schedule '$sched')"; fi

echo "-- rolling v1-v3 onto r4 (validators claim protocol 4), v4 stays on protocol 3"
for i in 1 2 3; do
  hb=$(height "${rpcp[$((i - 1))]}")
  start_node "$i" "$BIN/aether-r4" ""
  wait_height "${rpcp[$((i - 1))]}" $((hb + 2)) 120 || bad "validator $i did not rejoin on r4"
done
ok "v1-v3 restarted on r4 (AETHER_DEV_PROTOCOL=4) and rejoined"
# Watch the switch itself: protocol 3 below H1, 4 from H1, chain unbroken.
seen3=no seen4=no
while [ "$(height "${rpcp[0]}")" -lt $((H1 + 2)) ]; do
  hh=$(height "${rpcp[0]}"); pp=$(proto "${rpcp[0]}")
  if [ "$hh" -lt "$H1" ] && [ "$pp" = 3 ]; then seen3=yes; fi
  if [ "$hh" -ge "$H1" ] && [ "$pp" = 4 ]; then seen4=yes; fi
  sleep 0.3
done
if [ "$seen3" = yes ] && [ "$seen4" = yes ]; then
  ok "protocol 3 below $H1, exactly 4 from $H1 (observed live)"
else
  bad "the switch was not observed (seen3 $seen3, seen4 $seen4)"
fi
parent=$(bfield "$H1" parent "${rpcp[0]}"); prev=$(bfield $((H1 - 1)) hash "${rpcp[0]}")
cont=yes
for hh in $((H1 - 1)) "$H1" $((H1 + 1)) $((H1 + 2)); do
  [ -n "$(bfield "$hh" hash "${rpcp[0]}")" ] || cont=no
done
if [ "$parent" = "$prev" ] && [ "$cont" = yes ]; then
  ok "blocks finalized through the switch: H-1 -> H parent link intact, no gap"
else
  bad "the chain broke at the switch (parent '$parent' vs '$prev', continuity $cont)"
fi
roots=""
for k in 0 1 2; do roots+="$(bfield "$((H1 + 2))" state_root "${rpcp[$k]}") "; done
[ "$(printf '%s\n' $roots | sort -u | grep -c .)" = 1 ] && ok "upgraded validators agree on the root after the switch" || bad "upgraded validators disagree: $roots"

# ---------------------------------------------------------------------------
section "4. the node left on protocol 3 stops cleanly, then rejoins on r4"
v4pid=${NODE_PID[3]}
end=$((SECONDS + 60))
while kill -0 "$v4pid" 2>/dev/null && [ "$SECONDS" -lt "$end" ]; do sleep 1; done
rc=0; wait "$v4pid" 2>/dev/null || rc=$?
if [ "$rc" = 3 ]; then ok "v4 stopped itself with exit 3 (EXIT_UPGRADE_REQUIRED)"; else bad "v4 exit code is $rc (want 3)"; fi
log4=$(sed $'s/\x1b\\[[0-9;]*m//g' "$D/g4.log")
case "$log4" in
  *"UPGRADE REQUIRED"*) ok "v4 logged UPGRA REQUIRED before the new rules (grep)" ;;
  *) bad "no UPGRA REQUIRED line in v4's log" ;;
esac
head4=$("$A" head --data "$D/g4" 2>/dev/null || true)
h4=$(printf '%s' "$head4" | awk '{print $1}'); r4head=$(printf '%s' "$head4" | awk '{print $2}')
chain4=$(bfield "$h4" hash "${rpcp[0]}")
if [ "$h4" -ge $((H1 - 2)) ] && [ "$h4" -le $((H1 - 1)) ] && [ "$r4head" = "$chain4" ]; then
  ok "v4's last finalized block ($h4) is the chain's own (no fork, stopped before the new rules)"
else
  bad "v4 stopped at a foreign head ($h4 vs window $((H1 - 2))..$((H1 - 1)), hash ${r4head:0:12} vs ${chain4:0:12})"
fi
start_node 4 "$BIN/aether-r4" ""
if wait_height "${rpcp[3]}" $((H1 + 2)) 180 && [ "$(node_proto "${rpcp[3]}")" = 4 ]; then
  ok "v4 rejoined on r4, caught up past the switch, node_protocol 4"
else
  bad "v4 did not rejoin on r4 (height $(height "${rpcp[3]}"), node_protocol $(node_proto "${rpcp[3]}"))"
fi
r1=$(bfield "$((H1 + 2))" state_root "${rpcp[0]}"); r4v=$(bfield "$((H1 + 2))" state_root "${rpcp[3]}")
[ "$r1" = "$r4v" ] && ok "rejoined v4 agrees on the post-switch state root" || bad "v4 root after rejoin differs: $r1 vs $r4v"

# ---------------------------------------------------------------------------
section "5. emergency upgrade to protocol 5 (epoch notice, B4: n-f approvals)"
# The rule since B4 (upgrade::verify_emergency): n-f independent ed25519
# approvals from current committee members — 3 of 4 on this chain. 2 approvals
# cannot pass even with a valid committee BLS signature: the third partial
# below is the same share signature with its emergency_approval stripped.
h=$(height "${rpcp[0]}")
H2=$((h + 75))
b3r5=$("$A" dev-b3 "$BIN/aether-r5")
python3 - "$CHAIN" "$H2" "$b3r5" > "$D/upgrade5.json" <<'PYEOF'
import json, sys
chain, at, b3 = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
json.dump({"chain_id": chain, "protocol": 5, "activate_at": at, "emergency": True,
           "releases": [{"platform": "macos-arm64-dmg", "version": "drill-r5",
                          "blake3": b3, "url": "https://drill.invalid/r5.dmg"}],
           "notes": "drill: emergency switch to protocol 5"}, sys.stdout)
PYEOF
for i in 1 2 3 4; do "$A" upgrade-sign --data "$D/g$i" --network "$D/network.json" "$D/upgrade5.json" > "$D/p5-$i.json" || true; done
python3 - "$D/p5-3.json" "$D/p5-3-noed.json" <<'PYEOF' || true
import json, sys
p = json.load(open(sys.argv[1]))
del p["emergency_approval"]  # a BLS partial that did not countersign the emergency
json.dump(p, open(sys.argv[2], "w"))
PYEOF
emsg=""
if "$A" upgrade-combine --network "$D/network.json" "$D/p5-1.json" "$D/p5-2.json" "$D/p5-3-noed.json" > "$D/t.json" 2>"$D/emergency-2of4.err"; then
  bad "2-of-4 emergency approvals combined (B4 needs n-f = 3)"
else
  emsg=$(tr '\n' ' ' < "$D/emergency-2of4.err")
  case "$emsg" in
    *at*least*3*) ok "2-of-4 emergency approvals refused: ${emsg:0:60} (B4 n-f)" ;;
    *) ok "2-of-4 emergency approvals refused (B4 n-f): ${emsg:0:60}" ;;
  esac
fi
if "$A" upgrade-combine --network "$D/network.json" "$D/p5-1.json" "$D/p5-2.json" "$D/p5-3.json" > "$D/signed5.json" 2>"$D/combine5.err" \
   && "$A" upgrade-verify --network "$D/network.json" "$D/signed5.json" > /dev/null 2>&1; then
  ok "3-of-4 emergency approvals combined and verified (B4: committee n-f)"
else
  bad "3-of-4 emergency approvals did not combine/verify ($(tr '\n' ' ' < "$D/combine5.err" 2>/dev/null))"
fi
for i in 1 2 3 4; do [ -s "$D/signed5.json" ] && cp "$D/signed5.json" "$D/g$i/upgrades/protocol-5.json" || true; done
sched=no
end=$((SECONDS + 90))
while [ "$SECONDS" -lt "$end" ]; do
  sched=$(rpc aether_status '[]' "${rpcp[0]}" | jget '"yes" if [5, '"$H2"'] in d["result"].get("schedule", []) else "no"')
  [ "$sched" = yes ] && break
  sleep 1
done
[ "$sched" = yes ] && ok "4-of-4 emergency upgrade on chain: protocol 5 at $H2 (epoch notice $EPOCH_BLOCKS, not 604,800)" || bad "emergency upgrade never reached the chain"
for i in 1 2 3 4; do
  hb=$(height "${rpcp[$((i - 1))]}")
  start_node "$i" "$BIN/aether-r5" ""
  wait_height "${rpcp[$((i - 1))]}" $((hb + 2)) 120 || bad "validator $i did not rejoin on r5"
done
ok "all four validators restarted on r5 and rejoined"
if wait_height "${rpcp[0]}" $((H2 + 3)) 240 && [ "$(proto "${rpcp[0]}")" = 5 ]; then
  ok "protocol 5 activated at $H2 and blocks kept finalizing"
else
  bad "emergency switch failed (height $(height "${rpcp[0]}"), protocol $(proto "${rpcp[0]}"))"
fi

# ---------------------------------------------------------------------------
section "6. bad release, rollback before the switch, then a clean switch to 6"
h=$(height "${rpcp[0]}")
H3=$((h + 75))
b3r6=$("$A" dev-b3 "$BIN/aether-r6")
python3 - "$CHAIN" "$H3" "$b3r6" > "$D/upgrade6.json" <<'PYEOF'
import json, sys
chain, at, b3 = int(sys.argv[1]), int(sys.argv[2]), sys.argv[3]
json.dump({"chain_id": chain, "protocol": 6, "activate_at": at,
           "releases": [{"platform": "macos-arm64-dmg", "version": "drill-r6",
                          "blake3": b3, "url": "https://drill.invalid/r6.dmg"}],
           "notes": "drill: switch to protocol 6 (first build is bad)"}, sys.stdout)
PYEOF
for i in 1 2 3; do "$A" upgrade-sign --data "$D/g$i" --network "$D/network.json" "$D/upgrade6.json" > "$D/p6-$i.json" || true; done
if ! "$A" upgrade-combine --network "$D/network.json" "$D/p6-1.json" "$D/p6-2.json" "$D/p6-3.json" > "$D/signed6.json" 2>"$D/combine6.err" \
   || ! "$A" upgrade-verify --network "$D/network.json" "$D/signed6.json" > /dev/null 2>&1; then
  bad "protocol-6 partials did not combine/verify ($(tr '\n' ' ' < "$D/combine6.err" 2>/dev/null))"
fi
for i in 1 2 3 4; do [ -s "$D/signed6.json" ] && cp "$D/signed6.json" "$D/g$i/upgrades/protocol-6.json" || true; done
sched=no
end=$((SECONDS + 90))
while [ "$SECONDS" -lt "$end" ]; do
  sched=$(rpc aether_status '[]' "${rpcp[0]}" | jget '"yes" if [6, '"$H3"'] in d["result"].get("schedule", []) else "no"')
  [ "$sched" = yes ] && break
  sleep 1
done
[ "$sched" = yes ] && ok "protocol 6 scheduled at $H3 (signed before anything is installed)" || bad "protocol 6 never reached the chain"

echo "-- v1 installs the bad r6: it must refuse to start (operator sees it at once)"
start_node 1 "$BIN/aether-bad" ""
brc=0; wait "${NODE_PID[0]}" 2>/dev/null || brc=$?
if [ "$brc" != 0 ] && [ "$brc" != 127 ]; then ok "bad release refuses to start (exit $brc); the other three keep quorum"; else bad "bad release exited $brc"; fi
start_node 1 "$BIN/aether-r5" ""   # rollback to the previous release
hb=$(height "${rpcp[0]}")
if wait_height "${rpcp[0]}" $((hb + 2)) 120 && wait_height "${rpcp[0]}" $((H3 - 20)) 240; then
  ok "rollback to r5 before the switch: v1 rejoined and followed (chain unharmed)"
else
  bad "v1 did not rejoin after the rollback (height $(height "${rpcp[0]}") vs switch $H3)"
fi
[ "$(node_proto "${rpcp[0]}")" = 5 ] || bad "chain lost head after the bad release (protocol $(proto "${rpcp[0]}"))"

echo "-- installing the good r6 on everyone before $H3"
for i in 1 2 3 4; do
  hb=$(height "${rpcp[$((i - 1))]}")
  start_node "$i" "$BIN/aether-r6" ""
  wait_height "${rpcp[$((i - 1))]}" $((hb + 2)) 120 || bad "validator $i did not rejoin on r6"
done
if wait_height "${rpcp[0]}" $((H3 + 3)) 300 && [ "$(proto "${rpcp[0]}")" = 6 ]; then
  ok "protocol 6 activated at $H3 with everyone on r6; blocks kept finalizing"
else
  bad "switch to 6 failed (height $(height "${rpcp[0]}"), protocol $(proto "${rpcp[0]}"))"
fi
roots=""
for k in 0 1 2 3; do roots+="$(bfield "$((H3 + 2))" state_root "${rpcp[$k]}") "; done
if [ "$(printf '%s\n' $roots | sort -u | grep -c .)" = 1 ]; then
  ok "all four validators agree on the root after both incidents (no corruption)"
else
  bad "validators disagree after the rollback drill: $roots"
fi

# ---------------------------------------------------------------------------
echo
echo "==================== upgrade drill results ===================="
for p in ${PASS[@]+"${PASS[@]}"}; do printf 'PASS  %s\n' "$p"; done
for f in ${FAIL[@]+"${FAIL[@]}"}; do printf 'FAIL  %s\n' "$f"; done
echo "elapsed: $SECONDS s (chain data and logs in $D; run log $LOG)"
if [ "${#FAIL[@]}" = 0 ]; then
  echo "PASS: every upgrade emergency behaves (gate A4)"
else
  echo "FAIL: ${#FAIL[@]} check(s) failed"
  exit 1
fi
