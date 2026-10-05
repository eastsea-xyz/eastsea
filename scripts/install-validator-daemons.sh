#!/usr/bin/env bash
# Move the testnet validator LaunchAgents to real LaunchDaemons so the
# validators come back after a reboot without anyone logging in
# (docs/design/29-unattended-restart.md — the incident of 2026-10-05: a
# 07:50 reboot left all four validators dark until 10:30).
#
# Each ~/Library/LaunchAgents/com.pipln.aether.testnet.*.plist becomes a
# /Library/LaunchDaemons entry: same Label, ProgramArguments (minus
# --exit-with-parent, a login-session idea), ports, data directory, log paths
# and limits, plus UserName so launchd runs it as this user from boot — root
# never runs the node itself. The matching user LaunchAgent is booted out so
# one data directory keeps one node. Everything this does is printed.
#
#   sudo scripts/install-validator-daemons.sh              install
#   scripts/install-validator-daemons.sh --dry-run         write the converted
#                                                          plists into a temp
#                                                          dir, change nothing
#   sudo scripts/install-validator-daemons.sh --uninstall  remove the daemons
#
# macOS's bash 3.2: no mapfile. PlistBuddy reads the plists (python3/jq are
# not promised on every Mac).

set -u

usage() {
  sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
  echo "options: --dry-run  --uninstall  --agents-dir DIR  --daemons-dir DIR"
}

dry=0
uninstall=0
agents="$HOME/Library/LaunchAgents"
daemons="/Library/LaunchDaemons"
daemons_given=0
while [ $# -gt 0 ]; do
  case "$1" in
    --dry-run) dry=1 ;;
    --uninstall) uninstall=1 ;;
    --agents-dir) agents=${2:?}; shift ;;
    --daemons-dir) daemons=${2:?}; shift; daemons_given=1 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
  shift
done

pb=/usr/libexec/PlistBuddy
# The daemon runs as the human who ran sudo, not as root.
me=${SUDO_USER:-$USER}
uid=$(id -u "$me" 2>/dev/null) || { echo "no such user: $me" >&2; exit 1; }

if [ "$dry" = 0 ] && [ "$(id -u)" != 0 ]; then
  echo "Run with sudo (or --dry-run): $daemons and launchctl's system domain are root's." >&2
  exit 1
fi

# --dry-run must not touch anything: the plists it writes go to a temp dir
# (an explicit --daemons-dir keeps even that predictable for the test).
if [ "$dry" = 1 ] && [ "$daemons_given" = 0 ]; then
  daemons=$(mktemp -d "${TMPDIR:-/tmp}/validator-daemons.XXXXXX")
fi
if [ "$dry" = 1 ]; then
  echo "dry run: writing converted plists under $daemons and printing the rest"
fi
mkdir -p "$daemons"

# One element per line from a plist array, exactly as PlistBuddy prints it
# ("Array {" first, "}" last; trim before dropping the brace so indented
# closers go too).
print_array() { # print_array <key> <plist>
  "$pb" -c "Print :$1" "$2" 2>/dev/null | sed -e '1d' -e 's/^ *//' -e 's/ *$//' -e '/^$/d' -e '/^}$/d'
}

converted=0
for src in "$agents"/com.pipln.aether.testnet.*.plist; do
  [ -f "$src" ] || continue
  label=$("$pb" -c 'Print :Label' "$src" 2>/dev/null) || { echo "skip (no Label): $src"; continue; }
  dst="$daemons/$label.plist"

  if [ "$uninstall" = 1 ]; then
    if [ "$dry" = 1 ]; then
      echo "would: launchctl bootout system $dst"
      echo "would: rm $dst"
    else
      echo "booting out: $label"
      launchctl bootout system "$dst" 2>/dev/null || echo "  (not loaded)"
      rm -f "$dst"
      echo "removed: $dst"
      echo "the user LaunchAgent is untouched; to return to it: launchctl bootstrap gui/$uid $src"
    fi
    continue
  fi

  # The whole plist is kept — ports, data, log paths, KeepAlive,
  # ThrottleInterval, EnvironmentVariables, resource limits — with the one
  # key a daemon needs (UserName) added and the one login-session idea
  # removed from the arguments (--exit-with-parent: the daemon has no parent
  # that dies; the agent copy did).
  cp "$src" "$dst"
  "$pb" -c "Delete :ProgramArguments" "$dst" 2>/dev/null
  if ! "$pb" -c "Add :UserName string $me" "$dst" 2>/dev/null; then
    "$pb" -c "Set :UserName $me" "$dst"
  fi
  i=0
  while IFS= read -r a; do
    [ "$a" = "--exit-with-parent" ] && { echo "  dropping --exit-with-parent from $label"; continue; }
    "$pb" -c "Add :ProgramArguments:$i string $a" "$dst"
    i=$((i + 1))
  done < <(print_array ProgramArguments "$src")
  plutil -lint "$dst" >/dev/null || { echo "converted plist is invalid: $dst" >&2; exit 1; }

  echo "converted: $src"
  echo "   into:   $dst (runs as $me from boot)"
  converted=$((converted + 1))

  if [ "$dry" = 1 ]; then
    echo "would: launchctl bootout gui/$uid $src   (keep one node per data directory)"
    echo "would: launchctl bootstrap system $dst"
    echo "would: launchctl kickstart -k system/$label"
  else
    echo "booting out the agent copy: $label"
    launchctl bootout "gui/$uid" "$src" 2>/dev/null || echo "  (not loaded)"
    launchctl bootout system "$dst" 2>/dev/null
    launchctl bootstrap system "$dst"
    launchctl kickstart -k "system/$label"
    echo "started: $label (the plist stays at $src if you ever want the agent back)"
  fi
done

if [ "$converted" = 0 ] && [ "$uninstall" = 0 ]; then
  echo "no com.pipln.aether.testnet.*.plist found in $agents — nothing to convert" >&2
  exit 1
fi
[ "$dry" = 1 ] && echo "dry run: nothing was loaded or unloaded"
exit 0
