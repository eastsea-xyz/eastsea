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
#      the untouched file (26 rules: 21 genesis + release pin + 4 final-file);
#  11. verify-local refuses a threshold.json from another round (a validator
#      must not vote under a committee the final file does not name) and
#      passes with --ceremony on the matching one, storing the record in the
#      data dir for `aether run`;
#  12. audit 6 A6-3/A6-4 — the bind behind verify-local and node startup:
#      no --ceremony is refused; a chain id swapped in transit (7801 -> 7802)
#      is refused by both; node startup with no record names the operator
#      step; a missing, malformed, other-chain or stale-genesis (same
#      id+identity, another history) local network.json is refused, never
#      skipped; a same-roster-same-round file with another output is refused
#      by the digest; the matching Mac binds.
#  13. the run path every consumer Mac takes (the liveness half of audit 6):
#      `aether run` with the bundled pair starts and stores the record; with
#      no record anywhere it WARNS and still starts (a shareless Mac cannot
#      vote — it verifies blocks by certificate); with a mismatching bundled
#      record it refuses; a Mac seated by a later reshare round restarts
#      cleanly on the bundled record alone; and the run path still refuses a
#      signer with no record anywhere (test c).
#  14. the release gate: `aether mainnet-rules --bundle` demands the
#      coordinator's record next to the network.json, pinning its exact
#      bytes — missing or mismatching bundled records FAIL the gate, the
#      matching pair passes (27 rules), and the legacy testnet bundle
#      (apps/wallet/Resources, chain 7780) passes with no record.
#  15. checklist B6, the release pin: the DKG's final file carries the
#      `release` object assemble wrote (ReleaseLog 0x…7705, its code hash,
#      the three builder keys, 2/3 and 3/3); assemble refuses to run without
#      --release; a final file with the pin removed or its code hash changed
#      FAILS the strict gate naming "release pin".
#  16. pre-audit 7 PA7-01, the assembly-intent boundary: a chain-id-only
#      ceremony record is refused as incomplete; a final file whose registrar
#      was replaced with ANOTHER VALID P-256 key — every strict rule still
#      passes — is refused by the intent comparison, and so is one whose
#      builder approval set was replaced with three other valid keys (the
#      release rules check shape, never intended identity).
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
  if [ "$n" = 26 ]; then ok "rehearsal-mode check passes: all 26 rules on (21 genesis + release pin + 4 final-file)"; else bad "rehearsal check passed but printed $n ok lines (want 26)"; fi
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
# rehearsal id is refused before any rule runs, see test 2). The ceremony
# record is the dry run's COMPLETE assembly policy with the new id — since
# pre-audit 7 (PA7-01) a chain-id-only record is refused as incomplete (test 16).
python3 - "$WORK/dry/coordinator/ceremony.json" "$WORK/ceremony7801.json" <<'PY'
import json, sys
c = json.load(open(sys.argv[1]))
c["chain_id"] = 7801            # a new id: what the real launch assembles
json.dump(c, open(sys.argv[2], "w"), indent=2)
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
  --chain-id 7801 --registrar "$reg" --reserve-operator "$founder" --release "$WORK/dry/coordinator/release.json" \
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
python3 - "$NET" "$WORK/final7801.json" <<'PY'
import json, sys
n = json.load(open(sys.argv[1]))
n["chain_id"] = 7801            # a new id: what the real launch assembles
json.dump(n, open(sys.argv[2], "w"), indent=2)
PY
if out=$("$A" mainnet-rules --network "$WORK/final7801.json" 2>&1); then
  n=$(printf '%s\n' "$out" | grep -c '^ok' || true)
  if [ "$n" = 26 ]; then ok "strict check passes the new-id final file: 26 rules on"; else bad "strict check passed but printed $n ok lines (want 26)"; fi
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
# A validator Mac's data dir for chain 7801: this Mac's own DKG share plus the
# final file as its network.json (the dry run's dirs are chain 7799; a 7801
# record must not bind a 7799 local file).
mkdir -p "$WORK/v7801"
cp "$WORK/dry/v2/threshold.json" "$WORK/v7801/"
cp "$WORK/final7801.json" "$WORK/v7801/network.json"
if "$G" verify-local "$WORK/final7801.json" --data "$WORK/v7801" --ceremony "$WORK/ceremony-check.json" > "$WORK/verify-ok.out" 2>&1; then
  ok "verify-local passes with the record (--ceremony) and this Mac's own share"
