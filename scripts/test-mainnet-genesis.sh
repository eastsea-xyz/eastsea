#!/usr/bin/env bash
# Tests for scripts/mainnet-genesis.sh (the mainnet genesis ceremony tool):
# run the --dry-run end to end, then assert each contract of the real launch:
#   1. the dry run completes and labels its result "REHEARSAL, not a launch file";
#   2. the STRICT `aether mainnet-rules` (no --rehearsal) passes on the dry
#      run's final network.json — and exactly which values a rehearsal would
#      legitimately shorten (none: this tool always assembles with the
#      published policy; the shortened-timing allowance belongs to
#      scripts/mainnet-rehearsal.sh alone);
#   3. a tampered file (one genesis flag flipped) FAILS the check;
#   4. a rehearsal-only value injected by hand (epoch_blocks 40) FAILS the
#      strict check (candidate timing) — the real check never tolerates it;
#   5. a duplicate validator key is refused at assemble;
#   6. assemble refuses rehearsal-only flags (--epoch-blocks, --dev-registrar);
#   7. no secret file ever lands in the coordinator output directory;
#   8. `check` on the PRE-DKG genesis.json fails (it demands the final,
#      post-DKG file — the audit's point about checking the real final file);
#   9. STRICT mainnet-rules refuses the rehearsal chain id — exactly its own
#      rule — and a real-mode `check` on the 7799 dry file refuses it too
#      (both at test 2); a new-id copy of the same file then passes the real
#      mode, proving the refusal was about the id and nothing else;
#  10. audit 5 A5-4's file — valid structure, `identity: "aa"`,
#      `output: "bb"`, a new chain id — FAILS the strict gate (it used to
#      pass 16/16), and a real-mode check with the ceremony record passes on
#      the untouched file (23 rules: 19 genesis + 4 final-file);
#  11. verify-local refuses a threshold.json from another round (a validator
#      must not vote under a committee the final file does not name) and
#      passes on the matching one.
# Usage: scripts/test-mainnet-genesis.sh   (env: AETHER_BIN, as in the tool)
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
G=$ROOT/scripts/mainnet-genesis.sh
A=${AETHER_BIN:-}
if [ -z "$A" ]; then
  for c in "$ROOT/tmp/target/release/aether" "$ROOT/target/release/aether" "$(command -v aether 2>/dev/null || true)"; do
    if [ -x "$c" ]; then A=$c; break; fi
  done
fi
if [ -z "$A" ] || [ ! -x "$A" ]; then echo "no aether binary: build it, or set AETHER_BIN" >&2; exit 1; fi
mkdir -p "$ROOT/tmp"
WORK=$(mktemp -d "$ROOT/tmp/test-mainnet-genesis.XXXXXX")
REMOVE=$WORK   # emptied on failure, so the evidence stays for inspection
cleanup() { [ -n "${REMOVE:-}" ] && rm -rf "$REMOVE" || true; }
trap cleanup EXIT

PASS=() FAIL=()
ok() { PASS+=("$1"); echo "  ok    $1"; }
bad() { FAIL+=("$1"); echo "  FAIL  $1"; }
expect_fail() { # expect_fail <label> <command...> — the command must exit nonzero
  local label=$1; shift
  if "$@" > "$WORK/last.out" 2>&1; then bad "$label (it succeeded; output below)"; sed 's/^/        /' "$WORK/last.out"
  else ok "$label (refused, as required)"; sed 's/^/        /' "$WORK/last.out" | head -4; fi
}

echo "== 1. dry run end to end (REHEARSAL labeling)"
if "$G" --dry-run "$WORK/dry" > "$WORK/dry.out" 2>&1; then
  ok "--dry-run completes (output in $WORK/dry.out)"
else
  bad "--dry-run failed:"; sed 's/^/        /' "$WORK/dry.out" | tail -20
fi
if grep -q "REHEARSAL, not a launch file" "$WORK/dry.out" \
  && [ -f "$WORK/dry/REHEARSAL-NOT-A-LAUNCH-FILE" ] \
  && grep -q "^CHECK PASS" "$WORK/dry.out"; then
  ok "output labeled REHEARSAL, not a launch file (banner + marker file + CHECK PASS)"
else
  bad "dry run output is not clearly labeled a rehearsal"
fi
NET=$WORK/dry/v1/network.json
[ -f "$NET" ] || { bad "the dry run wrote no final network.json at $NET"; exit 1; }

