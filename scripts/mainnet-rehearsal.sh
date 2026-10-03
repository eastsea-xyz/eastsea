#!/usr/bin/env bash
# A throwaway local network with the exact mainnet genesis flags, run and
# checked end to end before the real thing (docs/ops/mainnet-launch.md,
# docs/design/12-launch-plan.md):
#   scripts/mainnet-rehearsal.sh <dir>
# <dir> is a new (or emptied) throwaway directory for keys, chain data and
# logs. Everything runs as plain background processes on localhost ports: no
# launchd, nothing touches the running testnet, and cleanup is trapped, so
# Ctrl-C is safe.
#
# The network: a new chain id, protocol 3 from genesis (proof market, registry
# v2 registration cap, 16-seat growth — no upgrade), history v2, node rewards
# from genesis, the dev registrar (no Apple DeviceCheck call), the founder's
# three reserve keys, no faucet and no premine. Four validators, the three
# reserve keys and two candidate Macs, each only `aether run`. Epochs are 40
# blocks of 500 ms so the rehearsal finishes in minutes; every other parameter
# keeps its mainnet default. Checks (PASS/FAIL table at the end):
#   - every mainnet rule is on at genesis (`aether mainnet-rules`, the one
#     list in crates/node/src/mainnet.rs; docs/ops/mainnet-launch.md §2);
#   - the chain runs protocol 3 at height 1 and the registration cap is
#     active (16 per epoch, read from the live state);
#   - blocks finalize and the four validators agree on the state root;
#   - empty blocks are quiet under history v2 (no statement, root unchanged);
#   - no premine and no faucet: dev accounts are unfunded, the faucet RPC refuses;
#   - the first epochs distribute exactly: the node pool of an epoch is the
#     epoch's issuance halves (rewards::issuance) and a fresh Mac's operator gets
#     pool × k × WARMUP_STEPS / (MAX_SHARE × FULL) for the k slots it answered
#     (docs/design/15-node-rewards.md; k varies with beacon timing in a 20 s epoch);
#   - the founder's reserve keys stay followers while the committee has four
#     seats (they only fill seats a committee is short of; audit 1.1);
#   - history pruning is the mainnet default: on, 30 days.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
A="${AETHER_BIN:-${CARGO_TARGET_DIR:-$ROOT/target}/release/aether}"
[ -x "$A" ] || { echo "no aether binary at $A (build it, or set AETHER_BIN)" >&2; exit 1; }
CHAIN=${REHEARSAL_CHAIN_ID:-7799}
EPOCH_BLOCKS=40
BLOCK_MS=500
D=${1:-}
[ -n "$D" ] || { sed -n '2,26p' "$0"; exit 1; }
mkdir -p "$D"
D=$(cd "$D" && pwd)
if [ -n "$(ls -A "$D" 2>/dev/null)" ]; then
  echo "$D is not empty: rehearsal data is throwaway, give a new dir" >&2
  exit 1
fi

PASS=() FAIL=()
ok() { PASS+=("$1"); echo "  ok    $1"; }
bad() { FAIL+=("$1"); echo "  FAIL  $1"; }

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
wait_height() { # <port> <target> <seconds>
  local end=$((SECONDS + $3))
  while [ "$(height "$1")" -lt "$2" ]; do
    if [ "$SECONDS" -ge "$end" ]; then return 1; fi
    sleep 2
  done
}

echo "== keys (4 validators, 3 reserve keys, 2 candidate Macs)"
names=(g1 g2 g3 g4 r1 r2 r3 c1 c2)
for k in ${names[@]+"${names[@]}"}; do "$A" keygen --data "$D/$k" >/dev/null; done
founder=$("$A" dev-accounts | awk '$1 == "dev" && $2 == 1 {print $3}')
ops=($("$A" dev-accounts | awk '$1 == "dev" && ($2 == 2 || $2 == 3) {print $3}'))
[ "${#ops[@]}" = 2 ] || { echo "could not read the dev accounts" >&2; exit 1; }

