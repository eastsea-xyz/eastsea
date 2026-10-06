#!/usr/bin/env bash
# The real mainnet genesis ceremony, across the several Macs of the launch
# (docs/ops/mainnet-launch.md "Ceremony with the script"). One subcommand per
# machine role; nothing here invents a policy value — every flag comes from the
# doc §2 and the defaults the strict rule check demands:
#   scripts/mainnet-genesis.sh keys [--data <dir>]
#       EACH validator Mac, once: creates this Mac's validator identity and
#       prints ONLY the public half. Copy <data>/validator.pub.json to the
#       coordinator; the secrets never leave this Mac.
#   scripts/mainnet-genesis.sh assemble \
#       --chain-id <new id> --registrar <x‖y hex from the signer Mac> \
#       --reserve-operator <founder address> --release <release.json> \
#       --reserve <pub.json> ×3 --validator <pub.json> ×4 [--out <dir>]
#       COORDINATOR: validates every input, builds genesis.json with the
#       published policy (protocol 3, history 2, node rewards, registry v3,
#       epoch 3600 blocks / min streak 24 / draw every 24 epochs as defaults —
#       no timing flag is ever passed) and refuses rehearsal-only values.
#       --release names the three app builders' P-256 keys
#       ({"builder_keys": [...]}, from each builder Mac's `builder-sign init`):
#       genesis.json carries the app's release-approval pin (ReleaseLog
#       predeploy, its code hash, the keys, 2/3 normal, 3/3 emergency —
#       docs/design/19, checklist B6) through the DKG into the shipped file.
#   scripts/mainnet-genesis.sh check <network.json> --chain-id <id> --ceremony <ceremony.json>
#       COORDINATOR, after the DKG: `aether mainnet-rules --network <file>`
#       STRICT (never --rehearsal) plus every genesis flag still carried by the
#       final file — the failure mode docs/ops/mainnet-launch.md §3 warns
#       about. --chain-id is required and must equal the id assemble recorded
#       in ceremony.json (a rehearsal/testnet id is refused outside --dry-run).
#       PASS/FAIL list; nonzero exit on any failure. On PASS it also writes
#       ceremony-check.json next to the file (aether ceremony-record): the
#       pin — chain id, DKG round, committee identity, the sha256 of the exact
#       bytes that passed, the immutable genesis — every validator Mac binds
#       to before it votes (audit 6, A6-3/A6-4). Public; copy it together
#       with the final network.json.
#   scripts/mainnet-genesis.sh verify-local <network.json> --data <dir> --ceremony <ceremony-check.json>
#       EACH validator Mac, before it votes: re-runs the strict check with the
#       chain id pinned by the RECORD — never read from the file being
#       verified, so a file swapped in transit cannot self-accept (A6-3) —
#       then binds this Mac to the checked genesis (aether mainnet-bind: the
#       record vs the file's bytes, vs this Mac's network.json/threshold.json;
#       the node makes the same refusal at startup). Stores the record at
#       <data>/ceremony-check.json, so `aether run` binds to the same
#       ceremony. Refuses the rehearsal and testnet chain ids like `check`.
#   scripts/mainnet-genesis.sh --dry-run [<dir>]
#       every subcommand on one machine with throwaway keys in a temp
#       dir, the genesis DKG over loopback included. REHEARSAL, not a launch
#       file: the only place --rehearsal is ever passed.
# Env: AETHER_BIN (default: <repo>/tmp/target/release/aether, then
# <repo>/target/release/aether, then aether on PATH).
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
A=${AETHER_BIN:-}
if [ -z "$A" ]; then
  for c in "$ROOT/tmp/target/release/aether" "$ROOT/target/release/aether" "$(command -v aether 2>/dev/null || true)"; do
    if [ -x "$c" ]; then A=$c; break; fi
  done
fi
if [ -z "$A" ] || [ ! -x "$A" ]; then echo "no aether binary: build it, or set AETHER_BIN" >&2; exit 1; fi

# The published mainnet policy (docs/ops/mainnet-launch.md §2; the one list in
# crates/node/src/mainnet.rs). The timing numbers are the DEFAULTS `aether
# network` applies when no timing flag is passed — this script never passes
# one, and refuses them, because the strict rule check demands exactly these.
PROTOCOL=3
HISTORY=2
EPOCH_BLOCKS=3600
MIN_STREAK=24
DRAW_EPOCHS=24
N_VALIDATORS=4
N_RESERVE=3
TESTNET_CHAIN_ID=7780      # the running testnet: a new genesis needs a new id
REHEARSAL_CHAIN_ID=7799    # scripts/mainnet-rehearsal.sh's default: rehearsal-only

REHEARSAL=0   # 1 only under --dry-run: --rehearsal may be passed there and nowhere else

die() { echo "error: $*" >&2; exit 1; }
usage() { sed -n '2,49p' "$0"; exit 1; }
fp() { printf '%s…' "${1:0:16}"; }
mode_of() { stat -f '%Lp' "$1" 2>/dev/null || stat -c '%a' "$1"; }

PASS=() FAIL=()
verdict() { # verdict <name> <0|1> <detail>
  if [ "$2" = 1 ]; then PASS+=("$1"); printf 'PASS  %s — %s\n' "$1" "$3"
  else FAIL+=("$1"); printf 'FAIL  %s — %s\n' "$1" "$3"; fi
}