echo "== 2. STRICT mainnet-rules (never --rehearsal) on the dry run's final file"
if out=$("$A" mainnet-rules --rehearsal --network "$NET" 2>&1); then
  n=$(printf '%s\n' "$out" | grep -c '^ok' || true)
  if [ "$n" = 23 ]; then ok "rehearsal-mode check passes: all 23 rules on (19 genesis + 4 final-file)"; else bad "rehearsal check passed but printed $n ok lines (want 23)"; fi
else
  bad "rehearsal check FAILED on the dry run's file:"$'\n'"$out"
fi
if out=$("$A" mainnet-rules --network "$NET" 2>&1); then
  bad "strict check PASSED on the rehearsal-id file (7799 must be refused outside --rehearsal)"
else
  fails=$(printf '%s\n' "$out" | grep -c '^FAIL' || true)
  if [ "$fails" = 1 ] && printf '%s\n' "$out" | grep -q '^FAIL  chain id'; then
    ok "strict check refuses the rehearsal chain id: exactly the chain id rule fails"
  else
    bad "strict check failed $fails rules on the 7799 file (want exactly 1: chain id)"; printf '%s\n' "$out" | sed 's/^/        /'
  fi
fi
expect_fail "check (real mode) refuses the rehearsal chain id 7799" "$G" check "$NET" --chain-id 7799 --ceremony "$WORK/dry/coordinator/ceremony.json"
if grep -q "reserved" "$WORK/last.out"; then ok "the refusal says the id is reserved"; else bad "the 7799 refusal does not explain itself"; fi
echo "  values a rehearsal legitimately shortens: NONE."
echo "  The dry run assembles with the published policy — epoch 3600 blocks, min streak 24,"
echo "  draw every 24 epochs (defaults; the tool refuses every timing flag) and a real P-256"
echo "  registrar key — so the strict rule check passes its file unchanged (test 10 runs the"
echo "  real-mode pass on a new-id copy). The shortened timing a rehearsal needs (epoch_blocks"
echo "  40 etc.) and the --rehearsal allowance belong to scripts/mainnet-rehearsal.sh alone;"
echo "  this tool passes --rehearsal only in --dry-run, as a label, never to excuse a value."

# Tests 3, 4 and 8 must reach the RULE list, so they use a NEW chain id (the 7799
# rehearsal id is refused before any rule runs, see test 2).
python3 - "$WORK/ceremony7801.json" <<'PY'
import json, sys
json.dump({"chain_id": 7801}, open(sys.argv[1], "w"))
PY
echo "== 3. a tampered file (one flag flipped: protocol 3 -> 1) FAILS"
python3 - "$NET" "$WORK/tampered.json" <<'PY'
import json, sys
n = json.load(open(sys.argv[1]))
n["protocol"] = 1
n["chain_id"] = 7801
json.dump(n, open(sys.argv[2], "w"), indent=2)
PY
expect_fail "check refuses a genesis with protocol 1" "$G" check "$WORK/tampered.json" --chain-id 7801 --ceremony "$WORK/ceremony7801.json"
if grep -q "^FAIL" "$WORK/last.out"; then ok "the refusal prints FAIL lines (a PASS/FAIL list, not silence)"
else bad "no FAIL line in the tampered-file refusal"; fi

echo "== 4. a rehearsal-only value injected by hand (epoch_blocks 40) FAILS the strict check"
python3 - "$NET" "$WORK/shortened.json" <<'PY'
import json, sys
n = json.load(open(sys.argv[1]))
n["chain_id"] = 7801
n["epoch_blocks"] = 40          # what scripts/mainnet-rehearsal.sh legitimately shortens
json.dump(n, open(sys.argv[2], "w"), indent=2)
PY
expect_fail "check refuses rehearsal-only timing (epoch_blocks 40)" "$G" check "$WORK/shortened.json" --chain-id 7801 --ceremony "$WORK/ceremony7801.json"
if grep -q "candidate timing" "$WORK/last.out"; then
  ok "the strict rule check names 'candidate timing' as the off rule"
else
  bad "the refusal does not name the 'candidate timing' rule"
fi

echo "== 5. a duplicate validator key is refused at assemble"
cp "$WORK/dry/coordinator/pub/v1.pub.json" "$WORK/dup-v2.pub.json"
reg=$(awk 'NR == 1 && $1 == "registrar" && $2 == "key" {print $3}' "$WORK/dry/registrar-key.out")
founder=$("$A" dev-accounts | awk '$1 == "dev" && $2 == 1 {print $3}')
if [ "${#reg}" = 128 ] && [ -n "$founder" ]; then
  ok "inputs for the assemble tests read from the dry run (registrar ${#reg} hex, reserve operator $founder)"