else
  bad "verify-local failed on the matching share:"; sed 's/^/        /' "$WORK/verify-ok.out" | tail -8
fi
if grep -q "VERIFY-LOCAL PASS" "$WORK/verify-ok.out" && [ -f "$WORK/v7801/ceremony-check.json" ]; then
  ok "the record is stored in the data dir (aether run binds to it on its next start)"
else
  bad "verify-local did not pass cleanly or did not store the record in the data dir"
fi
python3 - "$WORK/dry/v1/threshold.json" "$WORK/mismatch-dir" <<'PY'
import json, os, sys
src, dst = sys.argv[1], sys.argv[2]
os.makedirs(dst, exist_ok=True)
t = json.load(open(src))
t["round"] += 1               # a stale round: the committee the final file does not name
json.dump(t, open(os.path.join(dst, "threshold.json"), "w"), indent=2)
PY
cp "$WORK/final7801.json" "$WORK/mismatch-dir/network.json"
expect_fail "verify-local refuses a share from another round" "$G" verify-local "$WORK/final7801.json" --data "$WORK/mismatch-dir" --ceremony "$WORK/ceremony-check.json"
if grep -q "round" "$WORK/last.out"; then ok "the refusal names the round mismatch"
else bad "the mismatch refusal does not name the round"; fi
expect_fail "verify-local refuses the rehearsal chain id too" "$G" verify-local "$NET" --data "$WORK/dry/v1" --ceremony "$WORK/dry/v1/ceremony-check.json"

echo "== 12. audit 6 A6-3/A6-4: no validator votes from a genesis the ceremony did not check"
# (a) --ceremony is required: the expected chain id comes from the record,
#     never from the file being verified.
expect_fail "verify-local without --ceremony is refused" "$G" verify-local "$WORK/final7801.json" --data "$WORK/v7801"
if grep -q -- "--ceremony" "$WORK/last.out"; then ok "the refusal names the missing --ceremony record"
else bad "the no-record refusal does not name --ceremony"; fi
# (b) a chain id swapped in transit (7801 -> 7802): a self-derived id accepted
#     the swapped file; the record's pinned id refuses it.
python3 - "$WORK/final7801.json" "$WORK/swapped7802.json" <<'PY'
import json, sys
n = json.load(open(sys.argv[1]))
n["chain_id"] = 7802
json.dump(n, open(sys.argv[2], "w"), indent=2)
PY
expect_fail "verify-local refuses a chain id swapped in transit" "$G" verify-local "$WORK/swapped7802.json" --data "$WORK/v7801" --ceremony "$WORK/ceremony-check.json"
if grep -q "^FAIL  chain id" "$WORK/last.out"; then ok "the strict check fails the swapped id (pinned by the record)"
else bad "the transit-swap refusal does not fail the chain id rule"; fi
expect_fail "the node's own startup bind refuses the swapped file" "$A" mainnet-bind --network "$WORK/swapped7802.json" --data "$WORK/v7801" --ceremony "$WORK/ceremony-check.json"
if grep -q "7801" "$WORK/last.out" && grep -q "7802" "$WORK/last.out" && grep -q "ceremony" "$WORK/last.out"; then
  ok "the refusal names both chains and the record (A6-3)"