# Read a validator.pub.json and print "key node" (key lowercased). Dies with a
# named-file message on anything that is not a public entry.
read_pub() {
  python3 - "$1" <<'PY'
import json, re, sys
p = sys.argv[1]
try:
    d = json.load(open(p))
except Exception as e:
    sys.exit(f"{p}: not valid JSON ({e})")
k, n = d.get("key"), d.get("node")
if not (isinstance(k, str) and re.fullmatch(r"[0-9a-fA-F]{64}", k)):
    sys.exit(f'{p}: "key" must be the ed25519 public key (64 hex chars), got {k!r}')
if not (isinstance(n, str) and n and not re.search(r"\s", n)):
    sys.exit(f'{p}: "node" must be this Mac\'s iroh node id (non-empty, no whitespace), got {n!r}')
print(k.lower(), n)
PY
}

# A throwaway release config for the dry run: three fresh P-256 keys nobody
# keeps (REHEARSAL only — the launch uses the builder Macs' Secure Enclave keys).
throwaway_release() { # throwaway_release <out.json>
  local keys=() k i
  for i in 1 2 3; do
    k=$(openssl ecparam -name prime256v1 -genkey -noout 2>/dev/null | openssl ec -pubout -outform DER 2>/dev/null | tail -c 65 | xxd -p -c 65)
    [ "${#k}" = 130 ] || die "could not make a throwaway P-256 key with openssl"
    keys+=("$k")
  done
  printf '{"builder_keys": ["%s", "%s", "%s"]}\n' "${keys[@]}" > "$1"
}

# Structural checks of a network.json, one verdict line each: "ok|name|detail"
# or "bad|name|detail". <expected chain id> may be empty (then only its shape
# is checked). <stage> is "genesis" (pre-DKG: identity must NOT be there yet)
# or "final" (post-DKG: identity and output must be). This is the machine half
# of a check; `aether mainnet-rules` is the other half.
net_verdicts() { # net_verdicts <file> <expected chain id> <genesis|final>
  python3 - "$1" "${2:-}" "$3" <<'PY'
import json, sys

def line(ok, name, detail):
    print(("ok" if ok else "bad") + "|" + name + "|" + detail)

try:
    n = json.load(open(sys.argv[1]))
except Exception as e:
    line(False, "file is a network.json", f"{sys.argv[1]}: not valid JSON ({e})")
    sys.exit(0)
want_id, stage = sys.argv[2], sys.argv[3]
vs = n.get("validators") or []
rs = (n.get("reserve") or {}).get("validators") or []
vkeys = [v.get("key") for v in vs]
rkeys = [v.get("key") for v in rs]

if want_id:
    line(n.get("chain_id") == int(want_id), "chain id",
         f"chain_id is {n.get('chain_id')}, want {want_id}" if n.get("chain_id") != int(want_id)
         else f"chain_id {want_id}")
else:
    line(isinstance(n.get("chain_id"), int) and n["chain_id"] > 0, "chain id", f"chain_id is {n.get('chain_id')!r}")
line(n.get("protocol") == 3, "protocol 3 flag",
     f'"protocol" is {n.get("protocol")!r}, want 3 (proof market, registration cap, 16-seat growth from height 0)')
line(n.get("history") == 2, "history 2 flag", f'"history" is {n.get("history")!r}, want 2 (quiet empty blocks, era files)')
line(n.get("node_rewards") is True, "node rewards flag", f'"node_rewards" is {n.get("node_rewards")!r}, want true')
line(n.get("faucet") is None, "no faucet flag", f'"faucet" is {n.get("faucet")!r}, want absent (no premine, no faucet)')
timing = [n.get(k) for k in ("epoch_blocks", "min_streak", "draw_epochs")]
line(all(t is None for t in timing), "published timing untouched",
     f"epoch_blocks/min_streak/draw_epochs are {timing}, want [None, None, None]: the published policy (3600 blocks, "
     "streak 24, draw every 24 epochs) is what the defaults give; an override is a rehearsal-only value")
line(len(vs) == 4, "four validators", f"{len(vs)} validators, want 4")
line(len(set(vkeys)) == 4 and len({v.get("node") for v in vs}) == 4, "validator keys distinct",
     f"{len(set(vkeys))} distinct keys / {len({v.get('node') for v in vs})} distinct nodes of 4")
line(len(rs) == 3, "three reserve keys", f"{len(rs)} reserve keys, want 3")
line(len(set(rkeys)) == 3, "reserve keys distinct", f"{len(set(rkeys))} distinct reserve keys of 3")
overlap = set(rkeys) & set(vkeys)
line(not overlap, "reserve keys outside the validator set",
     "none of the founder's reserve keys is a validator key" if not overlap
     else f"a reserve key doubles as a validator key ({sorted(overlap)[:1][0][:16]}…)")
if stage == "final":
    line(isinstance(n.get("identity"), str) and n["identity"] != "", "committee identity (post-DKG)",
         "the identity wallets pin is present" if n.get("identity")
         else 'no "identity": pre-DKG file — run `check` on the network.json the DKG wrote (docs/ops/mainnet-launch.md §3)')
    line(isinstance(n.get("output"), str) and n["output"] != "", "committee output (post-DKG)",
         "the DKG output is present" if n.get("output") else 'no "output": pre-DKG file')
else:
    line(n.get("identity") is None, "pre-DKG (identity not there yet)",
         "no committee identity yet — the genesis DKG writes it (doc §3)" if n.get("identity") is None
         else 'already has an "identity": not the fresh pre-DKG file assemble writes')
rel = n.get("release") or {}
keys = rel.get("builder_keys") or []
line(rel.get("log") == "0x0000000000000000000000000000000000007705" and len(keys) == 3
     and len({k.lower() for k in keys}) == 3 and rel.get("threshold") == 2 and rel.get("emergency_threshold") == 3,
     "release pin (checklist B6)",
     "the app trusts only ReleaseLog 0x…7705 and the three builder keys, 2/3 normal, 3/3 emergency" if rel
     else 'no "release": the app would refuse every update — assemble with --release <release.json>')
line(n.get("group") in (None, 0), "group 0", f'"group" is {n.get("group")!r}, want absent or 0')
line(n.get("max_committee") in (None, 16), "committee ceiling 16",
     f'"max_committee" is {n.get("max_committee")!r}, want absent or 16 (the growth target)')
PY
}

