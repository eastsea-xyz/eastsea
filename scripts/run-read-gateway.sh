#!/usr/bin/env bash
# The public read-only gateway runner (docs/research/public-read-access-2026-10-05.md §8(a),
# full runbook: docs/ops/read-gateway.md).
#
# What it runs, once --apply is given (never by default):
#   1. `aether follow --public-read-only` — a follower that serves ONLY the
#      explorer's read methods, with caps, bound to 127.0.0.1:<port>;
#   2. `cloudflared tunnel` — the operator's chosen hostname -> 127.0.0.1:<port>.
#      Creates the named tunnel on first use; without DNS permissions it prints
#      the CNAME the operator must add by hand.
#
# The runner stays in the foreground of --apply and SUPERVISES the follower
# (audit 7 A7-2): `aether follow` exits on purpose (11: stalled transport,
# 12: disk floor) expecting a restart owner, and this loop is it — the same
# policy `aether run`'s supervisor applies (crates/node/src/supervisor.rs).
#
# SAFETY: the default is a dry-run. It starts nothing, contacts no network, and
# changes no files — it prints the plan (commands, config.yml, DNS records).
# This script never writes to a validator's data directory; give it its own
# (--data, default under /Volumes/workspace).
#
# Usage:
#   scripts/run-read-gateway.sh                       # dry-run: print the plan
#   scripts/run-read-gateway.sh --apply --hostname rpc.example.net
#   scripts/run-read-gateway.sh --data DIR --port 18550 --hostname rpc.example.net \
#       --tunnel eastsea-read --network network.json --from-rpc https://node1:18545
set -euo pipefail

DATA_DIR="/Volumes/workspace/eastsea-read-gateway"
PORT=18550
HOSTNAME=""
TUNNEL="eastsea-read"
NETWORK=""    # network.json of the chain to follow (empty = the public devnet keys)
FROM_RPC=()   # validator RPC(s) to follow through; empty = iroh discovery
APPLY=0

while [ $# -gt 0 ]; do
  case "$1" in
    --apply) APPLY=1 ;;
    --data) DATA_DIR=$2; shift ;;
    --port) PORT=$2; shift ;;
    --hostname) HOSTNAME=$2; shift ;;
    --tunnel) TUNNEL=$2; shift ;;
    --network) NETWORK=$2; shift ;;
    --from-rpc) FROM_RPC+=("$2"); shift ;;
    -h|--help) sed -n '2,22p' "$0"; exit 0 ;;
    *) echo "unknown option: $1 (try --help)" >&2; exit 2 ;;
  esac
  shift
done

if [ "$APPLY" = 1 ] && [ -z "$HOSTNAME" ]; then
  echo "--hostname is required for --apply (the explorer has no default gateway)" >&2
  exit 2
fi

say() { printf '%s\n' "$*"; }
section() { printf '\n== %s ==\n' "$*"; }

need() { command -v "$1" >/dev/null 2>&1 || { echo "missing: $1 — $2" >&2; MISSING+=("$1"); }; }
MISSING=()

AETHER=${AETHER:-aether}
CLOUDFLARED=${CLOUDFLARED:-cloudflared}