else bad "the transit-swap refusal does not name the record-pinned chain"; fi
# (c) node startup on a new genesis with no record anywhere: the operator step.
# The --network file must sit alone: the record also resolves from next to it
# (the app-bundle pair), and test 9's check left ceremony-check.json beside
# $WORK/final7801.json — with that, this start is refused for a missing LOCAL
# network.json instead of the missing record, a different scenario.
mkdir -p "$WORK/nowhere7801"
cp "$WORK/final7801.json" "$WORK/nowhere7801/network.json"
mkdir -p "$WORK/bare7801"
expect_fail "node startup refuses a new genesis with no ceremony record" "$A" node --network "$WORK/nowhere7801/network.json" --data "$WORK/bare7801" --port 0 --rpc-port 0
if grep -q "refusing to start" "$WORK/last.out" && grep -q "verify-local" "$WORK/last.out"; then
  ok "the startup refusal names the operator step (verify-local --ceremony)"
else bad "the startup refusal does not explain the operator step"; fi
# (d) this Mac's local files, against the same record: missing, malformed,
#     another chain, a stale genesis — each refused, never skipped (A6-4).
mkdir -p "$WORK/no-local-net"; cp "$WORK/dry/v2/threshold.json" "$WORK/no-local-net/"
expect_fail "the bind refuses a missing local network.json" "$A" mainnet-bind --network "$WORK/final7801.json" --data "$WORK/no-local-net" --ceremony "$WORK/ceremony-check.json"
if grep -q "network.json" "$WORK/last.out"; then ok "the refusal names the missing local network.json"
else bad "the missing-local refusal does not name network.json"; fi
mkdir -p "$WORK/torn-local-net"; cp "$WORK/dry/v2/threshold.json" "$WORK/torn-local-net/"; printf '{torn write' > "$WORK/torn-local-net/network.json"
expect_fail "the bind refuses a malformed local network.json" "$A" mainnet-bind --network "$WORK/final7801.json" --data "$WORK/torn-local-net" --ceremony "$WORK/ceremony-check.json"
python3 - "$WORK/final7801.json" "$WORK/local7802.json" <<'PY'
import json, sys
n = json.load(open(sys.argv[1]))
n["chain_id"] = 7802           # this Mac's local file is from another chain
json.dump(n, open(sys.argv[2], "w"), indent=2)
PY
mkdir -p "$WORK/other-chain"; cp "$WORK/dry/v2/threshold.json" "$WORK/other-chain/"; cp "$WORK/local7802.json" "$WORK/other-chain/network.json"
expect_fail "the bind refuses a local network.json of another chain" "$A" mainnet-bind --network "$WORK/final7801.json" --data "$WORK/other-chain" --ceremony "$WORK/ceremony-check.json"
if grep -q "7802" "$WORK/last.out" && grep -q "chain" "$WORK/last.out"; then ok "the refusal names the other chain"
else bad "the other-chain refusal does not name the chain"; fi
python3 - "$WORK/final7801.json" "$WORK/stale-genesis.json" <<'PY'
import json, sys
n = json.load(open(sys.argv[1]))
n["history"] = 1               # same id and committee identity, an older rule set
json.dump(n, open(sys.argv[2], "w"), indent=2)
PY
mkdir -p "$WORK/stale-genesis"; cp "$WORK/dry/v2/threshold.json" "$WORK/stale-genesis/"; cp "$WORK/stale-genesis.json" "$WORK/stale-genesis/network.json"
expect_fail "the bind refuses a stale local genesis (same id+identity, another history)" "$A" mainnet-bind --network "$WORK/final7801.json" --data "$WORK/stale-genesis" --ceremony "$WORK/ceremony-check.json"
if grep -q "stale" "$WORK/last.out"; then ok "the refusal calls the file stale, not 'the same network' (A6-4)"
else bad "the stale-genesis refusal does not say stale"; fi
# (e) same roster and round, a different committee output: the digest refuses.
python3 - "$WORK/final7801.json" "$WORK/other-output.json" <<'PY'
import json, sys
n = json.load(open(sys.argv[1]))
n["output"] = "ab" * 48
json.dump(n, open(sys.argv[2], "w"), indent=2)
PY
expect_fail "the bind refuses an output the coordinator did not check" "$A" mainnet-bind --network "$WORK/other-output.json" --data "$WORK/v7801" --ceremony "$WORK/ceremony-check.json"
if grep -q "digest" "$WORK/last.out"; then ok "the refusal names the digest (not the bytes the check passed)"
else bad "the other-output refusal does not name the digest"; fi
# (f) and the matching Mac binds — the same call the node makes at startup.
if "$A" mainnet-bind --network "$WORK/final7801.json" --data "$WORK/v7801" --ceremony "$WORK/ceremony-check.json" > "$WORK/bind-ok.out" 2>&1; then
  ok "the matching Mac binds (aether mainnet-bind = the node's startup gate)"