cmd_keys() {
  local data=$HOME/aether-mainnet f m
  while [ $# -gt 0 ]; do case $1 in
    --data) data=$2; shift 2 ;;
    *) die "keys: unknown option '$1' (usage: keys [--data <dir>])" ;;
  esac; done
  if [ -e "$data/validator.key" ]; then
    die "$data/validator.key exists: an identity is never overwritten. This directory already holds a key — \
use it (its public entry is $data/validator.pub.json) or move the whole directory aside first."
  fi
  echo "== validator identity (this Mac only)"
  mkdir -p "$data"
  # keygen prints the public entry plus where the secrets live; we discard its
  # output and print only the public half ourselves, from the file it wrote.
  "$A" keygen --data "$data" >/dev/null
  chmod 700 "$data"
  for f in validator.key node-account.key; do
    [ -f "$data/$f" ] || die "$data/$f missing: keygen did not write it"
    m=$(mode_of "$data/$f")
    [ "$m" = 600 ] || die "$data/$f has mode $m (want 600)"
  done
  read_pub "$data/validator.pub.json" >/dev/null   # shape-check before telling anyone to copy it
  echo
  echo "PUBLIC entry — the ONLY thing to copy to the coordinator (safe to share):"
  echo "  $data/validator.pub.json"
  cat "$data/validator.pub.json"; echo
  echo
  echo "SECRET files — never printed, never copied anywhere, never committed (mode 600):"
  echo "  $data/validator.key     this Mac's consensus signing key (the identity)"
  echo "  $data/node-account.key  the node account key (pays fees, sends beacons)"
  echo "  $data/threshold.json    appears here only after the genesis DKG (the secret share)"
  echo "Back both .key files up offline now (docs/ops/mainnet-launch.md §1): a lost validator key never sits again."
  echo "This Mac later runs the node with --data $data and the final network.json."
}

