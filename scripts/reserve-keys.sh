#!/usr/bin/env bash
# Founder reserve keys on the founder's one Mac (docs/ops/reserve-keys.md,
# docs/design/15-node-rewards.md "창업자 예비 키").
#   scripts/reserve-keys.sh init                  3 key folders + the lines for `aether network`
#   scripts/reserve-keys.sh install <network.json> [--print]
#                                                 one LaunchAgent per key running `aether run`
#                                                 (--print: show the plists, install nothing)
#   scripts/reserve-keys.sh status                agents, heights, voting or following
#   scripts/reserve-keys.sh uninstall             remove the agents (keys and chain data stay)
# Each key runs `aether run`: a verifying follower while the chain leaves it out
# of the voting set, a validator once the rules seat it (fewer than 4
# independent operators); it reshares and hands over by itself either way.
# `caffeinate -s` wraps each one: the Mac does not sleep while on power.
# Env: AETHER_RESERVE (default ~/aether-reserve), AETHER_BIN (default: aether on PATH),
#      RESERVE_P2P_BASE (19200: key i uses base+2i and base+2i+1 for reshares),
#      RESERVE_RPC_BASE (18700: key i uses base+i), SIGN_IDENTITY.
set -euo pipefail
R=${AETHER_RESERVE:-$HOME/aether-reserve}
KEYS=3
P2P_BASE=${RESERVE_P2P_BASE:-19200}
RPC_BASE=${RESERVE_RPC_BASE:-18700}
LA="$HOME/Library/LaunchAgents"
label() { echo "com.pipln.eastsea.reserve.$1"; }
p2p() { echo $((P2P_BASE + 2 * $1)); }
rpc() { echo $((RPC_BASE + $1)); }
src_bin() { if [ -n "${AETHER_BIN:-}" ]; then echo "$AETHER_BIN"; else command -v aether || true; fi; }
key_of() { python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["key"])' "$R/$1/validator.pub.json"; }

plist() {  # plist <i> <aether binary>
  local i=$1 a=$2
  cat <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$(label "$i")</string>
  <key>AssociatedBundleIdentifiers</key><array><string>com.pipln.eastsea</string></array>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/bin/caffeinate</string><string>-s</string>
    <string>$a</string><string>run</string>
    <string>--network</string><string>$R/network.json</string>
    <string>--data</string><string>$R/$i</string>
    <string>--port</string><string>$(p2p "$i")</string>
    <string>--rpc-port</string><string>$(rpc "$i")</string>
    <string>--exit-with-parent</string>
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>StandardOutPath</key><string>$R/reserve$i.log</string>
  <key>StandardErrorPath</key><string>$R/reserve$i.log</string>
</dict>
</plist>
PLIST
}

case "${1:-}" in
  init)
    a=$(src_bin); [ -x "$a" ] || { echo "no aether binary: set AETHER_BIN or put aether on PATH" >&2; exit 1; }
    mkdir -p "$R"
    for i in $(seq 1 $KEYS); do
      if [ -f "$R/$i/validator.key" ]; then
        echo "key $i: exists ($R/$i), kept"
      else
        "$a" keygen --data "$R/$i" >/dev/null
        echo "key $i: created in $R/$i"
      fi
      chmod 700 "$R/$i"
    done
    echo
    echo "Public entries (safe to share):"
    for i in $(seq 1 $KEYS); do cat "$R/$i/validator.pub.json"; echo; done
    echo "Add to the genesis network file:"
    printf '  aether network … --node-rewards --registrar <hex> --reserve-operator <founder address>'
    for i in $(seq 1 $KEYS); do printf ' \\\n    --reserve %s' "$R/$i/validator.pub.json"; done
    echo " \\"
    echo "    <genesis validators' validator.pub.json …>"
    echo "Back up $R/*/validator.key offline. Then, with the final network.json: $0 install <network.json>" ;;
  install)
    net=${2:-}; [ -f "$net" ] || { echo "usage: $0 install <network.json> [--print]" >&2; exit 1; }
    print=${3:-}
    for i in $(seq 1 $KEYS); do
      [ -f "$R/$i/validator.key" ] || { echo "no key $i: run $0 init first" >&2; exit 1; }
    done
    python3 - "$net" "$R" "$KEYS" <<'PY'