else
  bad "the matching Mac failed to bind:"; sed 's/^/        /' "$WORK/bind-ok.out" | tail -6
fi

echo "== 13. the run path every consumer Mac takes (the liveness half of audit 6)"
# The app-bundle pair: the final network.json and the coordinator's record
# shipped beside it (what `aether run` resolves on a fresh consumer Mac).
mkdir -p "$WORK/bundle"
cp "$WORK/final7801.json" "$WORK/bundle/network.json"
cp "$WORK/ceremony-check.json" "$WORK/bundle/ceremony-check.json"
# runs_and_lives <label> <data-dir> <network-file> <log-file>: `aether run` must
# get past the bind and stay in its supervisor loop (macOS has no timeout(1)).
runs_and_lives() {
  local label=$1 data=$2 network=$3 log=$4
  "$A" run --data "$data" --network "$network" --port 0 --rpc-port 0 --exit-with-parent --min-free-disk=0 >"$log" 2>&1 &
  local pid=$!
  sleep 6
  if ! grep -q "refusing to run" "$log" && kill -0 "$pid" 2>/dev/null && [ "$(ps -o stat= -p "$pid" 2>/dev/null)" != "Z" ]; then
    ok "$label"
  else
    bad "$label (it refused or exited):"; sed 's/^/        /' "$log" | head -8
  fi
  kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true
}
# (a1) the bundled pair: starts, and stores the record in the data dir (the
#      next start binds to it without --network).
runs_and_lives "run starts a consumer Mac on the bundled pair" "$WORK/consumer1" "$WORK/bundle/network.json" "$WORK/run-consumer1.log"
if [ -f "$WORK/consumer1/ceremony-check.json" ]; then
  ok "the bundled record is stored in the data dir (restarts need no --network)"
else
  bad "run started but stored no record in the data dir"
fi
# (a2) no record anywhere: a WARN, and it still follows (a shareless Mac
#      cannot vote — it verifies blocks by certificate).
mkdir -p "$WORK/bundle2"; cp "$WORK/final7801.json" "$WORK/bundle2/network.json"
runs_and_lives "run follows with no record anywhere (warns, never refuses)" "$WORK/consumer2" "$WORK/bundle2/network.json" "$WORK/run-consumer2.log"
if grep -q "no ceremony record" "$WORK/run-consumer2.log"; then
  ok "the no-record start says WHY it keeps following (the warn)"
else
  bad "the no-record start did not warn about the missing record"
fi
# (a3) a mismatching bundled record (pins other bytes): refuse, naming why.
mkdir -p "$WORK/bundle3"
python3 - "$WORK/final7801.json" "$WORK/bundle3/network.json" <<'PY'
import json, sys
n = json.load(open(sys.argv[1]))
n["output"] = "ab" * 48      # other bytes than the ones the check passed
json.dump(n, open(sys.argv[2], "w"), indent=2)
PY
cp "$WORK/ceremony-check.json" "$WORK/bundle3/ceremony-check.json"
rc=0; "$A" run --data "$WORK/consumer3" --network "$WORK/bundle3/network.json" --port 0 --rpc-port 0 --exit-with-parent --min-free-disk=0 > "$WORK/run-consumer3.log" 2>&1 || rc=$?
# The incoming-file bind refuses through the top-level error path ("error:
# digest mismatch: …"); the "refusing to run" wrap is the later, adopted-file
# stage. Either way the refusal must name the digest.
if [ "$rc" != 0 ] && grep -q "digest mismatch" "$WORK/run-consumer3.log"; then
  ok "run refuses a bundled record that pins other bytes (names the digest)"
