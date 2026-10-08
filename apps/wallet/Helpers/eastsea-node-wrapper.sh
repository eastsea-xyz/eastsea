#!/bin/bash
# EastSea unattended-restart user wrapper (docs/design/29-unattended-restart.md).
# The root stub drops to this as the node's user. It reads what the app wrote
# into the marker (binary, argv, proving address), keeps the Mac awake while
# the node lives, records its pid so the app can stop it, and becomes the node
# (exec: same pid, so the pid file and caffeinate stay truthful). The node's
# own run.lock is what keeps this and an app-started node from ever running
# together. macOS's bash 3.2: no mapfile.

set -u

marker=${1:-}
[ -f "$marker" ] || exit 1

pb=/usr/libexec/PlistBuddy
data=$($pb -c 'Print :data' "$marker" 2>/dev/null) || exit 1
binary=$($pb -c 'Print :binary' "$marker" 2>/dev/null) || exit 1
prove=$($pb -c 'Print :prove' "$marker" 2>/dev/null)
country_choice=$($pb -c 'Print :presence_country_choice' "$marker" 2>/dev/null)
country=$($pb -c 'Print :presence_country' "$marker" 2>/dev/null)

# The argv array in order: the marker's XML is exactly
# <key>argv</key><array><string>…</string>…</array>.
args=()
while IFS= read -r line; do
  args+=("$line")
done < <(/usr/bin/plutil -convert xml1 -o - "$marker" | sed -n '/<key>argv<\/key>/,/<\/array>/p' | sed -n 's/^[[:space:]]*<string>\(.*\)<\/string>[[:space:]]*$/\1/p')

# A pre-notice marker from an older app may contain a country. Both founder
# modes require a new screen answer: preserve only the exact country recorded
# by the current app after an affirmative choice. A decline or missing choice
# still starts the same node, with its ordinary country-free arguments.
filtered_args=()
skip_country_value=false
for argument in "${args[@]}"; do
  if [ "$skip_country_value" = true ]; then
    skip_country_value=false
    continue
  fi
  case "$argument" in
    --presence-country)
      skip_country_value=true
      ;;
    --presence-country=*)
      if [ "$country_choice" = share ] && [[ "$country" = [A-Z][A-Z] ]] && [ "$argument" = "--presence-country=$country" ]; then
        filtered_args+=("$argument")
      fi
      ;;
    *) filtered_args+=("$argument") ;;
  esac
done
args=("${filtered_args[@]}")

[ -x "$binary" ] || exit 1
[ "${args[0]:-}" = "run" ] || exit 1
mkdir -p "$data" || exit 1

echo $$ > "$data/unattended.pid"

# A voting Mac must not idle-sleep while nobody is logged in (the display may
# sleep). Watch this shell's pid: exec keeps it, so this lives exactly as
# long as the node does.
/usr/bin/caffeinate -s -w $$ &

cd "$data" || exit 1
if [ -n "$prove" ]; then
  exec /usr/bin/env AETHER_PROVE="$prove" "$binary" "${args[@]}" >> "$data/node.log" 2>&1
fi
exec "$binary" "${args[@]}" >> "$data/node.log" 2>&1