import json, sys
net, root, n = json.load(open(sys.argv[1])), sys.argv[2], int(sys.argv[3])
listed = {m["key"] for m in (net.get("reserve") or {}).get("validators", [])}
missing = [i for i in range(1, n + 1) if json.load(open(f"{root}/{i}/validator.pub.json"))["key"] not in listed]
if not net.get("identity"):
    sys.exit("network.json has no committee identity: use the one the genesis DKG wrote")
if missing:
    sys.exit(f"key(s) {missing} are not reserve keys in this network.json (\"reserve\")")
PY
    a="$R/bin/aether"
    if [ "$print" = --print ]; then
      for i in $(seq 1 $KEYS); do echo "== $LA/$(label "$i").plist"; plist "$i" "$a"; done
      exit 0
    fi
    s=$(src_bin); [ -x "$s" ] || { echo "no aether binary: set AETHER_BIN or put aether on PATH" >&2; exit 1; }
    # launchd cannot run binaries from an external volume reliably: keep a copy under $R.
    mkdir -p "$R/bin" "$LA"
    cp -f "$s" "$a"
    codesign --force --options runtime --timestamp --sign "${SIGN_IDENTITY:-Developer ID Application: Pipln (45WU468FZE)}" "$a"
    cp -f "$net" "$R/network.json"
    for i in $(seq 1 $KEYS); do
      plist "$i" "$a" > "$LA/$(label "$i").plist"
      launchctl bootout "gui/$(id -u)/$(label "$i")" 2>/dev/null || true
      for _ in 1 2 3 4 5 6 7 8 9 10; do
        launchctl bootstrap "gui/$(id -u)" "$LA/$(label "$i").plist" 2>/dev/null && break
        sleep 1
      done
      launchctl print "gui/$(id -u)/$(label "$i")" >/dev/null
      echo "installed $(label "$i")  p2p $(p2p "$i")  rpc $(rpc "$i")"
    done ;;
  status)
    for i in $(seq 1 $KEYS); do
      [ -f "$R/$i/validator.pub.json" ] || { echo "key $i: none (run $0 init)"; continue; }
      k=$(key_of "$i")
      loaded=no; launchctl print "gui/$(id -u)/$(label "$i")" >/dev/null 2>&1 && loaded=yes
      st=$(curl -s -m4 "localhost:$(rpc "$i")" -H 'content-type: application/json' \
        -d '{"jsonrpc":"2.0","id":1,"method":"aether_status","params":[]}' | python3 -c 'import json,sys
try: print(json.load(sys.stdin)["result"]["height"])
except Exception: print("-")' || true)
      role=$(curl -s -m4 "localhost:$(rpc "$i")" -H 'content-type: application/json' \
        -d '{"jsonrpc":"2.0","id":1,"method":"aether_network","params":[]}' | python3 -c 'import json,sys
try: print("voting" if any(m["key"] == sys.argv[1] for m in json.load(sys.stdin)["result"]["validators"]) else "following")
except Exception: print("-")' "$k" || true)
      echo "key $i ${k:0:16}…  agent $loaded  height $st  $role"
    done ;;
  uninstall)
    for i in $(seq 1 $KEYS); do
      launchctl bootout "gui/$(id -u)/$(label "$i")" 2>/dev/null || true
      rm -f "$LA/$(label "$i").plist"
      echo "removed $(label "$i") (keys and data kept in $R/$i)"
    done ;;
  *) sed -n '2,16p' "$0"; exit 1 ;;
esac
