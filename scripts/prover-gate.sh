#!/usr/bin/env bash
# Release gate: refuse to package a macOS app whose proving program differs from
# the program the bundled network's validators verify. Proofs of any other
# program never verify, so an app that ships one proves for nothing (since
# 2026-09-29 no shipped app's proofs had verified: .claude/team/prover-070-mismatch.md).
#
#   scripts/prover-gate.sh APP                  APP = path to EastSea.app
#   scripts/prover-gate.sh --prover BIN --node BIN --network FILE
#
# The app's program is `Contents/Helpers/aether-prover info`. The validators'
# is their live `aether_proverProgram` answer, asked by the app's own node
# (`aether validator-program`, over iroh from Contents/Resources/network.json;
# AETHER_PROVER_GATE_RPC=<url>[,<url>] asks HTTP endpoints instead). Validators
# from before that RPC map to the program compiled in for their chain
# (crates/node/src/prover.rs KNOWN_VERIFIER_PROGRAMS); an unknown chain or no
# answer refuses too: a release must be checked, not assumed.
#
# A coordinated release that ships before the validator upgrade sets
#   AETHER_PROVER_GATE_OVERRIDE=<the app's program id>
#   AETHER_PROVER_GATE_REASON="<why, e.g. the validator upgrade that follows>"
# The override must name the app's own program (so it cannot wave a different
# build through), and it is recorded: on stderr, in the build log, and appended
# to $AETHER_BUILD_LOG (default dist/release-gates.log).
set -euo pipefail
cd "$(dirname "$0")/.."

prover="" node="" network=""
if [ $# = 1 ]; then
  app="$1"
  prover="$app/Contents/Helpers/aether-prover"
  node="$app/Contents/Helpers/aether"
  network="$app/Contents/Resources/network.json"
else
  while [ $# -gt 0 ]; do
    case "$1" in
      --prover) prover="$2"; shift 2 ;;
      --node) node="$2"; shift 2 ;;
      --network) network="$2"; shift 2 ;;
      *) echo "usage: $0 APP | --prover BIN --node BIN --network FILE" >&2; exit 2 ;;
    esac
  done
fi
for f in "$prover" "$node" "$network"; do
  [ -e "$f" ] || { echo "prover gate: missing $f" >&2; exit 1; }
done
log="${AETHER_BUILD_LOG:-dist/release-gates.log}"

app_program="$("$prover" info | python3 -c 'import json,sys; print(json.load(sys.stdin)["guest_elf_sha256"])')" ||
  { echo "prover gate: cannot read the app's program ($prover info)" >&2; exit 1; }
chain="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["chain_id"])' "$network")"

rpc_args=()
if [ -n "${AETHER_PROVER_GATE_RPC:-}" ]; then
  IFS=, read -r -a urls <<<"$AETHER_PROVER_GATE_RPC"
  for u in "${urls[@]}"; do rpc_args+=(--rpc "$u"); done
fi
if live="$("$node" validator-program --network "$network" "${rpc_args[@]+"${rpc_args[@]}"}" 2>"${TMPDIR:-/tmp}/prover-gate.$$")"; then
  source_note="$(cat "${TMPDIR:-/tmp}/prover-gate.$$")"
  validators="$live"
else
  source_note="$(cat "${TMPDIR:-/tmp}/prover-gate.$$")"
  validators=""
fi
rm -f "${TMPDIR:-/tmp}/prover-gate.$$"

echo "prover gate: chain $chain, app program $app_program"
if [ -n "$validators" ]; then
  echo "prover gate: validators verify $validators${source_note:+ ($source_note)}"
else
  echo "prover gate: validators' program unknown: ${source_note:-no answer}"
fi
if [ -n "$validators" ] && [ "$validators" = "$app_program" ]; then
  echo "prover gate: ok, the app proves with the validators' program"
  exit 0
fi

override="${AETHER_PROVER_GATE_OVERRIDE:-}"
if [ -n "$override" ]; then
  if [ "$override" != "$app_program" ]; then
    echo "prover gate: AETHER_PROVER_GATE_OVERRIDE=$override does not name this app's program $app_program" >&2
    exit 1
  fi
  record="$(date -u +%Y-%m-%dT%H:%M:%SZ) OVERRIDE chain=$chain app=$app_program validators=${validators:-unknown} commit=$(git rev-parse --short HEAD 2>/dev/null || echo none) reason=${AETHER_PROVER_GATE_REASON:-none given}"
  mkdir -p "$(dirname "$log")"
  echo "$record" >>"$log"
  echo "prover gate: $record" >&2
  echo "prover gate: OVERRIDDEN — recorded in $log; the validators must move to $app_program before these proofs verify" >&2
  exit 0
fi

if [ -n "$validators" ]; then
  echo "prover gate: REFUSED — the app proves with $app_program but chain $chain's validators verify $validators; its proofs would never verify." >&2
else
  echo "prover gate: REFUSED — cannot confirm what chain $chain's validators verify, so this app's proofs may never verify." >&2
fi
echo "  Build the app from the validators' release commit, upgrade the validators first, or, for a coordinated" >&2
echo "  release that ships before the validator upgrade, set AETHER_PROVER_GATE_OVERRIDE=$app_program and AETHER_PROVER_GATE_REASON." >&2
exit 1