cmd_assemble() {
  local chain_id='' registrar='' rop='' release='' out=mainnet-genesis
  local vals resv net_args vkeys vnodes rkeys i j kn
  vals=() resv=() net_args=() vkeys=() vnodes=() rkeys=()
  while [ $# -gt 0 ]; do case $1 in
    --chain-id) chain_id=$2; shift 2 ;;
    --registrar) registrar=$2; shift 2 ;;
    --reserve-operator) rop=$2; shift 2 ;;
    --release) release=$2; shift 2 ;;
    --validator) vals+=("$2"); shift 2 ;;
    --reserve) resv+=("$2"); shift 2 ;;
    --out) out=$2; shift 2 ;;
    --epoch-blocks|--min-streak|--draw-epochs)
      die "$1 is a rehearsal-only shortcut (scripts/mainnet-rehearsal.sh shortens timing to finish in minutes). \
The mainnet runs the published timing — epoch $EPOCH_BLOCKS blocks, min streak $MIN_STREAK, draw every $DRAW_EPOCHS epochs — \
which is what the defaults give (docs/ops/mainnet-launch.md §2); the strict rule check demands exactly these numbers." ;;
    --dev-registrar)
      die "--dev-registrar is for local test networks only. The mainnet registrar is the signer Mac's \
Secure Enclave key: pass its x‖y hex (\`aether-registrar-signer public\`, docs/ops/registrar.md) as --registrar." ;;
    --faucet)
      die "--faucet is refused: the mainnet has no faucet and no premine — every token comes from issuance \
(docs/ops/mainnet-launch.md §2)." ;;
    *) die "assemble: unknown option '$1'" ;;
  esac; done
  [ -n "$chain_id" ] || die "assemble: --chain-id <new chain id> is required"
  [ -n "$registrar" ] || die "assemble: --registrar <x‖y hex> is required"
  [ -n "$rop" ] || die "assemble: --reserve-operator <founder address> is required"
  [ -n "$release" ] || die "assemble: --release <release.json> is required — {\"builder_keys\": [three 04‖x‖y hex keys]} from the three builder Macs' builder-sign init (docs/design/19; without it the app can never update)"
  [ -f "$release" ] || die "assemble: no release config at $release"
  case $chain_id in ''|*[!0-9]*|0) die "--chain-id must be a positive integer, got '$chain_id'" ;; esac
  if [ "$REHEARSAL" = 0 ]; then
    if [ "$chain_id" = "$TESTNET_CHAIN_ID" ]; then die "--chain-id $TESTNET_CHAIN_ID is the running testnet: a new genesis needs a new chain id."; fi
    if [ "$chain_id" = "$REHEARSAL_CHAIN_ID" ]; then die "--chain-id $REHEARSAL_CHAIN_ID is the rehearsal default: pick the real new chain id."; fi
  fi
  if [ "${#registrar}" != 128 ]; then die "--registrar must be the signer Mac's P-256 public key as x‖y hex (128 hex chars, from \`aether-registrar-signer public\`), got ${#registrar} chars"; fi
  case $registrar in *[!0-9a-fA-F]*) die "--registrar must be hex, got '$registrar'" ;; esac
  if [ "${#rop}" != 42 ] || [ "${rop:0:2}" != "0x" ]; then
    die "--reserve-operator must be the founder's address (0x + 40 hex chars), got '$rop'"
  fi
  case ${rop#0x} in *[!0-9a-fA-F]*) die "--reserve-operator must be hex, got '$rop'" ;; esac
  if [ "${#vals[@]}" != "$N_VALIDATORS" ]; then die "assemble: exactly $N_VALIDATORS --validator <validator.pub.json> files in validator order 1-$N_VALIDATORS (got ${#vals[@]})"; fi
  if [ "${#resv[@]}" != "$N_RESERVE" ]; then die "assemble: exactly $N_RESERVE --reserve <validator.pub.json> files, the founder Mac's from scripts/reserve-keys.sh init (got ${#resv[@]})"; fi

  echo "== validating the $N_VALIDATORS validator and $N_RESERVE reserve public entries"
  for i in $(seq 1 $N_VALIDATORS); do
    kn=$(read_pub "${vals[$((i - 1))]}")
    vkeys+=("$(printf '%s' "$kn" | awk '{print $1}')")
    vnodes+=("$(printf '%s' "$kn" | awk '{print $2}')")
  done
  for i in $(seq 1 $N_RESERVE); do
    kn=$(read_pub "${resv[$((i - 1))]}")
    rkeys+=("$(printf '%s' "$kn" | awk '{print $1}')")
  done
  for ((i = 0; i < N_VALIDATORS; i++)); do
    for ((j = i + 1; j < N_VALIDATORS; j++)); do
      if [ "${vkeys[$i]}" = "${vkeys[$j]}" ]; then
        die "validators $((i + 1)) and $((j + 1)) have the same key: each of the four Macs must run \`mainnet-genesis.sh keys\` on its own machine"
      fi
      if [ "${vnodes[$i]}" = "${vnodes[$j]}" ]; then
        die "validators $((i + 1)) and $((j + 1)) have the same node id: two public entries from one Mac"
      fi
    done
  done
  for ((i = 0; i < N_RESERVE; i++)); do
    for ((j = i + 1; j < N_RESERVE; j++)); do
      if [ "${rkeys[$i]}" = "${rkeys[$j]}" ]; then die "reserve keys $((i + 1)) and $((j + 1)) are the same: scripts/reserve-keys.sh init makes three distinct keys"; fi
    done
  done
  for ((i = 0; i < N_RESERVE; i++)); do
    for ((j = 0; j < N_VALIDATORS; j++)); do
      if [ "${rkeys[$i]}" = "${vkeys[$j]}" ]; then die "a reserve key is also validator $((j + 1))'s key: the founder's reserve keys must be their own keys"; fi
    done
  done

  echo "== assembling genesis.json (protocol $PROTOCOL, history $HISTORY, node rewards, registry v3; timing $EPOCH_BLOCKS/$MIN_STREAK/$DRAW_EPOCHS = published defaults, untouched)"
  mkdir -p "$out"
  # Exactly the doc §2 command: no --epoch-blocks/--min-streak/--draw-epochs
  # (the published defaults apply), no --faucet, no --dev-registrar.
  net_args=(network --chain-id "$chain_id" --protocol "$PROTOCOL" --history "$HISTORY" --node-rewards
    --registrar "$registrar" --reserve-operator "$rop" --release "$release")
  for i in $(seq 1 $N_RESERVE); do net_args+=(--reserve "${resv[$((i - 1))]}"); done
  for i in $(seq 1 $N_VALIDATORS); do net_args+=("${vals[$((i - 1))]}"); done
  "$A" "${net_args[@]}" > "$out/genesis.json"

  echo "== strict rule check on the assembled genesis (the doc §2 gate)"
  run_check "$out/genesis.json" "$chain_id" genesis

  echo
  echo "== genesis ready: $out/genesis.json (public — copy it to every validator Mac)"
  echo "  chain id          $chain_id"
  for i in $(seq 1 $N_VALIDATORS); do echo "  validator $i key    $(fp "${vkeys[$((i - 1))]}")"; done
  for i in $(seq 1 $N_RESERVE); do echo "  reserve key $i      $(fp "${rkeys[$((i - 1))]}")"; done
  echo "  registrar x‖y     $(fp "$registrar")"
  echo "  reserve operator  $rop"
  python3 - "$out/genesis.json" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))["release"]
print(f"  release log       {r['log']} (code hash {r['code_hash'][:18]}…)")
for i, k in enumerate(r["builder_keys"], 1):
    print(f"  builder key {i}     {k[:16]}…")