echo "== genesis (chain $CHAIN, protocol 3, history 2, node rewards, dev registrar, reserve keys, no faucet)"
"$A" network --chain-id "$CHAIN" --protocol 3 --epoch-blocks "$EPOCH_BLOCKS" --history 2 --node-rewards --dev-registrar \
  --reserve-operator "$founder" \
  --reserve "$D/r1/validator.pub.json" --reserve "$D/r2/validator.pub.json" --reserve "$D/r3/validator.pub.json" \
  "$D"/g1/validator.pub.json "$D"/g2/validator.pub.json "$D"/g3/validator.pub.json "$D"/g4/validator.pub.json \
  > "$D/genesis.json"

echo "== dkg (loopback tcp)"
ports=()
while [ "${#ports[@]}" -lt 31 ]; do
  p=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')
  case " ${ports[@]:-} " in *" $p "*) ;; *) ports+=("$p");; esac
done
dkgp=("${ports[@]:0:4}") p2p=("${ports[@]:4:9}") rpcp=("${ports[@]:13:9}") resh=("${ports[@]:22:9}")
for i in 1 2 3 4; do
  peers=""
  for j in 1 2 3 4; do [ "$j" = "$i" ] || peers+="${peers:+,}$j@127.0.0.1:${dkgp[$((j - 1))]}"; done
  "$A" dkg --network "$D/genesis.json" --port "${dkgp[$((i - 1))]}" --data "$D/g$i" --peers "$peers" --offline > "$D/dkg$i.log" 2>&1 &
  PIDS+=($!)
done
for p in ${PIDS[@]+"${PIDS[@]}"}; do wait "$p" || { echo "dkg failed (see $D/dkg*.log)" >&2; exit 1; }; done
PIDS=()
cp "$D/g1/network.json" "$D/network.json"

echo "== mainnet rule set (every rule on at height 1; docs/ops/mainnet-launch.md §2)"
if rules=$("$A" mainnet-rules --network "$D/network.json" 2>&1); then
  ok "aether mainnet-rules: every rule on ($(printf '%s\n' "$rules" | grep -c '^ok') items)"
else
  bad "aether mainnet-rules failed:"$'\n'"$rules"
fi

echo "== aether run × 9 (validators 1-4, reserve keys 5-7, candidates 8-9)"
mkdir -p "$D/peers"
for k in "${!names[@]}"; do
  others=""
  for j in "${!names[@]}"; do [ "$j" = "$k" ] || others+="${others:+,}http://127.0.0.1:${rpcp[$j]}"; done
  args=(run --exit-with-parent --data "$D/${names[$k]}" --network "$D/network.json"
    --port "${p2p[$k]}" --rpc-port "${rpcp[$k]}" --reshare-port "${resh[$k]}"
    --dev-peer-dir "$D/peers" --node-arg=--block-time-ms=$BLOCK_MS
    "--follow-arg=--from-rpc=$others" --reshare-timeout 120)
  if [ "${names[$k]}" = g1 ]; then args+=(--node-arg=--dev-registrar); fi
  RUST_LOG=info,commonware=warn nohup "$A" "${args[@]}" > "$D/${names[$k]}.log" 2>&1 &
  PIDS+=($!)
done

echo "== waiting for finalized blocks"
if wait_height "${rpcp[0]}" 12 180; then ok "blocks finalize (validator 1 reached height ≥ 12)"; else bad "no finalized blocks (see $D/*.log)"; fi
h=$(height "${rpcp[0]}")
roots=""
for k in 0 1 2 3; do
  roots+="$(rpc aether_getBlock "[$h]" "${rpcp[$k]}" | jget 'd["result"]["state_root"]') "
done
if [ "$(printf '%s\n' $roots | sort -u | grep -c .)" = 1 ]; then
  ok "the four validators agree on the state root at height $h"
else
  bad "the validators disagree at height $h: $roots"
fi

echo "== protocol 3 from genesis, registration cap active"
# The activation schedule carries [3, 0]: protocol 3 rules from height 0, so
# the protocol at height 1 is 3 (what aether_status reports at any height).
proto=$(rpc aether_status '[]' "${rpcp[0]}" | jget 'd["result"]["protocol"]')
at0=$(rpc aether_status '[]' "${rpcp[0]}" | jget '"yes" if [3, 0] in d["result"].get("schedule", []) else "no"')
if [ "$proto" = 3 ] && [ "$at0" = yes ]; then
  ok "the chain runs protocol 3 from height 0 (protocol at head $proto, [3, 0] on the schedule)"