else
  bad "the mismatching bundled record was not refused (rc=$rc):"; sed 's/^/        /' "$WORK/run-consumer3.log" | head -6
fi
# (b) a Mac seated by a later reshare round: round-5 local files, the round-0
#     bundled record — restarts cleanly, no operator step.
mkdir -p "$WORK/seated"
python3 - "$WORK/final7801.json" "$WORK/seated/network.json" "$WORK/dry/v2/threshold.json" "$WORK/seated/threshold.json" <<'PY'
import json, sys
n = json.load(open(sys.argv[1])); n["round"] += 5      # the reshare's round
json.dump(n, open(sys.argv[2], "w"), indent=2)
t = json.load(open(sys.argv[3])); t["round"] = n["round"]
json.dump(t, open(sys.argv[4], "w"), indent=2)
PY
if "$A" mainnet-bind --network "$WORK/seated/network.json" --data "$WORK/seated" --ceremony "$WORK/bundle/ceremony-check.json" > "$WORK/seated-bind.out" 2>&1; then
  ok "a reshare-seated Mac binds on the bundled record alone (round > record round)"
else
  bad "the seated Mac failed to bind:"; sed 's/^/        /' "$WORK/seated-bind.out" | head -6
fi
runs_and_lives "run restarts a reshare-seated Mac on the bundled pair" "$WORK/seated" "$WORK/bundle/network.json" "$WORK/run-seated.log"
# (c) the run path still refuses a signer with no record anywhere (test c).
mkdir -p "$WORK/gated-signer"; cp "$WORK/dry/v2/threshold.json" "$WORK/gated-signer/"; cp "$WORK/final7801.json" "$WORK/gated-signer/network.json"
rc=0; "$A" run --data "$WORK/gated-signer" --port 0 --rpc-port 0 --exit-with-parent --min-free-disk=0 > "$WORK/run-gated.log" 2>&1 || rc=$?
if [ "$rc" != 0 ] && grep -q "refusing to run" "$WORK/run-gated.log" && grep -q "verify-local" "$WORK/run-gated.log"; then
  ok "run still refuses a signer with no record (naming verify-local)"
else
  bad "the signer's no-record start was not refused (rc=$rc):"; sed 's/^/        /' "$WORK/run-gated.log" | head -6
fi

echo "== 14. the release gate: mainnet-rules --bundle pins the bundled record to the file"
if out=$("$A" mainnet-rules --bundle --network "$WORK/bundle/network.json" 2>&1); then
  n=$(printf '%s\n' "$out" | grep -c '^ok' || true)
  if [ "$n" = 27 ]; then ok "the bundled pair passes the gate: 27 rules on (26 + the record)"; else bad "the gate passed but printed $n ok lines (want 27)"; fi
else
  bad "the gate failed on the matching bundled pair:"$'\n'"$out"
fi
expect_fail "the gate refuses a new-genesis bundle with no record" "$A" mainnet-rules --bundle --network "$WORK/bundle2/network.json"
if grep -q "bundled ceremony record" "$WORK/last.out"; then ok "the missing-record refusal names the bundled ceremony record"
else bad "the missing-record refusal does not name the bundled record"; fi
expect_fail "the gate refuses a record pinning other bytes" "$A" mainnet-rules --bundle --network "$WORK/bundle3/network.json"
if grep -q "digest" "$WORK/last.out"; then ok "the mismatch refusal names the digest"
else bad " the mismatch refusal does not name the digest"; fi
# The legacy testnet app bundle (chain 7780) ships no record — and must not.
if out=$("$A" mainnet-rules --bundle --network "$ROOT/apps/wallet/Resources/network.json" 2>&1); then
  if printf '%s\n' "$out" | grep -q "not a new genesis"; then
    ok "the legacy 7780 app bundle passes the gate with no record (not a new genesis)"
  else
    bad "the 7780 gate pass did not explain that it ships no record"
  fi