print(f"  release rule      {r['threshold']}/3 normal, {r['emergency_threshold']}/3 emergency")
PY
  echo "  timing            epoch $EPOCH_BLOCKS blocks · min streak $MIN_STREAK · draw every $DRAW_EPOCHS epochs (defaults, untouched)"
  echo
  # Pre-audit 7 (PA7-01): the COMPLETE intended policy, read back from the
  # genesis just assembled — `check` compares the DKG's final file against it
  # field by field, so a valid-but-substituted registrar, roster, reserve or
  # builder approval set cannot ride the ceremony through (the strict rules
  # check shape and policy compliance, never intended identity).
  python3 - "$out/genesis.json" "$out/ceremony.json" <<'PY'
import datetime, json, sys
g = json.load(open(sys.argv[1]))
rel = g.get("release") or {}
json.dump({
    "chain_id": g["chain_id"],
    "assembled": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
    "registrar": (g.get("registrar") or "").lower(),
    "reserve_operator": ((g.get("reserve") or {}).get("operator") or "").lower(),
    "validators": [{"key": v["key"].lower(), "node": v["node"]} for v in g["validators"]],
    # The reserve is recorded as a complete {key, node} roster (pre-audit 7b
    # PA7B-01): keys alone would accept a final file whose reserve endpoints
    # were swapped for other valid ids — a delay to the intended fallback.
    "reserve": [{"key": v["key"].lower(), "node": v["node"]} for v in (g.get("reserve") or {}).get("validators", [])],
    "protocol": g.get("protocol"),
    "history": g.get("history"),
    "node_rewards": g.get("node_rewards"),
    "release": {
        "log": (rel.get("log") or "").lower(),
        "code_hash": (rel.get("code_hash") or "").lower(),
        "builder_keys": [(k or "").lower() for k in (rel.get("builder_keys") or [])],
        "threshold": rel.get("threshold"),
        "emergency_threshold": rel.get("emergency_threshold"),
    },
}, open(sys.argv[2], "w"), indent=2)
PY
  echo "== ceremony record: $out/ceremony.json (public: the COMPLETE intended policy — check compares the DKG's final file against it)"
  echo
  echo "NEXT — the genesis DKG (doc §3): copy $out/genesis.json to all $N_VALIDATORS validator Macs and run ON ALL FOUR AT ONCE,"
  echo "each on its own Mac with its own --data dir from \`mainnet-genesis.sh keys\`:"
  echo "  aether dkg --network <genesis.json> --port <this Mac's p2p port> --data <this Mac's data dir> \\"
  echo "    --peers 1@<ip1>:<port>,2@<ip2>:<port>,3@<ip3>:<port>,4@<ip4>:<port>"
  echo "THEN copy validator 1's <data>/network.json back here and run the strict final check:"
  echo "  scripts/mainnet-genesis.sh check <network.json> --chain-id $chain_id --ceremony $out/ceremony.json"
  echo "  (on PASS it writes ceremony-check.json next to the file — the record every Mac binds to)"
  echo "THEN, on EACH validator Mac before it votes (doc §3):"
  echo "  scripts/mainnet-genesis.sh verify-local <the same final network.json> \\"
  echo "    --data <this Mac's data dir> --ceremony <ceremony-check.json from the coordinator>"
}

cmd_check() {
  local file='' chain_id='' ceremony=''
  while [ $# -gt 0 ]; do case $1 in
    --chain-id) chain_id=$2; shift 2 ;;
    --ceremony) ceremony=$2; shift 2 ;;
    -*) die "check: unknown option '$1'" ;;
    *) if [ -n "$file" ]; then die "check: exactly one network.json"; fi; file=$1; shift ;;
  esac; done
  [ -n "$file" ] || usage
  [ -f "$file" ] || die "check: no file at $file"
  [ -n "$chain_id" ] || die "check: --chain-id <id> is required — the id assemble recorded in ceremony.json (audit 5, A5-4: a check without the intended id used to pass any positive id)"
  case $chain_id in ''|*[!0-9]*|0) die "--chain-id must be a positive integer, got '$chain_id'" ;; esac
  [ -n "$ceremony" ] || die "check: --ceremony <ceremony.json> is required — the record assemble wrote next to genesis.json"
  [ -f "$ceremony" ] || die "check: no ceremony record at $ceremony"
  # The ceremony record pins the intended chain id (and nothing secret).
  python3 - "$ceremony" "$chain_id" "$REHEARSAL" <<'PY'
import json, sys
cer, want, rehearsal = sys.argv[1], int(sys.argv[2]), sys.argv[3] == "1"
try:
    c = json.load(open(cer))
except Exception as e:
    sys.exit(f"{cer}: not valid JSON ({e})")
got = c.get("chain_id")
if got != want:
    sys.exit(f"{cer}: this ceremony assembled chain_id {got!r}, but --chain-id says {want}: "
             "check the file THIS ceremony wrote, not another network.json")
if not rehearsal and want in (7780, 7799):
    sys.exit(f"chain id {want} is reserved (testnet 7780, rehearsal 7799): the mainnet launch needs its own id")
PY
  if [ "$REHEARSAL" = 0 ]; then
    case $chain_id in
      "$TESTNET_CHAIN_ID") die "check: chain id $TESTNET_CHAIN_ID is the running testnet, not a new genesis" ;;
      "$REHEARSAL_CHAIN_ID") die "check: chain id $REHEARSAL_CHAIN_ID is the rehearsal default, not a launch id" ;;
    esac
  fi
  run_check "$file" "$chain_id" final || die "check failed: no ceremony record is written"
  # Pre-audit 7 (PA7-01): the rules above check that the final file is a VALID
  # mainnet genesis — not that it is THIS ceremony's. Before pinning it as the
  # record every Mac binds to, compare it against the intended assembly policy
  # the coordinator fixed at assemble: registrar, roster, reserve, the release
  # builder approval set, every rule flag. Only the DKG-generated fields
  # (identity, output, round, epochs) may differ. An incomplete ceremony.json
  # proves nothing about intent and is refused. This original-policy boundary
  # stays separate from the later byte-exact record binding (mainnet-bind).
  python3 - "$ceremony" "$file" <<'PY' || die "the final network.json is NOT the genesis this ceremony assembled — do not launch it"
