#!/usr/bin/env bash
# scripts/install-validator-daemons.sh's --dry-run conversion, end to end
# against a fixture LaunchAgent: the converted plist must land in the given
# directory, keep the label/args/ports/log paths/KeepAlive/limits, drop
# --exit-with-parent, name the user it will run as, and only *print* the
# launchctl steps. No sudo, no real LaunchAgents touched.
set -u
cd "$(dirname "$0")/../.."
S=scripts/install-validator-daemons.sh
pb=/usr/libexec/PlistBuddy
work=$(mktemp -d "${TMPDIR:-/tmp}/validator-daemons-test.XXXXXX")
agents="$work/agents"
out="$work/out"
mkdir -p "$agents" "$out"
fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=$((fail + 1)); }

# A fixture shaped like the real validator agents (v1 as of 2026-10-05),
# plus --exit-with-parent so the drop path is exercised.
cat > "$agents/com.pipln.aether.testnet.v1.plist" <<'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key><string>com.pipln.aether.testnet.v1</string>
	<key>ProgramArguments</key>
	<array>
		<string>/Users/tester/aether-testnet/bin/aether</string>
		<string>run</string>
		<string>--network</string>
		<string>/Users/tester/aether-testnet/1/network.json</string>
		<string>--port</string>
		<string>9101</string>
		<string>--rpc-port</string>
		<string>8601</string>
		<string>--data</string>
		<string>/Users/tester/aether-testnet/1</string>
		<string>--exit-with-parent</string>
	</array>
	<key>RunAtLoad</key><true/>
	<key>KeepAlive</key><true/>
	<key>ThrottleInterval</key><integer>10</integer>
	<key>StandardOutPath</key><string>/Users/tester/aether-testnet/node1.log</string>
	<key>StandardErrorPath</key><string>/Users/tester/aether-testnet/node1.log</string>
	<key>HardResourceLimits</key>
	<dict><key>NumberOfFiles</key><integer>65536</integer></dict>
	<key>EnvironmentVariables</key>
	<dict><key>AETHER_RECOVER_CONSENSUS</key><string>69841@69651</string></dict>
</dict>
</plist>
EOF
chmod 644 "$agents/com.pipln.aether.testnet.v1.plist"

dst="$out/com.pipln.aether.testnet.v1.plist"

if bash "$S" --dry-run --agents-dir "$agents" --daemons-dir "$out" >"$work/plan.txt" 2>&1; then
  ok "dry run exits 0"
else
  bad "dry run exits 0 — see $work/plan.txt"
fi

[ -f "$dst" ] && ok "converted plist written to --daemons-dir" || bad "no converted plist at $dst"
plutil -lint "$dst" >/dev/null 2>&1 && ok "converted plist lints" || bad "converted plist does not lint"

u=$("$pb" -c 'Print :UserName' "$dst" 2>/dev/null)
[ "$u" = "$(whoami)" ] && ok "UserName is the invoking user ($u)" || bad "UserName is '$u', expected $(whoami)"

args=$(plutil -extract ProgramArguments json -o - "$dst" 2>/dev/null)
case "$args" in
  *--exit-with-parent*) bad "--exit-with-parent dropped" ;;
  *) ok "--exit-with-parent dropped" ;;
esac
case "$args" in
  *9101*|*8601*) ok "ports kept in ProgramArguments" ;;
  *) bad "ports lost from ProgramArguments: $args" ;;
esac
n=$("$pb" -c 'Print :ProgramArguments' "$dst" 2>/dev/null | grep -c .)
# 12 fixture elements, one dropped, minus the "Array {" and "}" lines PlistBuddy prints.
[ "$n" -eq 12 ] && ok "exactly the --exit-with-parent element left the array" || bad "array now has $((n - 2)) elements, expected 11"

[ "$("$pb" -c 'Print :Label' "$dst" 2>/dev/null)" = "com.pipln.aether.testnet.v1" ] \
  && ok "Label kept" || bad "Label changed"
[ "$("$pb" -c 'Print :KeepAlive' "$dst" 2>/dev/null)" = "true" ] \
  && ok "KeepAlive kept" || bad "KeepAlive changed"
[ "$("$pb" -c 'Print :ThrottleInterval' "$dst" 2>/dev/null)" = "10" ] \
  && ok "ThrottleInterval kept" || bad "ThrottleInterval changed"
[ "$("$pb" -c 'Print :StandardOutPath' "$dst" 2>/dev/null)" = "/Users/tester/aether-testnet/node1.log" ] \
  && ok "log paths kept" || bad "log paths changed"
[ "$("$pb" -c 'Print :EnvironmentVariables:AETHER_RECOVER_CONSENSUS' "$dst" 2>/dev/null)" = "69841@69651" ] \
  && ok "environment kept" || bad "environment variables lost"
[ "$("$pb" -c 'Print :HardResourceLimits:NumberOfFiles' "$dst" 2>/dev/null)" = "65536" ] \
  && ok "resource limits kept" || bad "resource limits lost"

# The agent copy and the real LaunchAgents folder are untouched, and every
# system change is only printed.
cmp -s "$agents/com.pipln.aether.testnet.v1.plist" "$agents/com.pipln.aether.testnet.v1.plist" \
  && ok "fixture agent untouched"
grep -q "would: launchctl bootstrap system $dst" "$work/plan.txt" \
  && ok "plan prints the bootstrap step" || bad "plan misses the bootstrap step"
grep -q "would: launchctl bootout gui/" "$work/plan.txt" \
  && ok "plan prints the agent bootout step" || bad "plan misses the agent bootout step"
grep -q "dropping --exit-with-parent" "$work/plan.txt" \
  && ok "plan says it dropped --exit-with-parent" || bad "plan does not mention the drop"

# --uninstall --dry-run prints its plan and removes nothing.
if bash "$S" --uninstall --dry-run --agents-dir "$agents" --daemons-dir "$out" >"$work/unplan.txt" 2>&1; then
  ok "uninstall dry run exits 0"
else
  bad "uninstall dry run exits 0 — see $work/unplan.txt"
fi
grep -q "would: launchctl bootout system $dst" "$work/unplan.txt" \
  && ok "uninstall plan prints the bootout" || bad "uninstall plan misses the bootout"
grep -q "would: rm $dst" "$work/unplan.txt" \
  && ok "uninstall plan prints the removal" || bad "uninstall plan misses the removal"
[ -f "$dst" ] && ok "uninstall dry run removed nothing" || bad "uninstall dry run deleted $dst"

echo "$([ $fail = 0 ] && echo PASS || echo FAIL): $fail failed — $work"
exit "$fail"