else
  bad "protocol at height 1 is not 3 (protocol '$proto', [3, 0] on the schedule: '$at0')"
fi
cap=$(rpc aether_candidates '[]' "${rpcp[0]}" | jget 'd["result"]["max_per_epoch"]')
[ "$cap" = 16 ] && ok "registration cap active at genesis (max_per_epoch $cap)" || bad "no registration cap (max_per_epoch '$cap', want 16)"

echo "== history v2: empty blocks are quiet (no statement, root unchanged)"
# No candidates are registered yet, so no beacon answers: of the recent empty
# blocks only an epoch's first block (slots drawn) and a slot's next block
# (hash recorded) write state — under v2 the rest record no statement and keep
# the state root, under v1 every block records one and every root differs.
quiet="" seen=""
for ((b = h - 7; b < h; b++)); do
  ra=$(rpc aether_getBlock "[$b]" "${rpcp[0]}" | jget 'd["result"]["state_root"]')
  rb=$(rpc aether_getBlock "[$((b + 1))]" "${rpcp[0]}" | jget 'd["result"]["state_root"]')
  seen+="$b:${ra:-none} $((b + 1)):${rb:-none}  "
  if [ -n "$ra" ] && [ "$ra" = "$rb" ]; then quiet="$b $((b + 1))"; break; fi
done
if [ -n "$quiet" ]; then
  ok "blocks $quiet are empty: same state root (quiet under history v2)"
else
  bad "every recent block changed the state root: $seen"
fi

echo "== no premine, no faucet"
bal() { rpc aether_getAccount "[\"$1\"]" "${rpcp[0]}" | jget 'int(d["result"]["balance"], 16)'; }
unfunded=yes bals=""
for a in "$founder" "${ops[@]}"; do b=$(bal "$a"); bals+=" $b"; [ "$b" = "0" ] || unfunded=no; done
if [ "$unfunded" = yes ]; then ok "dev accounts (founder, operators) are unfunded at genesis"; else bad "a dev account has a balance:$bals"; fi
ferr=$(rpc aether_faucet "[\"$founder\"]" "${rpcp[0]}" | jget 'd["error"]["message"]')
case "$ferr" in
  *faucet*) ok "the faucet RPC refuses: '$ferr'" ;;
  *) bad "the faucet RPC did not refuse ('$ferr')" ;;
esac

echo "== registering the two candidate Macs (operators: dev 2 and 3, tip 0)"
if "$A" candidate-register --data "$D/c1" --registrar-rpc "http://127.0.0.1:${rpcp[0]}" --rpc "http://127.0.0.1:${rpcp[0]}" --from-dev 2 --tip 0 >/dev/null \
  && "$A" candidate-register --data "$D/c2" --registrar-rpc "http://127.0.0.1:${rpcp[0]}" --rpc "http://127.0.0.1:${rpcp[0]}" --from-dev 3 --tip 0 >/dev/null; then
  ok "two candidates registered (zero balance, zero fee)"
else
  bad "candidate registration failed (see $D/*.log)"
fi
cand=$(rpc aether_candidates '[]' "${rpcp[0]}" | jget 'len(d["result"]["candidates"])')
[ "$cand" = 2 ] && ok "the registry lists both Macs" || bad "the registry lists $cand Macs (want 2)"

