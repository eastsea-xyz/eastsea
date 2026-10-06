#!/bin/bash
# EastSea unattended-restart root stub (docs/design/29-unattended-restart.md).
# launchd runs this as root at boot, before any login (SMAppService.daemon,
# approved once by the user in System Settings > Login Items). It waits for
# the app's opt-in marker `unattended.plist` in a user's node data directory
# and then runs the node as that user: root runs nothing but this wait and
# the privilege drop, so node data and logs never end up root-owned.
#
# A marker that is not there yet is indistinguishable from FileVault still
# holding the volume locked (a cold boot waits at the unlock screen), so this
# never gives up on a missing marker — it polls quietly instead of exiting,
# which under KeepAlive/SuccessfulExit=false would end the daemon for the
# whole boot. macOS's bash 3.2: no mapfile.

set -u

POLL=30  # seconds; matches the plist's ThrottleInterval

child=0
term() {
  if [ "$child" -gt 0 ]; then kill -TERM "$child" 2>/dev/null; fi
  exit 0
}
trap term TERM INT

while true; do
  for marker in /Users/*/Library/Application\ Support/EastSea/node/unattended.plist; do
    [ -f "$marker" ] || continue
    user=$(/usr/libexec/PlistBuddy -c 'Print :user' "$marker" 2>/dev/null) || continue
    bundle=$(/usr/libexec/PlistBuddy -c 'Print :bundle' "$marker" 2>/dev/null) || continue
    # The marker is user-writable, so this stub trusts it for exactly one
    # thing: running that bundle's own wrapper as the user whose home the
    # marker lives in. Any other claim in the marker is the wrapper's to
    # re-check, at the user's own privilege.
    case "$marker" in
      "/Users/$user/"*) ;;
      *) continue ;;
    esac
    wrapper="$bundle/Contents/Resources/eastsea-node-wrapper.sh"
    [ -x "$wrapper" ] || continue
    /usr/bin/sudo -u "$user" /bin/bash "$wrapper" "$marker" &
    child=$!
    wait "$child"
    child=0
    # The wrapper's node ended (crash, the app stopping it, the marker going
    # away mid-run): look again after the throttle pause. If the marker is
    # gone the loop below just waits, as quiet as before the opt-in.
    sleep "$POLL"
  done
  sleep "$POLL"
done
