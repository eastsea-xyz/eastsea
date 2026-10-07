#!/usr/bin/env bash
# contracts-live: run every crates/contracts-onchain fixture against a real,
# local, NEW-GENESIS chain — deployed and driven through the wallet path (the
# `aether` CLI: sign_call_with + recommended_state_budget, the same code the
# wallet app and extension use), then checked in the explorer and three
# toolbox frontends in headless Chrome, then stressed under a B5 burst.
#
# Phases (defaults to all of them, in order):
#   bin        stage the built binaries side by side (aether + aether-prover)
#   deps       npm-install the driver deps (scripts/contracts-live/node_modules)
#   chain      genesis (chain 7796 = the toolbox gate 0x1e64, protocol 3, B5
#              paid state, faucet funds dev1) → DKG → 4 validators on loopback
#   registrar  the real CommitteeRegistry flow: aether candidate-register
#   flows      deploy all 41 fixtures + their user flows (deploy-flows.mjs)
#   explorer   read ≥5 views back in the explorer (explorer-check.mjs)
#   dapp       drive 3 toolbox frontends through an EIP-1193 shim (dapp-check.mjs)
#   stress     200 fresh-account transfers + 20 large deploys (stress.mjs)
#   stop       stop the validators (results stay in tmp/live)
#   reset      wipe the chain dirs (the default run restarts a fresh genesis for stress)
#
# Environment: KEEP=1 leaves validators running between invocations; FAST=1
# starts a legacy (free-state, no B5) genesis for debugging the drivers only.
# A full default run takes ~2 h: after the 100,000-unit burst the 41
# deployments wait on the 32-units-per-block refill (see the report).
#
# Everything the run creates — chain data, keys, logs, node_modules, ports —
# lives under <worktree>/tmp on loopback only. Nothing here touches
# ~/aether-testnet, the archive node, or the wallet app's data.

set -euo pipefail
cd "$(dirname "$0")/.."
ROOT="$PWD"

CHAIN=7796            # 0x1e64 — the chain id the toolbox frontends hard-require
EPOCH_BLOCKS=144
BLOCK_MS=1000
FOLLOWER_PORT=8649            # the stress phase's follower (round 2, finding 2)
RPC="http://127.0.0.1:8645"   # validator 1's JSON-RPC (8545/8601-8604/9101-9104/
                              # 18545/19101 belong to the real testnet — avoided)
D="$ROOT/tmp/live"            # run outputs (results, logs, shots)
CG="$D/chain"                 # validator keys + node data dirs
BIN="$ROOT/tmp/bin"           # aether + aether-prover, side by side
A="$BIN/aether"
SRV="$ROOT/scripts/contracts-live"