else
  bad "the legacy 7780 bundle failed the gate:"$'\n'"$out"
fi

echo "== 15. checklist B6: the release pin rides from assemble to the final file, and the gate demands it"
if python3 - "$NET" "$WORK/dry/coordinator/release.json" <<'PY'
import json, sys
n, cfg = json.load(open(sys.argv[1])), json.load(open(sys.argv[2]))
r = n["release"]
assert r["log"] == "0x0000000000000000000000000000000000007705", r["log"]
assert r["code_hash"] == "0x4417ad7040420fe3547cdc3fdcd0fa0a690ba2f65e98af851a5e9fa5589db1ec", r["code_hash"]
assert r["builder_keys"] == [k.lower() for k in cfg["builder_keys"]]
assert (r["threshold"], r["emergency_threshold"]) == (2, 3)
PY
then ok "the DKG's final file carries the release pin assemble wrote (ReleaseLog, code hash, the three keys, 2/3 and 3/3)"
else bad "the final file does not carry the release pin from the release config"; fi
expect_fail "assemble refuses to run without --release" "$G" assemble \
  --chain-id 7801 --registrar "$reg" --reserve-operator "$founder" \
  --reserve "$WORK/dry/coordinator/pub/r1.pub.json" --reserve "$WORK/dry/coordinator/pub/r2.pub.json" \
  --reserve "$WORK/dry/coordinator/pub/r3.pub.json" \
  --validator "$WORK/dry/coordinator/pub/v1.pub.json" --validator "$WORK/dry/coordinator/pub/v2.pub.json" \
  --validator "$WORK/dry/coordinator/pub/v3.pub.json" --validator "$WORK/dry/coordinator/pub/v4.pub.json" \
  --out "$WORK/norelease"
if grep -q "\-\-release" "$WORK/last.out"; then ok "the refusal names --release"
else bad "the missing-release refusal does not name --release"; fi
python3 - "$WORK/final7801.json" "$WORK/nopin.json" "$WORK/badhash.json" <<'PY'
import json, sys
n = json.load(open(sys.argv[1]))
bad = json.loads(json.dumps(n))
del n["release"]
json.dump(n, open(sys.argv[2], "w"), indent=2)
bad["release"]["code_hash"] = "0x" + "ab" * 32
json.dump(bad, open(sys.argv[3], "w"), indent=2)
PY
expect_fail "strict mainnet-rules refuses a final file with no release pin" "$A" mainnet-rules --network "$WORK/nopin.json"
if grep -q "FAIL  release pin" "$WORK/last.out"; then ok "the refusal names the release pin rule"
else bad "the missing-pin refusal does not name the release pin rule"; fi
expect_fail "strict mainnet-rules refuses a mismatched ReleaseLog code hash" "$A" mainnet-rules --network "$WORK/badhash.json"
if grep -q "does not match the ReleaseLog runtime code" "$WORK/last.out"; then ok "the refusal says the code hash does not match"
else bad "the mismatched-hash refusal does not explain itself"; fi

echo "== 16. pre-audit 7 PA7-01: the final file must be what this ceremony ASSEMBLED"
# (a) a chain-id-only record proves nothing about intent: refused as incomplete.
printf '{"chain_id": 7801}\n' > "$WORK/ceremony-only-id.json"
expect_fail "check refuses an incomplete (chain-id-only) ceremony record" \
  "$G" check "$WORK/final7801.json" --chain-id 7801 --ceremony "$WORK/ceremony-only-id.json"