else
  bad "could not read the dry run's registrar key ('$reg') or the dev-1 address ('$founder')"
fi
expect_fail "assemble refuses two validators with one key" "$G" assemble \
  --chain-id 7801 --registrar "$reg" --reserve-operator "$founder" \
  --reserve "$WORK/dry/coordinator/pub/r1.pub.json" --reserve "$WORK/dry/coordinator/pub/r2.pub.json" \
  --reserve "$WORK/dry/coordinator/pub/r3.pub.json" \
  --validator "$WORK/dry/coordinator/pub/v1.pub.json" --validator "$WORK/dup-v2.pub.json" \
  --validator "$WORK/dry/coordinator/pub/v3.pub.json" --validator "$WORK/dry/coordinator/pub/v4.pub.json" \
  --out "$WORK/dup"
if grep -q "same key" "$WORK/last.out"; then ok "the refusal says it is the same key (named validators)"
else bad "the duplicate-key refusal does not explain itself"; fi

echo "== 6. assemble refuses rehearsal-only flags"
expect_fail "assemble refuses --epoch-blocks" "$G" assemble --chain-id 7801 --registrar "$reg" \
  --reserve-operator "$founder" --epoch-blocks 40 \
  --reserve "$WORK/dry/coordinator/pub/r1.pub.json" --validator "$WORK/dry/coordinator/pub/v1.pub.json"
expect_fail "assemble refuses --dev-registrar" "$G" assemble --chain-id 7801 --dev-registrar \
  --reserve-operator "$founder" \
  --reserve "$WORK/dry/coordinator/pub/r1.pub.json" --validator "$WORK/dry/coordinator/pub/v1.pub.json"
expect_fail "assemble refuses the testnet chain id 7780" "$G" assemble --chain-id 7780 \
  --registrar "$reg" --reserve-operator "$founder" \
  --reserve "$WORK/dry/coordinator/pub/r1.pub.json" --validator "$WORK/dry/coordinator/pub/v1.pub.json"

echo "== 7. no secret file in the coordinator output directory"
secrets=$(find "$WORK/dry/coordinator" -type f \( -name '*.key' -o -name 'threshold.json' \) || true)
if [ -z "$secrets" ]; then
  ok "coordinator dir holds no secret ($(find "$WORK/dry/coordinator" -type f | wc -l | tr -d ' ') files: genesis.json + the copied public halves)"
else
  bad "secret files in the coordinator directory:"$'\n'"$secrets"
fi
if grep -q "PASS  no secret file in" "$WORK/dry.out"; then
  ok "the dry run's own end-of-run secret scan printed PASS"
else
  bad "the dry run's secret scan did not print its PASS line"
fi

echo "== 8. check demands the FINAL (post-DKG) file, not the pre-DKG genesis.json"
python3 - "$WORK/dry/coordinator/genesis.json" "$WORK/predkg7801.json" <<'PY'
import json, sys
n = json.load(open(sys.argv[1]))
n["chain_id"] = 7801
json.dump(n, open(sys.argv[2], "w"), indent=2)
PY
expect_fail "check refuses the pre-DKG genesis.json (no committee identity)" \
  "$G" check "$WORK/predkg7801.json" --chain-id 7801 --ceremony "$WORK/ceremony7801.json"
if grep -q "committee identity" "$WORK/last.out"; then ok "the refusal names the missing committee identity"
else bad "the pre-DKG refusal does not name the committee identity"; fi

echo "== 9. the 7799 refusal was about the id: a new-id copy of the same file passes"
# The strict refusal itself runs at test 2 (the CLI) and its expect_fail (the
# script's real-mode check); here a NEW-id copy of the same file passes the
# real mode — proving the refusal was about the id, not the file.
python3 - "$NET" "$WORK/final7801.json" "$WORK/ceremony7801.json" <<'PY'
import json, sys
n = json.load(open(sys.argv[1]))
n["chain_id"] = 7801            # a new id: what the real launch assembles
json.dump(n, open(sys.argv[2], "w"), indent=2)
json.dump({"chain_id": 7801}, open(sys.argv[3], "w"))
PY
if out=$("$A" mainnet-rules --network "$WORK/final7801.json" 2>&1); then
  n=$(printf '%s\n' "$out" | grep -c '^ok' || true)
  if [ "$n" = 23 ]; then ok "strict check passes the new-id final file: 23 rules on"; else bad "strict check passed but printed $n ok lines (want 23)"; fi