PIDS=()
# KEEP=1 leaves the validators running when this invocation exits, so phases
# can be run one at a time (`KEEP=1 scripts/contracts-live.sh bin deps chain`,
# then `scripts/contracts-live.sh flows ...`); `stop` halts them via the pid files.
cleanup() {
  if [ "${KEEP:-0}" != 1 ] && [ ${#PIDS[@]} -gt 0 ]; then
    kill "${PIDS[@]}" 2>/dev/null || true
    sleep 2
    kill -9 "${PIDS[@]}" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

say() { printf '\n== %s\n' "$*"; }
fail() { printf 'FAIL: %s\n' "$*" >&2; exit 1; }

rpc() { curl -sf -m 5 "$RPC" -H 'content-type: application/json' \
  -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$1\",\"params\":${2:-[]}}" ; }
height() { rpc aether_status | python3 -c 'import json,sys; print(json.load(sys.stdin)["result"]["height"])' 2>/dev/null || echo -1; }
wait_height() { # wait_height <target> <seconds>
  local target=$1 t0=$SECONDS h
  while :; do
    h=$(height)
    [ "$h" -ge "$target" ] && return 0
    [ $((SECONDS - t0)) -lt "$2" ] || return 1
    sleep 2
  done
}
port_free() { ! lsof -iTCP:"$1" -sTCP:LISTEN >/dev/null 2>&1; }

# ------------------------------------------------------------------ phases

phase_bin() {
  say "bin: staging aether + aether-prover (the node needs the prover beside it)"
  mkdir -p "$BIN"
  # The node: this worktree's build. The prover sidecar: this worktree's build
  # if present, else AETHER_PROVER_SRC, else the lead lane's rehearsal staging
  # (same lead-merge lineage; this run only VERIFIES — no node has AETHER_PROVE —
  # and verification is input-less, so any sidecar of the same interface works).
  cp -p "$ROOT/tmp/target.noindex/release/aether" "$BIN/aether" 2>/dev/null \
    || [ -x "$BIN/aether" ] || fail "build aether first: cargo build --release -p aether-node"
  if [ -x "$ROOT/tmp/target.noindex/release/aether-prover" ]; then
    cp -p "$ROOT/tmp/target.noindex/release/aether-prover" "$BIN/aether-prover"
  elif [ -n "${AETHER_PROVER_SRC:-}" ] && [ -x "$AETHER_PROVER_SRC" ]; then
    cp -p "$AETHER_PROVER_SRC" "$BIN/aether-prover"; echo "prover: $AETHER_PROVER_SRC"
  elif [ -x "$ROOT/../lead/tmp/rehearsal-bin/aether-prover" ]; then
    cp -p "$ROOT/../lead/tmp/rehearsal-bin/aether-prover" "$BIN/aether-prover"
    echo "prover: lead rehearsal staging (verify-only run — no AETHER_PROVE anywhere)"
  else
    fail "no aether-prover: build apps/prover or point AETHER_PROVER_SRC at one"
  fi
  "$A" --help >/dev/null 2>&1 || fail "$A does not run"
  "$BIN/aether-prover" info >/dev/null 2>&1 || fail "$BIN/aether-prover does not run"
}

phase_deps() {
  say "deps: npm install for the node drivers"
  [ -d "$SRV/node_modules/ethers" ] && { echo "already installed"; return; }
  if npm install --prefix "$SRV" --no-audit --no-fund --loglevel=error; then :; else
    # Offline fallback: the previous run's install, if any.
    [ -d "$D/node_modules" ] && cp -R "$D/node_modules" "$SRV/node_modules" \
      || fail "npm install failed and no cached node_modules"
  fi
  echo "node $(node --version), ethers $(node -e 'console.log(require("'$SRV'/node_modules/ethers/package.json").version)')"
}

phase_chain() {
  say "chain: genesis (chain $CHAIN, protocol 3, B5 paid state, faucet → dev1) + DKG + 4 validators"
  for p in 8701 8702 8703 8704 8711 8712 8713 8714 8645 8646 8647 8648; do
    port_free "$p" || fail "port $p is in use — another chain (or the testnet?) is there"
  done
  mkdir -p "$CG"
  [ -e "$CG/g1/threshold.json" ] || {
    for i in 1 2 3 4; do
      rm -rf "$CG/g$i"
      "$A" keygen --data "$CG/g$i" >/dev/null
    done
    dev1=$("$A" dev-accounts | awk '$2 == 1 {print $3}')
    [ "${#dev1}" = 42 ] || fail "could not read dev1 from aether dev-accounts"
    echo "faucet → dev1 $dev1"
    # The measured run is a new-genesis chain: history v2 + node rewards turn
    # on paid state and the B5 rolling budget (100,000-unit burst, 32 units
    # refilled per height). FAST=1 is for debugging the drivers only: a legacy
    # genesis without them, where state is free and unmetered — its numbers
    # are NOT B5 numbers and the report never uses them.
    growth=(--history 2 --node-rewards)
    if [ "${FAST:-0}" = 1 ]; then
      growth=()
      echo "FAST=1: legacy genesis, NO paid state / B5 — driver debugging only, not a measurement"
    fi
    "$A" network --chain-id "$CHAIN" --protocol 3 --epoch-blocks "$EPOCH_BLOCKS" \
      ${growth[@]+"${growth[@]}"} --dev-registrar --faucet "$dev1" \
      "$CG"/g1/validator.pub.json "$CG"/g2/validator.pub.json \
      "$CG"/g3/validator.pub.json "$CG"/g4/validator.pub.json > "$CG/genesis.json"
    say "dkg (loopback tcp)"
    dkgpids=()
    for i in 1 2 3 4; do
      peers=""
      for j in 1 2 3 4; do [ "$j" = "$i" ] || peers+="${peers:+,}$j@127.0.0.1:$((8700 + j))"; done
      "$A" dkg --network "$CG/genesis.json" --port $((8700 + i)) --data "$CG/g$i" \
        --peers "$peers" --offline > "$D/dkg$i.log" 2>&1 &
      dkgpids+=($!)
    done
    for p in "${dkgpids[@]}"; do wait "$p" || fail "dkg failed (see $D/dkg*.log)"; done
    PIDS=()
  }
  # Audit 6: a validator binds to the coordinator's ceremony record before it
  # votes on a new genesis. Like scripts/mainnet-rehearsal.sh, this run writes
  # the record for its own final network.json and passes it to every node, so
  # the startup bind runs instead of being switched off.
  [ -f "$CG/network.json" ] || cp "$CG/g1/network.json" "$CG/network.json"
  [ -f "$CG/ceremony-check.json" ] \
    || "$A" ceremony-record --network "$CG/network.json" --out "$CG/ceremony-check.json" >/dev/null

  say "validators (loopback tcp, offline; node 1 also runs the dev registrar)"
  [ -f "$CG/network.json" ] || fail "no $CG/network.json — dkg did not run"
  for i in 1 2 3 4; do
    peers=""
    for j in 1 2 3 4; do [ "$j" = "$i" ] || peers+="${peers:+,}$j@127.0.0.1:$((8710 + j))"; done
    args=(node --network "$CG/network.json" --ceremony "$CG/ceremony-check.json" --port $((8710 + i)) --rpc-port $((8644 + i))
      --data "$CG/g$i" --peers "$peers" --offline --block-time-ms "$BLOCK_MS")
    [ "$i" = 1 ] && args+=(--dev-registrar)   # chain has a faucet: allowed, prints a notice
    RUST_LOG=info,commonware=warn nohup "$A" "${args[@]}" > "$D/node$i.log" 2>&1 &
    echo $! > "$D/node$i.pid"
    PIDS+=($!)
  done
  say "waiting for finalized blocks"
  wait_height 2 180 || { tail -5 "$D"/node*.log; fail "validators did not produce blocks"; }
  echo "height $(height), rpc $RPC"
}

phase_registrar() {
  say "registrar: the real registry flow (aether candidate-register, dev registrar)"
  [ "${FAST:-0}" = 1 ] && { echo "FAST=1: legacy genesis has no free registration — skipped"; return; }
  [ -d "$CG/c1" ] || { "$A" keygen --data "$CG/c1" >/dev/null; }
  out=$("$A" candidate-register --data "$CG/c1" --registrar-rpc "$RPC" --rpc "$RPC" --from-dev 2 2>&1) \
    || fail "candidate-register failed: $out"
  echo "$out" | tail -2
  echo "$out" | grep -q "success=true" || fail "candidate registration not successful: $out"
}

phase_flows() {
  say "flows: every fixture, deployed + driven through the CLI wallet path"
  AETHER_RPC="$RPC" AETHER_BIN="$A" AETHER_CHAIN_ID="$CHAIN" \
    OUT="$D/results.json" node "$SRV/deploy-flows.mjs"
}

phase_explorer() {
  say "explorer: read the live views back in headless Chrome"
  AETHER_RPC="$RPC" OUT="$D/explorer.json" node "$SRV/explorer-check.mjs"
}

phase_dapp() {
  say "dapp: 3 toolbox frontends over the EIP-1193 shim (same CLI signing path)"
  AETHER_RPC="$RPC" AETHER_BIN="$A" OUT="$D/dapp.json" node "$SRV/dapp-check.mjs"
}

phase_stress() {
  say "stress: 200 fresh-account transfers + 20 large deploys in a burst"
  # A follower of the same chain (B5 review round 2, finding 2): the burst is
  # also read through it, as a remote wallet's reads are. NO_FOLLOWER=1 skips.
  local follower_rpc=""
  if [ "${NO_FOLLOWER:-0}" != 1 ] && port_free "$FOLLOWER_PORT"; then
    rm -rf "$CG/f1"
    RUST_LOG=info,commonware=warn nohup "$A" follow --network "$CG/network.json" --from-rpc "$RPC" \
      --data "$CG/f1" --rpc-port "$FOLLOWER_PORT" > "$D/follower.log" 2>&1 &
    echo $! > "$D/follower.pid"
    PIDS+=($!)
    for _ in $(seq 1 60); do
      curl -sf -m 2 "http://127.0.0.1:$FOLLOWER_PORT" -H 'content-type: application/json' \
        -d '{"jsonrpc":"2.0","id":1,"method":"aether_status","params":[]}' >/dev/null 2>&1 && { follower_rpc="http://127.0.0.1:$FOLLOWER_PORT"; break; }
      sleep 2
    done
    [ -n "$follower_rpc" ] && echo "follower: $follower_rpc" || { echo "follower did not answer (see $D/follower.log); stress runs without it"; tail -3 "$D/follower.log" || true; }
  fi
  AETHER_RPC="$RPC" AETHER_BIN="$A" AETHER_FOLLOWER_RPC="$follower_rpc" OUT="$D/stress.json" node "$SRV/stress.mjs"
}

phase_stop() {
  say "stop: halting the validators (results stay in $D)"
  pids=()
  for i in 1 2 3 4; do [ -f "$D/node$i.pid" ] && pids+=("$(cat "$D/node$i.pid")"); done
  [ -f "$D/follower.pid" ] && pids+=("$(cat "$D/follower.pid")")
  [ ${#pids[@]} -gt 0 ] && kill "${pids[@]}" 2>/dev/null || true
  for _ in $(seq 1 15); do
    alive=0; for p in ${pids[@]+"${pids[@]}"}; do kill -0 "$p" 2>/dev/null && alive=1; done
    [ "$alive" = 0 ] && break; sleep 1
  done
  [ ${#pids[@]} -gt 0 ] && kill -9 "${pids[@]}" 2>/dev/null || true
  rm -f "$D"/node*.pid "$D"/follower.pid
  PIDS=()
  echo "stopped."
}

phase_reset() {
  say "reset: wipe the chain so the next 'chain' phase starts a fresh genesis (full B5 burst)"
  rm -rf "$CG" "$D"/node*.pid
  echo "wiped $CG"
}

# ------------------------------------------------------------------ main

PHASES=("${@:-all}")
# The stress burst runs on its own fresh genesis so it meets a full 100,000-unit
# burst rather than the debt the 41 deployments left behind.
[ "${PHASES[0]}" = all ] && PHASES=(bin deps chain registrar flows explorer dapp stop reset bin chain stress stop)
for p in "${PHASES[@]}"; do
  case "$p" in
    bin|deps|chain|registrar|flows|explorer|dapp|stress|stop|reset) "phase_$p" ;;
    *) fail "unknown phase '$p' (bin deps chain registrar flows explorer dapp stress stop reset | all)" ;;
  esac
done
say "done: phases ${PHASES[*]} — outputs in $D"