if grep -q "incomplete assembly record" "$WORK/last.out"; then ok "the refusal says the assembly record is incomplete"
else bad "the incomplete-record refusal does not say incomplete"; fi
# (b) another VALID registrar: curve-checked (every strict rule passes), so
#     only the intent comparison can catch the substitution (PA7-01).
"$A" registrar-key --data "$WORK/other-registrar" > "$WORK/other-registrar.out"
other_reg=$(awk 'NR == 1 && $1 == "registrar" && $2 == "key" {print $3}' "$WORK/other-registrar.out")
if [ "${#other_reg}" = 128 ]; then
  python3 - "$WORK/final7801.json" "$WORK/other-registrar.json" "$other_reg" <<'PY'
import json, sys
n = json.load(open(sys.argv[1]))
n["registrar"] = sys.argv[3]    # another valid P-256 key — not the one assembled
json.dump(n, open(sys.argv[2], "w"), indent=2)
PY
  if out=$("$A" mainnet-rules --network "$WORK/other-registrar.json" 2>&1); then
    ok "setup: the substituted registrar PASSES every strict rule — only the intent comparison can catch it (PA7-01)"
  else
    bad "setup: the substituted registrar already fails strict rules, so the scenario is unreachable:"$'\n'"$out"
  fi
  expect_fail "check refuses a valid-but-substituted registrar" \
    "$G" check "$WORK/other-registrar.json" --chain-id 7801 --ceremony "$WORK/ceremony7801.json"
  if grep -q "registrar replaced" "$WORK/last.out"; then ok "the refusal names the replaced registrar"
  else bad "the substituted-registrar refusal does not name it"; fi
else
  bad "could not make a second valid registrar key for the substitution test"
fi
# (c) another VALID builder set (three fresh P-256 keys): the release rules
#     check shape, log, code hash, 2/3 and 3/3 — never intended identity.
others=() k=
for i in 1 2 3; do
  k=$(openssl ecparam -name prime256v1 -genkey -noout 2>/dev/null | openssl ec -pubout -outform DER 2>/dev/null | tail -c 65 | xxd -p -c 65)
  if [ "${#k}" != 130 ]; then break; fi
  others+=("$k")
done
if [ "${#others[@]}" = 3 ]; then
  python3 - "$WORK/final7801.json" "$WORK/other-builders.json" "${others[@]}" <<'PY'
import json, sys
n = json.load(open(sys.argv[1]))
n["release"]["builder_keys"] = sys.argv[3:]   # three other valid keys
json.dump(n, open(sys.argv[2], "w"), indent=2)
PY
  if out=$("$A" mainnet-rules --network "$WORK/other-builders.json" 2>&1); then
    ok "setup: the substituted builder set PASSES every strict rule — only the intent comparison can catch it (PA7-01)"
  else
    bad "setup: the substituted builder set already fails strict rules, so the scenario is unreachable:"$'\n'"$out"
  fi
  expect_fail "check refuses a valid-but-substituted builder set" \
    "$G" check "$WORK/other-builders.json" --chain-id 7801 --ceremony "$WORK/ceremony7801.json"
  if grep -q "release pin replaced" "$WORK/last.out"; then ok "the refusal names the replaced release pin"
  else bad "the substituted-builders refusal does not name it"; fi
else
  bad "could not make three fresh builder keys for the substitution test"
fi
# (d) the reserve ENDPOINT IDS substituted with other valid ones, every
#     reserve key and the operator kept (PA7B-01, pass 2): the roster still
#     parses (fresh keygen EndpointIds are valid curve points, no validator
#     overlap), so only the intent comparison can catch the swapped endpoints.
#     (A bytewise XOR is NOT enough for this scenario: an arbitrary 32 bytes
#     fails iroh's on-curve check, so the strict rules would catch it first.)
for i in 1 2 3; do "$A" keygen --data "$WORK/fresh$i" >/dev/null; done
python3 - "$WORK/final7801.json" "$WORK/other-reserve-nodes.json" \
  "$WORK/fresh1/validator.pub.json" "$WORK/fresh2/validator.pub.json" "$WORK/fresh3/validator.pub.json" <<'PY'
