#!/bin/bash
# Observe the shipped app on a canary Mac. No node/validator tools are invoked.
#   scripts/release-canary.sh [--dry-run] <dmg> <host>
# Requires an SSH user logged into that Mac's GUI. Noninteractive sudo is only
# needed if the installed app or /Applications is not writable by that user.
# Backups and evidence remain in ~/EastSea-canary-backup/<run>/.
set -euo pipefail

fail() { printf 'FAIL: canary: %s\n' "$*" >&2; exit 1; }
usage() {
  echo 'Usage: scripts/release-canary.sh [--dry-run] <dmg> <host>'
  echo 'AETHER_CANARY_SECONDS: observation duration, at least 1800 (default 1800).'
  echo 'AETHER_CANARY_POLL_SECONDS: polling interval, 1 through 30 (default 30).'
}
dry_run=0
case "${1:-}" in
  --dry-run) dry_run=1; shift ;;
  --help|-h) usage; exit 0 ;;
esac
[ "$#" -eq 2 ] || { usage >&2; exit 2; }
dmg=$1
host=$2
[[ "$host" =~ ^([A-Za-z_][A-Za-z0-9_.-]*@)?[A-Za-z0-9][A-Za-z0-9_.-]*$ ]] \
  || fail 'host must be an SSH alias, hostname, or IPv4 address (optionally user@host)'
target=$(printf '%s' "${host##*@}" | /usr/bin/tr '[:upper:]' '[:lower:]')
case "$host" in root@*) fail 'the canary SSH user must own a logged-in GUI session, not be root' ;; esac
case "$target" in
  localhost|localhost.*|127.*|0.0.0.0|poc-cuda|poc-cuda.*|poc-nas|poc-nas.*|100.121.197.74|100.100.59.78)
    fail "unsafe canary host: $host" ;;
esac
local_name=$(/bin/hostname | /usr/bin/tr '[:upper:]' '[:lower:]')
if [ "$target" = "$local_name" ] || [ "$target" = "${local_name%%.*}" ]; then
  fail "canary host is this release Mac: $host"
