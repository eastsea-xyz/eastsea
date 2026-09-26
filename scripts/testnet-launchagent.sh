#!/usr/bin/env bash
# Keep this Mac's testnet validators running across logins and crashes (launchd).
#   scripts/testnet-launchagent.sh install | uninstall
# One LaunchAgent per validator (KeepAlive): launchd restarts a validator that
# exits and starts all of them at login. Chain data is never wiped.
set -euo pipefail
T=${AETHER_TESTNET:-$HOME/aether-testnet}
A="$T/bin/aether"
LA="$HOME/Library/LaunchAgents"
N=$(ls -d "$T"/[0-9]* 2>/dev/null | wc -l | tr -d ' ')
label() { echo "com.pipln.aether.testnet.v$1"; }
case "${1:-}" in
  install)
    mkdir -p "$LA"
    "$(dirname "$0")/testnet.sh" stop >/dev/null 2>&1 || true
    for i in $(seq 1 "$N"); do
      faucet=""
      [ -f "$T/$i/faucet.key" ] && faucet="<string>--faucet-key</string><string>$T/$i/faucet.key</string>"
      cat > "$LA/$(label "$i").plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$(label "$i")</string>
  <key>ProgramArguments</key>
  <array>
    <string>$A</string><string>node</string>
    <string>--network</string><string>$T/$i/network.json</string>
    <string>--port</string><string>$((9100 + i))</string>
    <string>--rpc-port</string><string>$((8600 + i))</string>
    <string>--data</string><string>$T/$i</string>
    $faucet
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>StandardOutPath</key><string>$T/node$i.log</string>
  <key>StandardErrorPath</key><string>$T/node$i.log</string>
</dict>
</plist>
PLIST
      launchctl bootout "gui/$(id -u)/$(label "$i")" 2>/dev/null || true
      launchctl bootstrap "gui/$(id -u)" "$LA/$(label "$i").plist"
      echo "installed $(label "$i")"
    done ;;
  uninstall)
    for i in $(seq 1 "$N"); do
      launchctl bootout "gui/$(id -u)/$(label "$i")" 2>/dev/null || true
      rm -f "$LA/$(label "$i").plist"
      echo "removed $(label "$i")"
    done ;;
  *) echo "usage: $0 install | uninstall"; exit 1 ;;
esac