else
  bad "strict check FAILED on a valid new-id final file:"$'\n'"$out"
fi
if "$G" check "$WORK/final7801.json" --chain-id 7801 --ceremony "$WORK/ceremony7801.json" > "$WORK/check-7801.out" 2>&1; then
  ok "real-mode check (strict, --chain-id + --ceremony) passes on the new-id final file"
else
  bad "real-mode check failed on the new-id final file:"; sed 's/^/        /' "$WORK/check-7801.out" | tail -8
fi

echo "== 10. audit 5 A5-4: valid structure, junk committee fields, must FAIL"
python3 - "$NET" "$WORK/junk.json" "$WORK/ceremony7801.json" <<'PY'
import json, sys
n = json.load(open(sys.argv[1]))
n["chain_id"] = 7801            # the audit's real-ceremony id
n["identity"] = "aa"            # undecodable junk that used to pass "nonempty"
n["output"] = "bb"
json.dump(n, open(sys.argv[2], "w"), indent=2)
PY
expect_fail "strict mainnet-rules refuses identity aa / output bb" "$A" mainnet-rules --network "$WORK/junk.json"
if grep -q "committee output decodes" "$WORK/last.out"; then ok "the refusal names the committee output decodes rule"
else bad "the junk-fields refusal does not name the final-file rules"; fi
expect_fail "check (real mode) refuses the junk committee fields" "$G" check "$WORK/junk.json" --chain-id 7801 --ceremony "$WORK/ceremony7801.json"
if grep -q "^FAIL" "$WORK/last.out"; then ok "the refusal prints FAIL lines (a PASS/FAIL list, not silence)"
else bad "no FAIL line in the junk-fields refusal"; fi
# A swapped validator (the DKG seated someone else) fails the seating rule.
python3 - "$NET" "$WORK/swap.json" "$WORK/ceremony7801.json" <<'PY'
import json, sys
n = json.load(open(sys.argv[1]))
n["chain_id"] = 7801
n["validators"][3]["key"] = "11" * 32   # not the key the DKG seated
json.dump(n, open(sys.argv[2], "w"), indent=2)
PY
expect_fail "check refuses an output that does not seat the roster" "$G" check "$WORK/swap.json" --chain-id 7801 --ceremony "$WORK/ceremony7801.json"
if grep -q "output seats the genesis roster" "$WORK/last.out"; then ok "the refusal names the seating rule"
else bad "the swapped-roster refusal does not name the seating rule"; fi

echo "== 11. verify-local: this Mac's share must match the final file before voting"
if "$G" verify-local "$WORK/final7801.json" --data "$WORK/dry/v2" > "$WORK/verify-ok.out" 2>&1; then
  ok "verify-local passes on a validator's own threshold.json"
else
  bad "verify-local failed on the matching share:"; sed 's/^/        /' "$WORK/verify-ok.out" | tail -8
fi
python3 - "$WORK/dry/v1/threshold.json" "$WORK/mismatch-dir" <<'PY'
import json, os, sys
src, dst = sys.argv[1], sys.argv[2]
os.makedirs(dst, exist_ok=True)
t = json.load(open(src))
t["round"] += 1               # a stale round: the committee the final file does not name
json.dump(t, open(os.path.join(dst, "threshold.json"), "w"), indent=2)
PY
expect_fail "verify-local refuses a share from another round" "$G" verify-local "$WORK/final7801.json" --data "$WORK/mismatch-dir"
if grep -q "round" "$WORK/last.out"; then ok "the refusal names the round mismatch"
else bad "the mismatch refusal does not name the round"; fi
expect_fail "verify-local refuses the rehearsal chain id too" "$G" verify-local "$NET" --data "$WORK/dry/v1"

echo
echo "==================== test results ===================="
for p in ${PASS[@]+"${PASS[@]}"}; do printf 'PASS  %s\n' "$p"; done
for f in ${FAIL[@]+"${FAIL[@]}"}; do printf 'FAIL  %s\n' "$f"; done
if [ "${#FAIL[@]}" = 0 ]; then
  echo "PASS: mainnet-genesis.sh behaves as the ceremony requires (throwaway work dir removed)"
else
  echo "FAIL: ${#FAIL[@]} test(s) failed — work dir kept for inspection at $WORK"
  REMOVE=""
  exit 1
fi