fi
seconds=${AETHER_CANARY_SECONDS:-1800}
poll_seconds=${AETHER_CANARY_POLL_SECONDS:-30}
case "$seconds:$poll_seconds" in *[!0-9:]*|:*|*:) fail 'canary duration and polling interval must be positive integer seconds' ;; esac
[ "${#seconds}" -le 8 ] && [ "${#poll_seconds}" -le 2 ] || fail 'canary duration or polling interval is out of range'
seconds=$((10#$seconds))
poll_seconds=$((10#$poll_seconds))
[ "$seconds" -ge 1800 ] || fail 'the real canary observation must last at least 1800 seconds'
[ "$poll_seconds" -ge 1 ] && [ "$poll_seconds" -le 30 ] || fail 'canary polling interval must be between 1 and 30 seconds'

# This branch deliberately precedes filesystem/tool checks: a release dry run
# can name an artifact that has not been built, even with no dist/ directory.
if [ "$dry_run" -eq 1 ]; then
  printf 'DRY-RUN: canary host=%s dmg=%s duration=%ss poll=%ss grace=30s\n' "$host" "$dmg" "$seconds" "$poll_seconds"
  printf 'DRY-RUN CHANGE [%s]: create ~/EastSea-canary-backup/<run>/tmp/ and evidence files\n' "$host"
  printf 'DRY-RUN CHANGE [%s]: copy %s to ~/EastSea-canary-backup/<run>/tmp/shipped.dmg\n' "$host" "$dmg"
  printf 'DRY-RUN CHANGE [%s]: mount that DMG read-only and stage its verified EastSea.app in <run>/tmp/install/\n' "$host"
  printf 'DRY-RUN CHANGE [%s]: terminate only the existing /Applications/EastSea.app GUI process, if running\n' "$host"
  printf 'DRY-RUN CHANGE [%s]: retain /Applications/EastSea.app at ~/EastSea-canary-backup/<run>/previous/EastSea.app, then install the staged app\n' "$host"
  printf 'DRY-RUN CHANGE [%s]: launch /Applications/EastSea.app as the logged-in SSH user; retain crash snapshots and PID observations\n' "$host"
  printf 'DRY-RUN: require a different Mac, the same app PID for %ss, and no new/changed EastSea crash reports through the 30s reporting grace\n' "$seconds"
  exit 0
fi

[ -f "$dmg" ] || fail "DMG not found: $dmg"
[ "$(/usr/bin/uname -s)" = Darwin ] || fail 'the release host must be macOS'
for tool in ssh scp; do command -v "$tool" >/dev/null || fail "missing dependency: $tool"; done
local_uuid=$(/usr/sbin/ioreg -rd1 -c IOPlatformExpertDevice | /usr/bin/awk -F '"' '/"IOPlatformUUID"/{print $(NF-1); exit}')
[[ "$local_uuid" =~ ^[A-Fa-f0-9-]+$ ]] || fail 'cannot establish the release Mac hardware UUID'
dmg_hash=$(/usr/bin/shasum -a 256 -- "$dmg" | /usr/bin/awk '{print $1}')
root=$(cd "$(dirname "$0")/.." && pwd -P)
run_id="$(/bin/date -u +%Y%m%dT%H%M%SZ)-$$-$RANDOM"
work="$root/tmp/release-canary-$run_id"
printf 'CHANGE [local]: create %s for the remote script and transport/evidence log\n' "$work"
mkdir -p "$work"
export TMPDIR="$work"
log="$work/canary.log"
remote_script="$work/remote.sh"
local_finish() {
  local status=$?
  if [ "$status" -ne 0 ]; then
    printf 'FAIL: canary host=%s; transport/install/observation failed; evidence: %s and ~/EastSea-canary-backup/%s/ on the canary\n' "$host" "$work" "$run_id" >&2
  fi
}
trap local_finish EXIT

cat > "$remote_script" <<'REMOTE'
#!/bin/bash
set -euo pipefail
phase=$1
run_id=$2
expected_hash=$3
seconds=$4
poll_seconds=$5
release_uuid=$6
fail() { printf 'FAIL: canary: %s\n' "$*" >&2; exit 1; }
change() { printf 'CHANGE [canary]: %s\n' "$*"; }
[ "$(/usr/bin/uname -s)" = Darwin ] || fail 'target is not macOS'
remote_uuid=$(/usr/sbin/ioreg -rd1 -c IOPlatformExpertDevice | /usr/bin/awk -F '"' '/"IOPlatformUUID"/{print $(NF-1); exit}')
[[ "$remote_uuid" =~ ^[A-Fa-f0-9-]+$ ]] || fail 'cannot establish the canary Mac hardware UUID'
[ "$remote_uuid" != "$release_uuid" ] || fail 'SSH target resolves to the release Mac; refusing all changes'
user=$(/usr/bin/id -un)
uid=$(/usr/bin/id -u)
[ "$uid" -ne 0 ] || fail 'SSH target user is root'
[ "$(/usr/bin/stat -f %u /dev/console)" = "$uid" ] || fail "SSH user $user does not own the logged-in GUI session"
[ -d "$HOME" ] && [ "$HOME" != / ] || fail 'target user home is invalid'
base="$HOME/EastSea-canary-backup"
run="$base/$run_id"
scratch="$run/tmp"
[ ! -L "$base" ] && [ ! -L "$run" ] || fail 'canary backup path is a symlink'
app=/Applications/EastSea.app
app_directory=${app%/*}
exe="$app/Contents/MacOS/EastSea"
reports="$HOME/Library/Logs/DiagnosticReports"
printf 'EVIDENCE: canary user=%s uid=%s hardware=%s run=%s\n' "$user" "$uid" "$remote_uuid" "$run"
if [ "$phase" = prepare ]; then
  [ ! -e "$run" ] || fail 'canary run directory already exists'
  change "mkdir -p $base, $run, $scratch, $scratch/mount, $scratch/install and $run/previous (new directories mode 700; existing permissions retained)"
  umask 077
  /bin/mkdir -p "$base" "$run" "$scratch" "$scratch/mount" "$scratch/install" "$run/previous"
  exit 0
fi
[ "$phase" = observe ] || fail 'unknown canary phase'
[ -d "$scratch" ] && [ ! -L "$scratch" ] || fail 'prepared canary scratch directory is missing or unsafe'

app_pids() {
  # comm is the executable path, not an arbitrary command-line substring.
  /bin/ps -ww -axo pid=,uid=,comm= | /usr/bin/awk -v u="$uid" -v p="$exe" '$2 == u && $3 == p && NF == 3 {print $1}'
}
uptime_seconds() {
  # Foundation's monotonic uptime prevents a clock adjustment from shortening
  # the observation. This uses the stock JXA bridge and sends no Apple events.
  /usr/bin/osascript -l JavaScript -e 'ObjC.import("Foundation"); Math.floor($.NSProcessInfo.processInfo.systemUptime)'
}
snapshot_reports() {
  local output=$1 report hash
  change "write crash-report snapshot $output"
  (
    shopt -s nullglob
    for report in "$reports"/EastSea-* "$reports"/EastSea_*; do
      [ -f "$report" ] || continue
      hash=$(/usr/bin/shasum -a 256 -- "$report" | /usr/bin/awk '{print $1}')
      printf '%s\t%s\n' "$report" "$hash"
    done
  ) | LC_ALL=C /usr/bin/sort > "$output"
}
verify_app() {
  local candidate=$1 team details
  [ -d "$candidate" ] && [ ! -L "$candidate" ] || fail "EastSea.app missing or symlinked: $candidate"
  [ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$candidate/Contents/Info.plist")" = com.pipln.eastsea ] || fail 'shipped app has the wrong bundle identifier'
  [ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$candidate/Contents/Info.plist")" = EastSea ] || fail 'shipped app has the wrong executable'
  /usr/bin/codesign --verify --deep --strict "$candidate"
  details=$(/usr/bin/codesign -dv --verbose=4 "$candidate" 2>&1)
  team=$(printf '%s\n' "$details" | /usr/bin/awk -F= '/^TeamIdentifier=/{print $2}')
  [ "$team" = 45WU468FZE ] || fail "shipped app has an unexpected signing team: $team"
  /usr/sbin/spctl --assess --type execute "$candidate"
}
observe() {
  # Keep cleanup state available to the EXIT handler and the normal path.
  mounted=0
  installed=0
  moved_previous=0
  install_with_sudo=0
  install_command() {
    if [ "$install_with_sudo" = 1 ]; then
      /usr/bin/sudo -n "$@"
    else
      "$@"
    fi
  }
  local install_method="as SSH user $user"
  local pid='' fingerprint='' start now elapsed remaining delay old_pids old_pid waited new_reports current
  # Invoked indirectly by the EXIT trap.
  # shellcheck disable=SC2329
  finish() {
    local status=$?
    trap - EXIT
    if [ "$moved_previous" = 1 ] && [ "$installed" = 0 ] && [ ! -e "$app" ]; then
      change "restore $run/previous/EastSea.app to $app after an install failure"
      install_command /bin/mv "$run/previous/EastSea.app" "$app" || {
        printf 'FAIL: canary: could not restore previous app to %s %s\n' "$app" "$install_method" >&2
        status=1
      }
    fi
    if [ "$mounted" = 1 ]; then
      change "detach read-only DMG at $scratch/mount"
      /usr/bin/hdiutil detach -quiet "$scratch/mount" || status=1
    fi
    if [ "$status" -ne 0 ]; then
      printf 'FAIL: canary; evidence retained at %s\n' "$run" >&2
    fi
    exit "$status"
  }
  trap finish EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM HUP
  [ -f "$scratch/shipped.dmg" ] && [ ! -L "$scratch/shipped.dmg" ] || fail 'staged DMG is missing or unsafe'
  [ "$(/usr/bin/shasum -a 256 "$scratch/shipped.dmg" | /usr/bin/awk '{print $1}')" = "$expected_hash" ] || fail 'copied DMG SHA-256 differs from the release artifact'
  /usr/bin/hdiutil verify -quiet "$scratch/shipped.dmg"
  change "mount $scratch/shipped.dmg read-only at $scratch/mount"
  mounted=1
  /usr/bin/hdiutil attach -quiet -readonly -nobrowse -mountpoint "$scratch/mount" "$scratch/shipped.dmg"
  verify_app "$scratch/mount/EastSea.app"
  change "copy verified shipped app to $scratch/install/EastSea.app as SSH user $user"
  /usr/bin/ditto "$scratch/mount/EastSea.app" "$scratch/install/EastSea.app"
  verify_app "$scratch/install/EastSea.app"
  snapshot_reports "$run/crashes-before.tsv"
  [ -d "$app_directory" ] && [ ! -L "$app_directory" ] && [ ! -L "$app" ] || fail 'installed application directory is missing or symlinked'
  [ ! -e "$app" ] || [ -d "$app" ] || fail 'installed EastSea.app is not a directory'
  if [ ! -w "$app_directory" ] || { [ -e "$app" ] && [ ! -w "$app" ]; }; then
    /usr/bin/sudo -n -v >/dev/null 2>&1 \
      || fail "cannot write $app or $app_directory as SSH user $user; noninteractive sudo is unavailable"
    install_with_sudo=1
    install_method='with sudo -n'
  fi
  printf 'EVIDENCE: installation permission selected: %s\n' "$install_method"
  old_pids=$(app_pids)
  for old_pid in $old_pids; do
    change "send TERM to the existing GUI EastSea process PID $old_pid ($exe, uid=$uid)"
    /bin/kill -TERM "$old_pid"
  done
  waited=0
  while [ -n "$(app_pids)" ]; do
    [ "$waited" -lt 30 ] || fail 'previous GUI app did not terminate within 30 seconds'
    /bin/sleep 1
    waited=$((waited + 1))
  done
  if [ -e "$app" ]; then
    change "move previous $app to $run/previous/EastSea.app (retained backup) $install_method"
    install_command /bin/mv "$app" "$run/previous/EastSea.app" \
      || fail "could not retain previous app $install_method"
    moved_previous=1
  fi
  change "move $scratch/install/EastSea.app to $app $install_method"
  install_command /bin/mv "$scratch/install/EastSea.app" "$app" \
    || fail "could not install app in $app_directory $install_method"
  installed=1
  verify_app "$app"
  change "launch $app as GUI user $user (uid=$uid) using open"
  /usr/bin/open "$app"
  waited=0
  while [ -z "$pid" ]; do
    pid=$(app_pids)
    [ "$waited" -lt 30 ] || fail 'new GUI app did not start within 30 seconds'
    if [ -z "$pid" ]; then /bin/sleep 1; waited=$((waited + 1)); fi
  done
  case "$pid" in *[!0-9]*|'') fail 'expected exactly one GUI EastSea process after launch' ;; esac
  fingerprint=$(/bin/ps -ww -p "$pid" -o uid=,lstart=,comm=)
  start=$(uptime_seconds)
  case "$start" in *[!0-9]*|'') fail 'cannot read the monotonic observation clock' ;; esac
  printf 'EVIDENCE: installed SHA-256=%s GUI PID=%s observation=%ss reporting-grace=30s\n' "$expected_hash" "$pid" "$seconds"
  change "create $run/observations.log; append each PID/crash observation"
  : > "$run/observations.log"
  while :; do
    current=$(/bin/ps -ww -p "$pid" -o uid=,lstart=,comm=) || fail "GUI EastSea PID $pid exited during observation"
    [ "$current" = "$fingerprint" ] || fail "GUI EastSea PID $pid changed identity or restarted during observation"
    snapshot_reports "$run/crashes-latest.tsv"
    new_reports=$(LC_ALL=C /usr/bin/comm -13 "$run/crashes-before.tsv" "$run/crashes-latest.tsv")
    if [ -n "$new_reports" ]; then
      printf 'EVIDENCE: new/changed crash reports (path and SHA-256):\n%s\n' "$new_reports"
      fail 'new or changed EastSea diagnostic report'
    fi
    now=$(uptime_seconds)
    case "$now" in *[!0-9]*|'') fail 'cannot read the monotonic observation clock' ;; esac
    [ "$now" -ge "$start" ] || fail 'monotonic observation clock moved backwards'
    elapsed=$((now - start))
    printf 'EVIDENCE: elapsed=%ss PID=%s unchanged crash reports=0\n' "$elapsed" "$pid" | /usr/bin/tee -a "$run/observations.log"
    # Keep the app alive for an additional 30 seconds so delayed .ips reports
    # do not arrive just after a successful observation.
    remaining=$((seconds + 30 - elapsed))
    [ "$remaining" -gt 0 ] || break
    delay=$poll_seconds
    [ "$remaining" -ge "$delay" ] || delay=$remaining
    /bin/sleep "$delay"
  done
  change "detach read-only DMG at $scratch/mount"
  /usr/bin/hdiutil detach -quiet "$scratch/mount"
  mounted=0
  printf 'PASS: canary installed DMG SHA-256=%s; GUI PID=%s survived %ss; no new/changed EastSea diagnostic reports; evidence: %s\n' "$expected_hash" "$pid" "$elapsed" "$run"
}
change "create $run/evidence.log for install/observation output"
observe 2>&1 | /usr/bin/tee "$run/evidence.log"
REMOTE
/bin/bash -n "$remote_script"
ssh_args=(-o BatchMode=yes -o ConnectTimeout=15 -o ServerAliveInterval=30 -o ServerAliveCountMax=3)
# All command arguments below are constrained to digits/hex/hyphens above.
remote_args="'$run_id' '$dmg_hash' '$seconds' '$poll_seconds' '$local_uuid'"
printf 'EVIDENCE: source DMG SHA-256=%s; target=%s; remote evidence=~/EastSea-canary-backup/%s/\n' "$dmg_hash" "$host" "$run_id" | tee -a "$log"
# shellcheck disable=SC2029
ssh "${ssh_args[@]}" "$host" "/bin/bash -s -- prepare $remote_args" < "$remote_script" 2>&1 | tee -a "$log"
printf 'CHANGE [%s]: copy %s to ~/EastSea-canary-backup/%s/tmp/shipped.dmg\n' "$host" "$dmg" "$run_id" | tee -a "$log"
scp "${ssh_args[@]}" -- "$dmg" "$host:EastSea-canary-backup/$run_id/tmp/shipped.dmg" 2>&1 | tee -a "$log"
# shellcheck disable=SC2029
ssh "${ssh_args[@]}" "$host" "/bin/bash -s -- observe $remote_args" < "$remote_script" 2>&1 | tee -a "$log"
printf 'PASS: canary host=%s; local evidence: %s\n' "$host" "$log"
