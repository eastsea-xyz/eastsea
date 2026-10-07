#!/usr/bin/env bash
# Tests for the packaging gate scripts/prover-gate.sh, with stand-in binaries
# (no build): it passes only an app whose program is the validators', refuses
# a different or an unknown one, and lets a coordinated release through only
# with an override that names the app's own program, recorded in the log.
#
#   scripts/test-prover-gate.sh
set -euo pipefail
cd "$(dirname "$0")/.."
tmp="$(mktemp -d "${TMPDIR:-/tmp}/aether-prover-gate-XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "ok: $*"; }

app="$tmp/EastSea.app"
mkdir -p "$app/Contents/Helpers" "$app/Contents/Resources"
printf '{"chain_id": 7780}\n' >"$app/Contents/Resources/network.json"
cat >"$app/Contents/Helpers/aether-prover" <<'EOF'
#!/bin/sh
[ "$1" = info ] && printf '{"guest_elf_sha256":"%s","guest_elf_bytes":1}\n' "$GATE_APP_PROGRAM"
EOF
# The node stand-in records its arguments and answers $GATE_VALIDATORS
# (empty: no answer, the way an unreachable network fails).
cat >"$app/Contents/Helpers/aether" <<EOF
#!/bin/sh
echo "\$@" >"$tmp/node-args"
[ "\$1" = validator-program ] || exit 2
if [ -z "\$GATE_VALIDATORS" ]; then echo "no validator answered aether_proverProgram within 30s" >&2; exit 1; fi
echo "\$GATE_VALIDATORS"
EOF
chmod +x "$app/Contents/Helpers/"*
export AETHER_BUILD_LOG="$tmp/gates.log"
gate() { env -u AETHER_PROVER_GATE_OVERRIDE -u AETHER_PROVER_GATE_REASON -u AETHER_PROVER_GATE_RPC "$@" scripts/prover-gate.sh "$app" >"$tmp/out" 2>&1; }

gate GATE_APP_PROGRAM=aaaa GATE_VALIDATORS=aaaa || { cat "$tmp/out"; fail "the validators' own program was refused"; }
grep -q "ok, the app proves with the validators' program" "$tmp/out" || fail "no ok line"
grep -q -- "--network $app/Contents/Resources/network.json" "$tmp/node-args" || fail "the gate did not ask about the bundled network"
pass "passes an app whose program the validators verify"

if gate GATE_APP_PROGRAM=aaaa GATE_VALIDATORS=bbbb; then fail "a different program was packaged"; fi
grep -q "REFUSED — the app proves with aaaa but chain 7780's validators verify bbbb" "$tmp/out" || { cat "$tmp/out"; fail "no refusal reason"; }
if gate GATE_APP_PROGRAM=aaaa GATE_VALIDATORS=; then fail "an unconfirmed program was packaged"; fi
grep -q "REFUSED — cannot confirm" "$tmp/out" || fail "no unknown refusal"
[ ! -e "$AETHER_BUILD_LOG" ] || fail "a refusal wrote an override record"
pass "refuses a different program and an unknown one"

if gate GATE_APP_PROGRAM=aaaa GATE_VALIDATORS=bbbb AETHER_PROVER_GATE_OVERRIDE=cccc; then
  fail "an override naming another program let this build through"
fi
grep -q "does not name this app's program aaaa" "$tmp/out" || fail "no override mismatch reason"
gate GATE_APP_PROGRAM=aaaa GATE_VALIDATORS=bbbb AETHER_PROVER_GATE_OVERRIDE=aaaa \
  AETHER_PROVER_GATE_REASON="validators upgrade to 0.7.1 after release" || { cat "$tmp/out"; fail "a coordinated override was refused"; }
grep -q "OVERRIDE chain=7780 app=aaaa validators=bbbb .* reason=validators upgrade to 0.7.1 after release" "$AETHER_BUILD_LOG" ||
  fail "the override was not recorded in the build log"
grep -q "OVERRIDDEN" "$tmp/out" || fail "the override was not reported"
gate GATE_APP_PROGRAM=aaaa GATE_VALIDATORS= AETHER_PROVER_GATE_OVERRIDE=aaaa || fail "an override with the validators unreachable was refused"
grep -q "validators=unknown" "$AETHER_BUILD_LOG" || fail "an override with no answer was not recorded as unknown"
pass "an override must name the app's program and is recorded"

gate GATE_APP_PROGRAM=aaaa GATE_VALIDATORS=aaaa AETHER_PROVER_GATE_RPC="http://127.0.0.1:8601,http://127.0.0.1:8602" || fail "rpc run failed"
grep -q -- "--rpc http://127.0.0.1:8601 --rpc http://127.0.0.1:8602" "$tmp/node-args" || fail "AETHER_PROVER_GATE_RPC was not passed on"
pass "AETHER_PROVER_GATE_RPC asks the given endpoints"
grep -q 'scripts/prover-gate.sh "$src"' scripts/package-mac.sh || fail "package-mac.sh does not run the gate"
pass "package-mac.sh runs the gate before packaging"
echo "all prover gate tests passed"
