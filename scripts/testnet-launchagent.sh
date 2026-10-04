#!/usr/bin/env bash
# Keep this Mac's testnet validators running across logins and crashes (launchd).
#   scripts/testnet-launchagent.sh install | uninstall
# One LaunchAgent per validator (KeepAlive): launchd restarts a validator that
# exits and starts all of them at login. Chain data is never wiped. Each runs
# `aether run`: a validator while the network keeps it in the voting set, a
# verifying follower once registered Macs take the seats.
# The binary is signed with the Developer ID (SIGN_IDENTITY) and the agents name
# the EastSea app, so System Settings ▸ Login Items lists them as EastSea (Pipln),
# not as an unidentified command-line tool.
# `caffeinate -s` wraps each validator: the Mac does not sleep while on power
# (a sleeping validator is a missing vote). `--exit-with-parent` stops the node
# if caffeinate is killed, so nothing is left running outside launchd.
set -euo pipefail
T=${AETHER_TESTNET:-$HOME/aether-testnet}
A="$T/bin/aether"
LA="$HOME/Library/LaunchAgents"
N=$(ls -d "$T"/[0-9]* 2>/dev/null | wc -l | tr -d ' ')
label() { echo "com.pipln.aether.testnet.v$1"; }
case "${1:-}" in
  install)
    mkdir -p "$LA"
    codesign --force --options runtime --timestamp --sign "${SIGN_IDENTITY:-Developer ID Application: Pipln (45WU468FZE)}" "$A"
    "$(dirname "$0")/testnet.sh" stop >/dev/null 2>&1 || true
    for i in $(seq 1 "$N"); do
      extra=""
      [ -f "$T/$i/faucet.key" ] && extra+="<string>--node-arg=--faucet-key=$T/$i/faucet.key</string>"
      # The registrar (validator 1) checks each Mac with Apple DeviceCheck.
      dc=$(ls "$HOME"/.config/aether/devicecheck/AuthKey_*.p8 2>/dev/null | head -1 || true)
      if [ -f "$T/$i/registrar.key" ] && [ -n "$dc" ]; then
        kid=$(basename "$dc" .p8); kid=${kid#AuthKey_}
        extra+="<string>--node-arg=--devicecheck-key=$dc</string><string>--node-arg=--devicecheck-key-id=$kid</string>"
      fi
      cat > "$LA/$(label "$i").plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$(label "$i")</string>
  <key>AssociatedBundleIdentifiers</key><array><string>com.pipln.eastsea</string></array>
  <key>ProgramArguments</key>
  <array>
    <string>/usr/bin/caffeinate</string><string>-s</string>
    <string>$A</string><string>run</string>
    <string>--network</string><string>$T/$i/network.json</string>
    <string>--port</string><string>$((9100 + i))</string>
    <string>--rpc-port</string><string>$((8600 + i))</string>
    <string>--data</string><string>$T/$i</string>
    <string>--exit-with-parent</string>
    $extra
  </array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <!-- The vote journal keeps one file per section: launchd's default of 256 open files is too few. -->
  <key>SoftResourceLimits</key><dict><key>NumberOfFiles</key><integer>65536</integer></dict>
  <key>HardResourceLimits</key><dict><key>NumberOfFiles</key><integer>65536</integer></dict>
  <key>ThrottleInterval</key><integer>10</integer>
  <key>StandardOutPath</key><string>$T/node$i.log</string>
  <key>StandardErrorPath</key><string>$T/node$i.log</string>
</dict>
</plist>
PLIST
      launchctl bootout "gui/$(id -u)/$(label "$i")" 2>/dev/null || true
      # bootout finishes asynchronously; retry until the old job is gone.
      for _ in 1 2 3 4 5 6 7 8 9 10; do
        launchctl bootstrap "gui/$(id -u)" "$LA/$(label "$i").plist" 2>/dev/null && break
        sleep 1
      done
      launchctl print "gui/$(id -u)/$(label "$i")" >/dev/null
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
