#!/usr/bin/env bash
# Soak monitor: every run, read height and hash from this Mac's validators and
# from the nodes on the other test Macs (over ssh), append one line to the log,
# and raise a macOS notification when the chain stalls, a node falls behind, or
# two nodes disagree on a block hash. Run it every minute (launchd, see README).
#   scripts/soak/monitor.sh
# Each problem alerts once when it starts and once when it clears (not every
# minute): the open problems are files in $LOG_DIR/open.
# Self-contained: launchd runs a copy from ~/aether-soak/bin (its bash cannot
# read /Volumes), so it never reads anything from the repo.
# Hosts: AETHER_SOAK_HOSTS="user@host other" (optional ssh targets running a node on :18545;
# unreachable hosts are skipped: validator liveness is read from the chain itself)
# AETHER_SOAK_FINALITY: seconds without a finalized block before "finalization is slow" (default 30).
set -uo pipefail
LOG_DIR=${AETHER_SOAK_LOG:-$HOME/aether-soak}
OPEN="$LOG_DIR/open"
mkdir -p "$LOG_DIR" "$OPEN"
LOG="$LOG_DIR/monitor.csv"
STATE="$LOG_DIR/monitor.state"
HOSTS=${AETHER_SOAK_HOSTS:-}
LAG_LIMIT=${AETHER_SOAK_LAG:-30}
FINALITY_LIMIT=${AETHER_SOAK_FINALITY:-30}
RPC=${AETHER_SOAK_RPC:-localhost:8601}
q='{"jsonrpc":"2.0","id":1,"method":"aether_status","params":[]}'
parse() { python3 -c 'import json,sys
try:
  r=json.load(sys.stdin)["result"]; print(r["height"], r["hash"], r["mempool"], r.get("timestamp_ms", 0))
except Exception: print("0 - 0 0")'; }
local_status() { curl -s -m4 "$1" -H 'content-type: application/json' -d "$q" | parse; }
remote_status() { ssh -o BatchMode=yes -o ConnectTimeout=6 "$1" "curl -s -m4 localhost:18545 -H 'content-type: application/json' -d '$q'" 2>/dev/null | parse; }
hash_at() { curl -s -m4 "$RPC" -H 'content-type: application/json' -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"aether_getBlock\",\"params\":[$1]}" | python3 -c 'import json,sys
try: print(json.load(sys.stdin)["result"]["hash"])
except Exception: print("-")'; }
say() {
  osascript -e "display notification \"$2\" with title \"Aether soak\" subtitle \"$1\"" >/dev/null 2>&1
  echo "$(date -u +%FT%TZ) $1 $2" >> "$LOG_DIR/alerts.log"
}
# problem <key> <message>: alert only if <key> was not already open.
problem() {
  local f="$OPEN/$1"
  [ -e "$f" ] && return 0
  echo "$(date +%s) $2" > "$f"
  say ALERT "$2"
}
# ok <key> <message>: if <key> was open, close it and say it recovered.
ok() {
  local f="$OPEN/$1"
  [ -e "$f" ] || return 0
  local since
  since=$(cut -d' ' -f1 "$f")
  rm -f "$f"
  say RECOVERED "$2 (after $(( ($(date +%s) - since) / 60 )) min)"
}
key() { printf '%s' "$1" | tr -c 'A-Za-z0-9._-' '_'; }

now=$(date -u +%FT%TZ)
now_ms=$(python3 -c 'import time; print(int(time.time()*1000))')
read -r vh vhash vpool vts < <(local_status "$RPC")
line="$now,validator,$vh,$vpool"
for h in $HOSTS; do
  read -r rh rhash _ _ < <(remote_status "$h")
  line="$line,$h,$rh"
  # Unreachable over ssh (e.g. Tailscale off) is not an alarm: chain-side checks below cover liveness.
  if [ "$rh" = 0 ]; then continue; fi
  k=$(key "$h")
  if [ "$vh" -gt 0 ] && [ $((vh - rh)) -gt "$LAG_LIMIT" ]; then problem "lag-$k" "$h is $((vh - rh)) blocks behind"
  else ok "lag-$k" "$h caught up"; fi
  # Same height, different hash would be a fork: compare at the node's height.
  if [ "$rh" -gt 0 ] && [ "$rh" -le "$vh" ]; then
    vhh=$(hash_at "$rh")
    if [ "$vhh" != "-" ] && [ "$rhash" != "-" ] && [ "$vhh" != "$rhash" ] && [ "0x$rhash" != "$vhh" ] && [ "$rhash" != "${vhh#0x}" ]; then problem "fork-$k" "$h disagrees at block $rh"
    else ok "fork-$k" "$h agrees again"; fi
  fi
done
# Chain-side liveness (no ssh, no Tailscale): every validator should propose within the last 100 blocks.
missing=$(curl -s -m6 "$RPC" -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"aether_recentBlocks","params":[100]}' | python3 -c '
import json, sys
try:
    blocks = json.load(sys.stdin)["result"]
    print(len({b["proposer"] for b in blocks}))
except Exception:
    print(-1)')
vals=$(curl -s -m4 "$RPC" -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"aether_network","params":[]}' | python3 -c 'import json,sys
try: print(len(json.load(sys.stdin)["result"]["validators"]))
except Exception: print(-1)')
line="$line,proposers_last100,$missing,validators,$vals"
if [ "$missing" -ge 0 ] && [ "$vals" -gt 0 ]; then
  if [ "$missing" -lt "$vals" ]; then problem proposers "only $missing of $vals validators proposed in the last 100 blocks"
  else ok proposers "all $vals validators propose again"; fi
fi
# Finalization delay: the newest finalized block's age (its timestamp), or no
# new height since the last run when the node gives no timestamp.
prev=$(cat "$STATE" 2>/dev/null || echo 0)
age=-1
[ "${vts:-0}" -gt 0 ] && age=$(( (now_ms - vts) / 1000 ))
line="$line,finality_age_s,$age"
echo "$line" >> "$LOG"
if [ "$vh" = 0 ]; then
  problem down "validators not answering"
else
  ok down "validators answer again at $vh"
  if [ "$age" -gt "$FINALITY_LIMIT" ] || { [ "$age" -lt 0 ] && [ "$vh" -le "$prev" ]; }; then
    [ "$age" -ge 0 ] && for_s="${age}s" || for_s="a minute"
    problem finality "no block finalized for $for_s (chain paused at $vh)"
  else
    ok finality "blocks finalize again (height $vh)"
  fi
fi
echo "$vh" > "$STATE"
