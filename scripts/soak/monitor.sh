#!/usr/bin/env bash
# Soak monitor: every run, read height and hash from this Mac's validators and
# from the nodes on the other test Macs (over ssh), append one line to the log,
# and raise a macOS notification when the chain stalls, a node falls behind, or
# two nodes disagree on a block hash. Run it every minute (launchd, see README).
#   scripts/soak/monitor.sh
# Hosts: AETHER_SOAK_HOSTS="user@host other" (optional ssh targets running a node on :18545;
# unreachable hosts are skipped: validator liveness is read from the chain itself)
set -uo pipefail
LOG_DIR=${AETHER_SOAK_LOG:-$HOME/aether-soak}
mkdir -p "$LOG_DIR"
LOG="$LOG_DIR/monitor.csv"
STATE="$LOG_DIR/monitor.state"
HOSTS=${AETHER_SOAK_HOSTS:-}
LAG_LIMIT=${AETHER_SOAK_LAG:-30}
q='{"jsonrpc":"2.0","id":1,"method":"aether_status","params":[]}'
parse() { python3 -c 'import json,sys
try:
  r=json.load(sys.stdin)["result"]; print(r["height"], r["hash"], r["mempool"])
except Exception: print("0 - 0")'; }
local_status() { curl -s -m4 "localhost:$1" -H 'content-type: application/json' -d "$q" | parse; }
remote_status() { ssh -o BatchMode=yes -o ConnectTimeout=6 "$1" "curl -s -m4 localhost:18545 -H 'content-type: application/json' -d '$q'" 2>/dev/null | parse; }
hash_at() { curl -s -m4 "localhost:8601" -H 'content-type: application/json' -d "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"aether_getBlock\",\"params\":[$1]}" | python3 -c 'import json,sys
try: print(json.load(sys.stdin)["result"]["hash"])
except Exception: print("-")'; }
notify() { osascript -e "display notification \"$1\" with title \"Aether soak\"" >/dev/null 2>&1; echo "$(date -u +%FT%TZ) ALERT $1" >> "$LOG_DIR/alerts.log"; }

now=$(date -u +%FT%TZ)
read -r vh vhash vpool < <(local_status 8601)
line="$now,validator,$vh,$vpool"
max=$vh
for h in $HOSTS; do
  read -r rh rhash _ < <(remote_status "$h")
  line="$line,$h,$rh"
  # Unreachable over ssh (e.g. Tailscale off) is not an alarm: chain-side checks below cover liveness.
  if [ "$rh" = 0 ]; then continue; fi
  [ "$rh" -gt "$max" ] && max=$rh
  if [ "$vh" -gt 0 ] && [ $((vh - rh)) -gt "$LAG_LIMIT" ]; then notify "$h is $((vh - rh)) blocks behind"; fi
  # Same height, different hash would be a fork: compare at the node's height.
  if [ "$rh" -gt 0 ] && [ "$rh" -le "$vh" ]; then
    vhh=$(hash_at "$rh")
    if [ "$vhh" != "-" ] && [ "$rhash" != "-" ] && [ "$vhh" != "$rhash" ] && [ "0x$rhash" != "$vhh" ] && [ "$rhash" != "${vhh#0x}" ]; then notify "$h disagrees at block $rh"; fi
  fi
done
# Chain-side liveness (no ssh, no Tailscale): every validator should propose within the last 200 blocks.
missing=$(curl -s -m6 localhost:8601 -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"aether_recentBlocks","params":[100]}' | python3 -c '
import json, sys
try:
    blocks = json.load(sys.stdin)["result"]
    seen = sorted({b["proposer"] for b in blocks})
    print(len(seen))
except Exception:
    print(-1)')
vals=$(curl -s -m4 localhost:8601 -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"aether_network","params":[]}' | python3 -c 'import json,sys
try: print(len(json.load(sys.stdin)["result"]["validators"]))
except Exception: print(-1)')
line="$line,proposers_last100,$missing,validators,$vals"
if [ "$missing" -ge 0 ] && [ "$vals" -gt 0 ] && [ "$missing" -lt "$vals" ]; then notify "only $missing of $vals validators proposed in the last 100 blocks"; fi
echo "$line" >> "$LOG"
prev=$(cat "$STATE" 2>/dev/null || echo 0)
if [ "$vh" = 0 ]; then notify "validators not answering"
elif [ "$vh" -le "$prev" ]; then notify "chain stalled at $vh"; fi
echo "$vh" > "$STATE"