import json, sys

cer, net = sys.argv[1], sys.argv[2]
def die(msg):
    sys.exit(msg)

try:
    c = json.load(open(cer))
except Exception as e:
    die(f"{cer}: not valid JSON ({e})")
try:
    n = json.load(open(net))
except Exception as e:
    die(f"{net}: not valid JSON ({e})")

# A chain id alone proves nothing about intent: refuse records that do not
# carry the complete policy assemble wrote (a bare {"chain_id": …} used to
# pass this gate, pre-audit 7 PA7-01).
need = ("chain_id", "registrar", "reserve_operator", "validators", "reserve",
        "protocol", "history", "node_rewards", "release")
missing = [f for f in need if f not in c]
if missing:
    die(f"{cer}: incomplete assembly record — no {', '.join(missing)}; regenerate it "
        "(assemble writes the complete intended policy next to genesis.json)")
rel = c["release"]
missing = [f for f in ("log", "code_hash", "builder_keys", "threshold", "emergency_threshold") if f not in rel]
if missing:
    die(f"{cer}: incomplete assembly record — release has no {', '.join(missing)}")

def lower(s):
    return (s or "").lower()

def roster(vs):
    return [{"key": lower(v.get("key")), "node": v.get("node")} for v in (vs or [])]

bad = []
if lower(n.get("registrar")) != lower(c["registrar"]):
    bad.append(f"registrar replaced: assembled with {lower(c['registrar'])[:16]}…, "
               f"the final file carries {lower(n.get('registrar'))[:16]}…")
if lower((n.get("reserve") or {}).get("operator")) != lower(c["reserve_operator"]):
    bad.append("reserve operator replaced")
want = roster(c["validators"])
if roster(n.get("validators")) != want:
    bad.append("the opening roster (validators) is not the one assembled")
if roster(n.get("genesis_validators")) != want:
    bad.append("genesis_validators is not the one assembled")
# The reserve is compared as a complete {key, node} roster (pre-audit 7b
# PA7B-01): keys alone let a substituted reserve endpoint id — valid,
# curve-checked, not the one assembled — ride the ceremony through.
if roster((n.get("reserve") or {}).get("validators")) != roster(c["reserve"]):
    bad.append("the reserve roster (keys and node ids) is not the one assembled")
for flag in ("protocol", "history", "node_rewards"):
    if n.get(flag) != c[flag]:
        bad.append(f"{flag} is {n.get(flag)!r}, the ceremony assembled {c[flag]!r}")

def pin(r):
    return {"log": lower(r.get("log")), "code_hash": lower(r.get("code_hash")),
            "builder_keys": [lower(k) for k in (r.get("builder_keys") or [])],
            "threshold": r.get("threshold"), "emergency_threshold": r.get("emergency_threshold")}
if pin(n.get("release") or {}) != pin(rel):
    bad.append("release pin replaced (the ReleaseLog, code hash or builder approval set "
               "the ceremony assembled is not what the final file carries)")
if bad:
    die("the final network.json differs from the assembly intent: " + "; ".join(bad))
PY
  # Audit 6, A6-3/A6-4: the independent pin every validator Mac binds to
  # before it votes — the coordinator's own tool cannot take it from the file
  # being checked (that is the transit-swap hole), so it derives it here, from
  # the bytes that just passed, and writes it next to the file.
  local rec
  rec="$(dirname "$file")/ceremony-check.json"
  "$A" ceremony-record --network "$file" --out "$rec"
  echo
  echo "== ceremony record: $rec"
  echo "    PUBLIC (no secret). Copy it TOGETHER with the final network.json to every"
  echo "    validator Mac: verify-local --ceremony and the node's startup bind refuse"
  echo "    to vote without it (audit 6: no validator votes from a genesis the"
  echo "    ceremony did not check)."
  echo "    The app bundle ships the same pair: copy BOTH unchanged into"
  echo "    apps/wallet/Resources/ for the release build (docs/ops/mainnet-launch.md"
  echo "    6단계) — scripts/build-wallet.sh runs mainnet-rules --bundle on it, so a"
  echo "    build without the matching record does not ship."
}