import json, sys
n = json.load(open(sys.argv[1]))
nodes = [json.load(open(p))["node"] for p in sys.argv[3:]]   # other VALID endpoint ids
for v, node in zip((n.get("reserve") or {}).get("validators", []), nodes):
    v["node"] = node
json.dump(n, open(sys.argv[2], "w"), indent=2)
PY
if out=$("$A" mainnet-rules --network "$WORK/other-reserve-nodes.json" 2>&1); then
  ok "setup: the substituted reserve node ids PASS every strict rule — only the intent comparison can catch them (PA7B-01)"
else
  bad "setup: the substituted reserve node ids already fail strict rules, so the scenario is unreachable:"$'\n'"$out"
fi
expect_fail "check refuses valid-but-substituted reserve node ids" \
  "$G" check "$WORK/other-reserve-nodes.json" --chain-id 7801 --ceremony "$WORK/ceremony7801.json"
if grep -q "reserve roster" "$WORK/last.out"; then ok "the refusal names the reserve roster"
else bad "the substituted-reserve-nodes refusal does not name it"; fi
# (e) the reserve KEYS substituted with other valid ones (fresh keygen keys —
#     ed25519 parsing is on-curve too), endpoints kept: same hole from the key side.
python3 - "$WORK/final7801.json" "$WORK/other-reserve-keys.json" \
  "$WORK/fresh1/validator.pub.json" "$WORK/fresh2/validator.pub.json" "$WORK/fresh3/validator.pub.json" <<'PY'
import json, sys
n = json.load(open(sys.argv[1]))
keys = [json.load(open(p))["key"].lower() for p in sys.argv[3:]]   # other VALID ed25519 keys
for v, k in zip((n.get("reserve") or {}).get("validators", []), keys):
    v["key"] = k
json.dump(n, open(sys.argv[2], "w"), indent=2)
PY
if out=$("$A" mainnet-rules --network "$WORK/other-reserve-keys.json" 2>&1); then
  ok "setup: the substituted reserve keys PASS every strict rule — only the intent comparison can catch them (PA7B-01)"
else
  bad "setup: the substituted reserve keys already fail strict rules, so the scenario is unreachable:"$'\n'"$out"
fi
expect_fail "check refuses valid-but-substituted reserve keys" \
  "$G" check "$WORK/other-reserve-keys.json" --chain-id 7801 --ceremony "$WORK/ceremony7801.json"
if grep -q "reserve roster" "$WORK/last.out"; then ok "the refusal names the reserve roster for keys too"
else bad "the substituted-reserve-keys refusal does not name it"; fi
# (f) the reserve OPERATOR replaced by another valid address: the accounting
#     rule checks it is an address, never that it is the assembled one.
python3 - "$WORK/final7801.json" "$WORK/other-reserve-operator.json" <<'PY'
import json, sys
n = json.load(open(sys.argv[1]))
op = n["reserve"]["operator"]
op = op[2:] if op.startswith("0x") else op
b = bytearray(bytes.fromhex(op))
b[19] ^= 0xff
n["reserve"]["operator"] = ("0x" if n["reserve"]["operator"].startswith("0x") else "") + b.hex()
json.dump(n, open(sys.argv[2], "w"), indent=2)
PY
if out=$("$A" mainnet-rules --network "$WORK/other-reserve-operator.json" 2>&1); then
  ok "setup: the substituted reserve operator PASSES every strict rule — only the intent comparison can catch it (PA7B-01)"
else
  bad "setup: the substituted reserve operator already fails strict rules, so the scenario is unreachable:"$'\n'"$out"
fi
expect_fail "check refuses a valid-but-substituted reserve operator" \
  "$G" check "$WORK/other-reserve-operator.json" --chain-id 7801 --ceremony "$WORK/ceremony7801.json"
if grep -q "reserve operator replaced" "$WORK/last.out"; then ok "the refusal names the replaced reserve operator"
else bad "the substituted-operator refusal does not name it"; fi

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