echo "== first distributions (an epoch's node pool is its issuance halves)"
# Day 0 issuance is 1e18 a block: an epoch's pool is 40 × 5e17. Fresh Macs
# (warm-up level 0) weigh k × 14 for the k slots they answered in the epoch; with
# fewer than 16 operators the sum stays below the floor MAX_SHARE × FULL
# = 16 × (SLOTS × 2 × 14) = 5376 (SLOTS = 12), so an operator gets exactly
# pool × k × 14 / 5376 (rewards::lib docs). How many slots answer in a 20 s
# epoch depends on timing, so the check is: every operator is paid in every epoch
# after the first (a partial one: the Macs registered inside it) and each amount
# is exactly one of the legal values for k = 1..SLOTS.
if wait_height "${rpcp[0]}" "$(( (h / EPOCH_BLOCKS + 4) * EPOCH_BLOCKS ))" 180; then
  paid=yes detail=""
  for o in "${ops[@]}"; do
    raw=$(rpc aether_rewards "[\"$o\"]" "${rpcp[0]}")
    res=$(printf '%s' "$raw" | python3 -c '
import json, sys
EPOCH, SLOTS, WARM, MAX_SHARE = 40, 12, 14, 16
pool = EPOCH * (10**18 // 2)
legal = {pool * k * WARM // (MAX_SHARE * SLOTS * 2 * WARM) for k in range(1, SLOTS + 1)}
try:
    rows = [r for r in json.load(sys.stdin)["result"] if r["kind"] == "node"]
except Exception as e:
    print("unreadable:", e); sys.exit(1)
rows = sorted(rows, key=lambda r: r["height"])[1:]
if len(rows) < 2:
    print("only %d full epochs paid" % len(rows)); sys.exit(1)
bad = [int(r["amount"], 16) for r in rows if int(r["amount"], 16) not in legal]
if bad:
    print("illegal amounts", bad); sys.exit(1)
print("%d epochs, amounts %s" % (len(rows), sorted({int(r["amount"], 16) for r in rows})))
' 2>&1) || { paid=no; detail+=" [$o: $res]"; echo "    aether_rewards $o: $(printf '%s' "$raw" | head -c 500)" >&2; continue; }
    detail+=" [$res]"
  done
  if [ "$paid" = yes ]; then
    ok "each operator was paid in every full epoch, each amount exactly pool·k·14/5376:$detail"
  else
    bad "a node reward was missing or not an exact slot share:$detail"
  fi
else
  bad "the chain did not reach the next epochs"
fi

echo "== founder reserve keys stay out of a full (4-seat) committee"
rkeys=""
for r in r1 r2 r3; do rkeys+=" $(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["key"])' "$D/$r/validator.pub.json")"; done
members=$(rpc aether_handoff '[]' "${rpcp[0]}" | jget '" ".join(m["key"] for m in d["result"]["members"])' || true)
seated=no
for k in $rkeys; do case " $members " in *" $k "*) seated=yes;; esac; done
if [ "$seated" = no ]; then ok "no reserve key in the voting set (4 genesis seats)"; else bad "a reserve key joined a full committee"; fi
following=yes
for r in r1 r2 r3; do [ -f "$D/$r/threshold.json" ] && following=no; done
if [ "$following" = yes ]; then ok "the reserve keys run as followers (no key share)"; else bad "a reserve key holds a key share"; fi

echo "== history pruning is the mainnet default (30 days, on)"
retain=$((30 * 86400000 / BLOCK_MS))
# The validators' logs carry ANSI colors; grep -q in a pipe would also race
# sed on SIGPIPE under pipefail, so strip into a variable and match on it.
log=$(sed $'s/\x1b\\[[0-9;]*m//g' "$D/g1.log")
case "$log" in
  *"pruning history older than the retention window"*"retain_blocks=$retain "*)
    ok "pruning on, retain_blocks=$retain" ;;
  *)
    bad "no prune line with retain_blocks=$retain in validator 1's log" ;;
esac
if grep -q "archive node: keeping every block" "$D"/*.log; then bad "a node runs as an archive node"; else ok "no node keeps every block"; fi

echo "== shadow replay with this binary"
shadow_to=$(( $(height "${rpcp[0]}") - 2 ))
if [ "$shadow_to" -ge 1 ] && "$A" shadow --from "http://127.0.0.1:${rpcp[0]}" --to "$shadow_to" > "$D/shadow.log" 2>&1; then
  ok "shadow replay matches state and receipt digests through height $shadow_to"
else
  bad "shadow replay diverged or could not read source blocks (see $D/shadow.log)"
fi

echo
echo "==================== rehearsal results ===================="
for p in ${PASS[@]+"${PASS[@]}"}; do printf 'PASS  %s\n' "$p"; done
for f in ${FAIL[@]+"${FAIL[@]}"}; do printf 'FAIL  %s\n' "$f"; done
if [ "${#FAIL[@]}" = 0 ]; then
  echo "PASS: the mainnet genesis flags behave (chain data and logs kept in $D)"
else
  echo "FAIL: ${#FAIL[@]} check(s) failed (chain data and logs kept in $D)"
  exit 1
fi