# Each validator Mac, before it votes: the strict check on the final file it
# received, with the chain id pinned by the coordinator's RECORD (never taken
# from the file being verified — that is A6-3's transit-swap hole), then the
# one fail-closed bind (aether mainnet-bind): the record against the file's
# exact bytes, this Mac's network.json and threshold.json. The node makes the
# same refusal at startup. Never prints the share.
cmd_verify_local() {
  local file='' data='' ceremony=''
  while [ $# -gt 0 ]; do case $1 in
    --data) data=$2; shift 2 ;;
    --ceremony) ceremony=$2; shift 2 ;;
    -*) die "verify-local: unknown option '$1'" ;;
    *) if [ -n "$file" ]; then die "verify-local: exactly one network.json"; fi; file=$1; shift ;;
  esac; done
  [ -n "$file" ] || usage
  [ -f "$file" ] || die "verify-local: no file at $file"
  [ -n "$data" ] || die "verify-local: --data <this Mac's data dir> is required (where threshold.json lives)"
  [ -f "$data/threshold.json" ] || die "verify-local: no $data/threshold.json — run the genesis DKG on this Mac first (doc §3)"
  [ -n "$ceremony" ] || die "verify-local: --ceremony <ceremony-check.json> is required — the record the coordinator's check wrote next to the final network.json. Audit 6, A6-3: the expected chain id comes from the record, never from the file being verified."
  [ -f "$ceremony" ] || die "verify-local: no ceremony record at $ceremony"
  echo "== verify-local: the strict rule check, run on THIS Mac against the final file"
  # A6-3: the expected chain id comes from the RECORD, never from the file
  # being verified — a file swapped in transit cannot self-accept.
  chain_id=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["chain_id"])' "$ceremony")
  case $chain_id in ''|*[!0-9]*) die "$ceremony: no chain_id — not a ceremony record (regenerate it with check)" ;; esac
  run_check "$file" "$chain_id" final
  echo "== verify-local: binding this Mac to the checked genesis (never prints the share)"
  "$A" mainnet-bind --network "$file" --data "$data" --ceremony "$ceremony"
  # Keep the record with the data: the wallet's `aether run` (no --network,
  # no --ceremony) binds against this copy on its next start. The coordinator
  # may be validator 1 itself, whose check already wrote the record into this
  # data dir — then it is already in place (cp refuses to copy onto itself).
  if ! cp "$ceremony" "$data/ceremony-check.json" 2>/dev/null \
     && ! cmp -s "$ceremony" "$data/ceremony-check.json"; then
    die "cannot store the record at $data/ceremony-check.json"
  fi
  echo "VERIFY-LOCAL PASS: this Mac votes under the committee of the genesis the ceremony checked"
  echo "  (the record is stored at $data/ceremony-check.json; aether run binds to it)"
}