section "plan"
say "data dir    : $DATA_DIR (its own directory, never a validator's)"
say "rpc         : http://127.0.0.1:$PORT  (--public-read-only: allowlist + caps, loopback bind)"
say "tunnel      : $CLOUDFLARED named tunnel '$TUNNEL' -> $HOSTNAME -> 127.0.0.1:$PORT"
[ -n "$NETWORK" ] && say "network     : $NETWORK" || say "network     : (default public devnet keys — pass --network for the real chain)"
[ ${#FROM_RPC[@]} -gt 0 ] && say "follow from : ${FROM_RPC[*]}" || say "follow from : (iroh discovery — pass --from-rpc URL to pull over HTTP)"

FOLLOW_ARGS=(--data "$DATA_DIR" --public-read-only --rpc-port "$PORT")
[ -n "$NETWORK" ] && FOLLOW_ARGS+=(--network "$NETWORK")
for u in "${FROM_RPC[@]:-}"; do [ -n "$u" ] && FOLLOW_ARGS+=(--from-rpc "$u"); done

section "the follower (loopback only)"
say "  $AETHER follow ${FOLLOW_ARGS[*]}"
say "  Refuses to bind anything but loopback; writes and node-local methods"
say "  answer -32601 with 'public read-only gateway: …' (docs/ops/read-gateway.md)."

section "the tunnel (the only exposure)"
CF_DIR="$HOME/.cloudflared"
CONFIG="$CF_DIR/config-$TUNNEL.yml"
say "  named tunnel : $TUNNEL (created on first --apply if missing)"
say "  config file  : $CONFIG"
say "--- config.yml it would write ---"
cat <<EOF
tunnel: (\$TUNNEL_ID)
credentials-file: $CF_DIR/<tunnel-id>.json
ingress:
  - hostname: $HOSTNAME
    service: http://127.0.0.1:$PORT
  - service: http_status:404
EOF
say "--- end config.yml ---"
say "  dns record   : $HOSTNAME CNAME <tunnel-id>.cfargotunnel.com"
say "                 (cloudflared tries 'tunnel route dns' itself; without zone"
say "                  permissions it prints this CNAME for the operator to add)"

section "checks"
need "$AETHER" "the node binary (cargo build -p aether-node, or scripts/build.sh)"
need "$CLOUDFLARED" "brew install cloudflared (the only exposure path)"
if [ ${#MISSING[@]} -gt 0 ]; then
  say "  cannot --apply yet: install the missing ${MISSING[*]}"
else
  say "  all present"
fi

if [ "$APPLY" -ne 1 ]; then
  section "dry-run"
  say "Nothing was started and nothing was written. Re-run with --apply to:"
  say "  1. create/find tunnel '$TUNNEL' and write $CONFIG"
  say "  2. try 'tunnel route dns' (or print the CNAME to add by hand)"
  say "  3. start cloudflared and supervise the follower in the foreground —"
  say "     it restarts after a stall/crash exit with the same read-only args (logs under $DATA_DIR/logs)"
  say "Read docs/ops/read-gateway.md first — it lists what the gateway refuses,"
  say "the caps, the Cloudflare rate-limit rule to add, and how to verify it all."
  exit 0
fi

# ---- --apply: everything below touches the world; the runbook owns it ----

if [ ${#MISSING[@]} -gt 0 ]; then
  echo "refusing: missing ${MISSING[*]}" >&2
  exit 1
fi
mkdir -p "$DATA_DIR/logs" "$CF_DIR"

TUNNEL_ID="$("$CLOUDFLARED" tunnel list --output json \
  | /usr/bin/python3 -c 'import json,sys
for t in json.load(sys.stdin):
    if t["name"] == sys.argv[1]:
        print(t["id"]); break' "$TUNNEL")"
if [ -z "${TUNNEL_ID:-}" ]; then
  say "creating tunnel '$TUNNEL'…"
  CREATE_OUT="$("$CLOUDFLARED" tunnel create "$TUNNEL")"
  TUNNEL_ID="$("$CLOUDFLARED" tunnel list --output json \
    | /usr/bin/python3 -c 'import json,sys
for t in json.load(sys.stdin):
    if t["name"] == sys.argv[1]:
        print(t["id"]); break' "$TUNNEL")"
  [ -n "$TUNNEL_ID" ] || { echo "could not create/find tunnel: $CREATE_OUT" >&2; exit 1; }
fi
CRED="$CF_DIR/$TUNNEL_ID.json"
[ -f "$CRED" ] || { echo "tunnel credentials not found: $CRED" >&2; exit 1; }

cat > "$CONFIG" <<EOF
tunnel: $TUNNEL_ID
credentials-file: $CRED
ingress:
  - hostname: $HOSTNAME
    service: http://127.0.0.1:$PORT
  - service: http_status:404
EOF

if ! "$CLOUDFLARED" tunnel route dns "$TUNNEL" "$HOSTNAME" 2>&1; then
  say ""
  say "could not set DNS (no zone permission?). Add this record by hand:"
  say "  $HOSTNAME  CNAME  $TUNNEL_ID.cfargotunnel.com  (proxied)"
fi

# ---- the follower: this runner is its restart owner (audit 7 A7-2) ----
# `aether follow` exits on purpose expecting a supervisor: 11 restarts a
# transport that made no verified progress for ten minutes, 12 waits for disk
# space. `aether run` has that supervisor in-process; a bare `nohup` follow —
# what this runner used to do — leaves the first stall exit dead with the
# tunnel still up against a dead origin. So --apply stays in the foreground
# and applies the same policy as the node supervisor: bounded backoff that
# starts over after five minutes of life, a stop for codes no restart fixes
# (3 upgrade, 4 storage, 5 verifier, 6 identity, 7 locked), a stop after four
# stall exits in an hour, and a clean stop on exit 0. FOLLOW_ARGS above — the
# read-only allowlist and the loopback bind — is reused verbatim on every
# restart: the gateway never comes back wider than it went down.

nohup "$CLOUDFLARED" tunnel --config "$CONFIG" run "$TUNNEL" \
  >> "$DATA_DIR/logs/cloudflared.log" 2>&1 &
CF_PID=$!
echo "$CF_PID" > "$DATA_DIR/logs/cloudflared.pid"
cleanup() { if kill -0 "$CF_PID" 2>/dev/null; then kill "$CF_PID" 2>/dev/null || true; fi; return 0; }
trap cleanup EXIT

# The tunnel is the only exposure; if it dies the runner brings it back
# before the next follower start.
ensure_tunnel() {
  if ! kill -0 "$CF_PID" 2>/dev/null; then
    say "[gateway] cloudflared is down; restarting the tunnel"
    nohup "$CLOUDFLARED" tunnel --config "$CONFIG" run "$TUNNEL" \
      >> "$DATA_DIR/logs/cloudflared.log" 2>&1 &
    CF_PID=$!
    echo "$CF_PID" > "$DATA_DIR/logs/cloudflared.pid"
  fi
  return 0
}

# Keep the follower log bounded (~10 MB), as the archive runner does.
trim_follower_log() {
  local f="$DATA_DIR/logs/follower.log" size
  if [ -f "$f" ]; then
    size=$(wc -c < "$f" 2>/dev/null || echo 0)
    if [ "${size:-0}" -gt 10485760 ]; then
      if tail -c 2097152 "$f" > "$f.part"; then mv "$f.part" "$f"; fi
    fi
  fi
  return 0
}

section "supervising the follower"
say "follower log : $DATA_DIR/logs/follower.log"
say "cloudflared  : pid $CF_PID (log: $DATA_DIR/logs/cloudflared.log)"
say "this console : restarts and stops (Ctrl-C stops the gateway)"
n=0
backoff=1
stalls=""   # epoch seconds of recent stall (11) exits, for the hour window
while :; do
  n=$((n + 1))
  ensure_tunnel
  trim_follower_log
  started=$(date +%s)
  code=0
  "$AETHER" follow "${FOLLOW_ARGS[@]}" >> "$DATA_DIR/logs/follower.log" 2>&1 || code=$?
  lived=$(( $(date +%s) - started ))
  case "$code" in
    0)
      say "[gateway] follower exited 0 after ${lived}s — deliberate stop; gateway down"
      exit 0
      ;;
    3|4|5|6|7)
      say "[gateway] follower exited $code after ${lived}s — no restart fixes this (docs/ops/read-gateway.md); stopping"
      exit "$code"
      ;;
    12)
      say "[gateway] follower exited 12 (disk floor) after ${lived}s; waiting 30s for space"
      sleep 30
      ;;
    11)
      now=$(date +%s)
      keep=""
      for t in $stalls; do
        if [ $((now - t)) -lt 3600 ]; then keep="$keep $t"; fi
      done
      stalls="$keep $now"
      set -- $stalls
      if [ "$#" -ge 4 ]; then
        say "[gateway] four stall exits within an hour — stopping (the supervisor's own policy); see $DATA_DIR/logs/follower.log"
        exit 11
      fi
      say "[gateway] follower stalled (exit 11) after ${lived}s; restart #$n in ${backoff}s"
      sleep "$backoff"
      backoff=$((backoff * 2))
      if [ "$backoff" -gt 60 ]; then backoff=60; fi
      ;;
    *)
      say "[gateway] follower exited $code after ${lived}s; restart #$n in ${backoff}s"
      sleep "$backoff"
      backoff=$((backoff * 2))
      if [ "$backoff" -gt 60 ]; then backoff=60; fi
      ;;
  esac
  # Five minutes of life between exits is progress, not a crash loop (the
  # supervisor's PROGRESS_MS): the backoff starts over.
  if [ "$lived" -ge 300 ]; then backoff=1; fi
done