run_check() { # run_check <network.json> [expected chain id] <genesis|final>
  local file=$1 want=$2 stage=$3 out line st rest name detail
  local mode=STRICT tag=""
  if [ "$REHEARSAL" = 1 ]; then mode=REHEARSAL; tag=" (--rehearsal passed: allowed only in the dry run)"; fi
  echo "== aether mainnet-rules ($mode)$tag"
  local rules
  rules=(mainnet-rules --network "$file")
  if [ "$REHEARSAL" = 1 ]; then rules=(mainnet-rules --rehearsal --network "$file"); fi
  if out=$("$A" "${rules[@]}" 2>&1); then
    verdict "aether mainnet-rules, the full rule set (21 genesis + release pin + 4 final-file rules)" 1 "every rule on ($(printf '%s\n' "$out" | grep -c '^ok') ok; the final-file gate decodes the committee output, seats the roster, pins the identity, refuses revealed shares)$tag"
  else
    verdict "aether mainnet-rules, the full rule set (21 genesis + release pin + 4 final-file rules)" 0 "$mode check failed:$tag"$'\n'"$(printf '%s\n' "$out" | sed 's/^/    /')"
  fi
  echo "== the file carries every genesis flag (stage: $stage)"
  out=$(net_verdicts "$file" "$want" "$stage")
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    st=${line%%|*}; rest=${line#*|}; name=${rest%%|*}; detail=${rest#*|}
    verdict "$name" "$([ "$st" = ok ] && echo 1 || echo 0)" "$detail"
  done <<< "$out"
  echo
  if [ "${#FAIL[@]}" = 0 ]; then
    echo "CHECK PASS: ${#PASS[@]}/${#PASS[@]} — $file satisfies the strict mainnet rule set"
    if [ "$REHEARSAL" = 1 ]; then
      echo "  …but this ran in --dry-run mode: REHEARSAL, not a launch file (throwaway keys; never launch with it)."
    fi
    PASS=() FAIL=()
    return 0
  fi
  echo "CHECK FAIL: ${#FAIL[@]} of $(( ${#PASS[@]} + ${#FAIL[@]} )) checks failed — do NOT launch with $file"
  for f in "${FAIL[@]}"; do echo "  - $f"; done
  PASS=() FAIL=()
  return 1
}

cmd_dry_run() {
  local dir=${1:-} i j peers ports p founder registrar_hex bad pid
  if [ -z "$dir" ]; then mkdir -p "$ROOT/tmp"; dir=$(mktemp -d "$ROOT/tmp/mainnet-genesis-dry.XXXXXX"); fi
  mkdir -p "$dir"; dir=$(cd "$dir" && pwd)
  echo "******************************************************************"
  echo "*   REHEARSAL, not a launch file — the whole ceremony on ONE     *"
  echo "*   machine with throwaway keys in $dir"
  echo "*   Everything below is throwaway; delete the directory after.   *"
  echo "******************************************************************"
  REHEARSAL=1
  echo
  echo "== [keys] ×$N_VALIDATORS (what each validator Mac runs)"
  for i in $(seq 1 $N_VALIDATORS); do echo "-- validator $i"; cmd_keys --data "$dir/v$i"; echo; done
  echo "== founder reserve keys ×$N_RESERVE (what scripts/reserve-keys.sh init makes on the founder Mac; throwaway here)"
  for i in $(seq 1 $N_RESERVE); do "$A" keygen --data "$dir/r$i" >/dev/null; done
  echo "  three throwaway reserve identities in $dir/r{1,2,3}"
  echo "== throwaway registrar key (a REAL P-256 key, so assemble runs exactly as in the ceremony)"
  "$A" registrar-key --data "$dir/registrar" > "$dir/registrar-key.out"
  registrar_hex=$(awk 'NR == 1 && $1 == "registrar" && $2 == "key" {print $3}' "$dir/registrar-key.out")
  if [ "${#registrar_hex}" != 128 ]; then die "could not read the throwaway registrar public key from $dir/registrar-key.out"; fi
  founder=$("$A" dev-accounts | awk '$1 == "dev" && $2 == 1 {print $3}')
  [ -n "$founder" ] || die "could not read a dev address for the dry run's --reserve-operator"
  echo "  throwaway registrar $(fp "$registrar_hex"), throwaway reserve operator $founder (dev 1)"
  echo "== throwaway builder keys (what the three builder Macs' builder-sign init print; throwaway here)"
  mkdir -p "$dir/coordinator"
  throwaway_release "$dir/coordinator/release.json"
  echo "  three throwaway builder keys in $dir/coordinator/release.json"
  echo "== the copy step: ONLY the public halves move to the coordinator"
  mkdir -p "$dir/coordinator/pub"
  for i in $(seq 1 $N_VALIDATORS); do cp "$dir/v$i/validator.pub.json" "$dir/coordinator/pub/v$i.pub.json"; done
  for i in $(seq 1 $N_RESERVE); do cp "$dir/r$i/validator.pub.json" "$dir/coordinator/pub/r$i.pub.json"; done
  echo "  copied the $N_VALIDATORS validator and $N_RESERVE reserve public entries into $dir/coordinator/pub/"
  echo
  echo "== [assemble] on the coordinator (REHEARSAL: chain id $REHEARSAL_CHAIN_ID, throwaway inputs)"
  local as_args
  as_args=(--chain-id "$REHEARSAL_CHAIN_ID" --registrar "$registrar_hex" --reserve-operator "$founder" --release "$dir/coordinator/release.json" --out "$dir/coordinator")
  for i in $(seq 1 $N_RESERVE); do as_args+=(--reserve "$dir/coordinator/pub/r$i.pub.json"); done
  for i in $(seq 1 $N_VALIDATORS); do as_args+=(--validator "$dir/coordinator/pub/v$i.pub.json"); done
  cmd_assemble "${as_args[@]}"
  echo
  echo "== genesis DKG over loopback (REHEARSAL of doc §3: really run, throwaway keys)"
  ports=()
  while [ "${#ports[@]}" -lt 4 ]; do
    p=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()')
    case " ${ports[*]:-} " in *" $p "*) ;; *) ports+=("$p");; esac
  done
  local pids=""
  for i in $(seq 1 $N_VALIDATORS); do
    peers=""
    for j in $(seq 1 $N_VALIDATORS); do if [ "$j" != "$i" ]; then peers+="${peers:+,}$j@127.0.0.1:${ports[$((j - 1))]}"; fi; done
    "$A" dkg --network "$dir/coordinator/genesis.json" --port "${ports[$((i - 1))]}" --data "$dir/v$i" --peers "$peers" --offline \
      > "$dir/dkg$i.log" 2>&1 &
    pids="$pids $!"
  done
  for pid in $pids; do
    if ! wait "$pid"; then echo "dkg failed (see $dir/dkg*.log)" >&2; exit 1; fi
  done
  echo "  all four DKGs done; validator 1 wrote $dir/v1/network.json (identity + output)"
  echo
  echo "== [check] on the DKG's network.json (REHEARSAL mode: --rehearsal passed here and ONLY here)"
  cmd_check "$dir/v1/network.json" --chain-id "$REHEARSAL_CHAIN_ID" --ceremony "$dir/coordinator/ceremony.json"
  echo
  echo "== [verify-local] on each validator Mac's data dir (what every validator runs before voting; REHEARSAL)"
  for i in $(seq 1 $N_VALIDATORS); do
    echo "-- validator $i"
    cmd_verify_local "$dir/v1/network.json" --data "$dir/v$i" --ceremony "$dir/v1/ceremony-check.json"
  done
  echo
  echo "== coordinator directory must hold no secret (the ceremony's copy rule)"
  bad=$(find "$dir/coordinator" -type f \( -name '*.key' -o -name 'threshold.json' \) | head -3 || true)
  if [ -z "$bad" ]; then
    echo "  PASS  no secret file in $dir/coordinator (only genesis.json and the copied public halves)"
  else
    echo "  FAIL  secret files in the coordinator directory:"$'\n'"$bad"
    exit 1
  fi
  echo "REHEARSAL, not a launch file" > "$dir/REHEARSAL-NOT-A-LAUNCH-FILE"
  echo
  echo "******************************************************************"
  echo "*   DRY RUN DONE — REHEARSAL, not a launch file.                 *"
  echo "*   Throwaway output kept in $dir (delete it).   *"
  echo "******************************************************************"
}

case "${1:-}" in
  keys) shift; cmd_keys "$@" ;;
  assemble) shift; cmd_assemble "$@" ;;
  check) shift; cmd_check "$@" ;;
  verify-local) shift; cmd_verify_local "$@" ;;
  --dry-run) shift; cmd_dry_run "$@" ;;
  *) usage ;;
esac
