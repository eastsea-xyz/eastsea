#!/bin/bash
# Release-pipeline checks that need no cargo/Xcode build (0.7.0 release review,
# docs/research/release-070-migration-2026-10-07.md B3).
#
#   scripts/test-release-scripts.sh
#
# 1. Every release script runs under macOS's /bin/bash 3.2 with `set -u`, where
#    "${arr[@]}" of an EMPTY array is an "unbound variable" error. An array that
#    starts empty (`name=()`) may only be expanded guarded:
#    ${name[@]+"${name[@]}"}. (build-wallet.sh broke this way whenever
#    OTHER_SWIFT_FLAGS was unset, and package-mac.sh calls it.)
# 2. The unattended daemon's BundleProgram names a file the app build actually
#    places there, and that place is not a code location (Contents/Helpers,
#    Contents/MacOS), where an unsigned shell script fails
#    `codesign --verify --deep --strict`.
# 3. The two Sparkle feeds stay apart: EastSea reads eastsea-appcast.xml, the
#    Aether bridge (and every shipped Aether <= 0.6.6) reads appcast.xml, and
#    release-mac.sh writes EastSea's items only to eastsea-appcast.xml.
# 4. The release identity gate (scripts/release-identity-gate.sh) refuses a
#    wrong bundle id, a wrong team (new or previous), a changed designated
#    requirement, and a Terms.version change without TERMS_BUMP_REASON; each
#    check is shown failing once on a fake input.
set -eu
cd "$(dirname "$0")/.."
bad=0
fail() { echo "FAIL: $*" >&2; bad=$((bad + 1)); }
pass() { echo "ok: $*"; }

echo "=== [1/4] empty arrays under bash 3.2 set -u ==="
for f in scripts/build-wallet.sh scripts/package-mac.sh scripts/release-mac.sh scripts/build-bridge.sh scripts/release-bridge.sh; do
  [ -f "$f" ] || continue
  /bin/bash -n "$f" || fail "$f does not parse under /bin/bash $BASH_VERSION"
  for name in $(grep -oE '(^|[^A-Za-z0-9_])[A-Za-z_][A-Za-z0-9_]*=\(\)' "$f" | sed -E 's/^[^A-Za-z_]//; s/=\(\)$//' | sort -u); do
    # Any "${name[@]}" that is not inside the ${name[@]+...} guard.
    unguarded=$(grep -nE "\"\\\$\{$name\[@\]\}\"" "$f" | grep -vE "\\\$\{$name\[@\]\+\"\\\$\{$name\[@\]\}\"\}" || true)
    if [ -n "$unguarded" ]; then
      fail "$f expands the possibly-empty array $name unguarded: $unguarded"
    fi
  done
done
# The guard itself does what it claims on this very shell.
out=$(/bin/bash -c 'set -u; a=(); f() { echo $#; }; f ${a[@]+"${a[@]}"}; a=(x "y z"); f ${a[@]+"${a[@]}"}' 2>&1 | tr '\n' ' ')
[ "$out" = "0 2 " ] && pass "guarded expansion: empty -> 0 args, two -> 2 args" || fail "guarded expansion gave '$out'"
[ "$bad" -eq 0 ] && pass "no unguarded empty-array expansion in the release scripts"

echo "=== [2/4] daemon BundleProgram ==="
plist=apps/wallet/Daemons/com.pipln.eastsea.node.plist
prog=$(/usr/libexec/PlistBuddy -c 'Print :BundleProgram' "$plist")
case "$prog" in
  Contents/Resources/*) pass "BundleProgram $prog is under Contents/Resources (sealed resource, not nested code)" ;;
  *) fail "BundleProgram $prog must be a bundle-root-relative path under Contents/Resources" ;;
esac
stub=$(basename "$prog")
[ -f "apps/wallet/Helpers/$stub" ] || fail "the stub $stub is not in apps/wallet/Helpers"
grep -q 'R="$BUILT_PRODUCTS_DIR/$CONTENTS_FOLDER_PATH/Resources"' apps/wallet/project.yml \
  && grep -q 'cp -f "$SRCROOT/Helpers/$s" "$R/"' apps/wallet/project.yml \
  && pass "project.yml copies the stubs into Contents/Resources" \
  || fail "project.yml does not copy the daemon stubs into Contents/Resources"
grep -q 'wrapper="$bundle/Contents/Resources/eastsea-node-wrapper.sh"' apps/wallet/Helpers/eastsea-node-daemon.sh \
  && pass "the root stub looks for the wrapper in Contents/Resources" \
  || fail "the root stub looks for the wrapper somewhere the build does not put it"

echo "=== [3/4] Sparkle feeds ==="
feed() { /usr/libexec/PlistBuddy -c 'Print :SUFeedURL' "$1"; }
east=$(feed apps/wallet/Info-mac.plist)
case "$east" in
  */eastsea-appcast.xml) pass "EastSea reads its own feed ($east)" ;;
  *) fail "EastSea must not read the legacy appcast.xml that Aether 0.6.6 reads (got $east)" ;;
esac
if [ -f apps/bridge/Info.plist ]; then
  bridge=$(feed apps/bridge/Info.plist)
  case "$bridge" in
    */releases/latest/download/appcast.xml) pass "the bridge reads the legacy feed ($bridge)" ;;
    *) fail "the bridge must stay on the legacy feed (got $bridge)" ;;
  esac
else
  fail "apps/bridge/Info.plist is missing"
fi
grep -q 'cat > dist/eastsea-appcast.xml' scripts/release-mac.sh \
  && ! grep -q 'cat > dist/appcast.xml' scripts/release-mac.sh \
  && pass "release-mac.sh writes EastSea items only to eastsea-appcast.xml" \
  || fail "release-mac.sh must write EastSea items to eastsea-appcast.xml, never to appcast.xml"

echo "=== [4/4] release identity gate ==="
grep -q 'scripts/release-identity-gate.sh release "$dmg" "$tag" "$repo"' scripts/release-mac.sh \
  && pass "release-mac.sh runs the identity gate after packaging" \
  || fail "release-mac.sh does not run the identity gate"
g=$PWD/tmp/identity-gate-test
rm -rf "$g"; mkdir -p "$g/bin"
# A fake codesign: the team and the requirement come from files in the bundle.
cat > "$g/bin/codesign" <<'SH'
#!/bin/bash
app=${!#}
case "$1" in
  -dv) echo "Identifier=x" >&2; echo "TeamIdentifier=$(cat "$app/fake-team")" >&2 ;;
  -dr) echo "Executable=$app/Contents/MacOS/EastSea" >&2; echo "designated => $(cat "$app/fake-req")" ;;
esac
SH
chmod +x "$g/bin/codesign"
req='identifier "com.pipln.eastsea" and anchor apple generic and certificate leaf[subject.OU] = "45WU468FZE"'
mkapp() { # dir bundle-id team requirement
  mkdir -p "$1/Contents"
  /usr/libexec/PlistBuddy -c "Add :CFBundleIdentifier string $2" "$1/Contents/Info.plist" >/dev/null
  printf '%s' "$3" > "$1/fake-team"; printf '%s' "$4" > "$1/fake-req"
}
mkapp "$g/good.app" com.pipln.eastsea 45WU468FZE "$req"
mkapp "$g/prev.app" com.pipln.eastsea 45WU468FZE "$req"
mkapp "$g/wrong-id.app" com.pipln.aether 45WU468FZE "$req"
mkapp "$g/wrong-team.app" com.pipln.eastsea ABCDE12345 "$req"
mkapp "$g/prev-wrong-team.app" com.pipln.eastsea ABCDE12345 "$req"
mkapp "$g/new-req.app" com.pipln.eastsea 45WU468FZE 'identifier "com.pipln.eastsea" and anchor apple generic'
printf '    static let version = isTestnet ? 5 : 6\n' > "$g/terms-old.swift"
printf '    static let version = isTestnet ? 6 : 7\n' > "$g/terms-new.swift"
gate() { # expected-refusal app prev terms prev-terms
  local want=$1 out rc; shift
  out=$(PATH="$g/bin:$PATH" scripts/release-identity-gate.sh check --app "$1" --prev-app "$2" \
    --terms "$3" --prev-terms "$4" --log "$g/release-gates.log" 2>&1) && rc=0 || rc=$?
  if [ -z "$want" ]; then
    [ "$rc" -eq 0 ] && pass "gate passes: $(basename "$1") vs $(basename "$2")" || fail "gate refused a good release: $out"
  else
    [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q "REFUSED: $want" \
      && pass "gate refuses: $want" || fail "gate did not refuse '$want' (rc $rc): $out"
  fi
}
unset TERMS_BUMP_REASON
gate "" "$g/good.app" "$g/prev.app" "$g/terms-old.swift" "$g/terms-old.swift"
gate "bundle id is 'com.pipln.aether'" "$g/wrong-id.app" "$g/prev.app" "$g/terms-old.swift" "$g/terms-old.swift"
gate "new app is signed by team 'ABCDE12345'" "$g/wrong-team.app" "$g/prev.app" "$g/terms-old.swift" "$g/terms-old.swift"
gate "previous release is signed by team 'ABCDE12345'" "$g/good.app" "$g/prev-wrong-team.app" "$g/terms-old.swift" "$g/terms-old.swift"
gate "designated requirement changed" "$g/new-req.app" "$g/prev.app" "$g/terms-old.swift" "$g/terms-old.swift"
gate "Terms.version changed" "$g/good.app" "$g/prev.app" "$g/terms-new.swift" "$g/terms-old.swift"
TERMS_BUMP_REASON="   " gate "Terms.version changed" "$g/good.app" "$g/prev.app" "$g/terms-new.swift" "$g/terms-old.swift"
TERMS_BUMP_REASON="new risk: validator slashing" gate "" "$g/good.app" "$g/prev.app" "$g/terms-new.swift" "$g/terms-old.swift"
grep -q "reason: new risk: validator slashing" "$g/release-gates.log" \
  && pass "the bump reason is written to the gates log" || fail "the bump reason is not in the gates log"
# The real codesign on an ad-hoc signed bundle: no Developer ID team at all.
mkdir -p "$g/adhoc.app/Contents/MacOS"
/usr/libexec/PlistBuddy -c "Add :CFBundleIdentifier string com.pipln.eastsea" -c "Add :CFBundleExecutable string EastSea" "$g/adhoc.app/Contents/Info.plist" >/dev/null
cp /usr/bin/true "$g/adhoc.app/Contents/MacOS/EastSea"
codesign -s - --force "$g/adhoc.app" 2>/dev/null
out=$(scripts/release-identity-gate.sh check --app "$g/adhoc.app" --prev-app "$g/adhoc.app" \
  --terms "$g/terms-old.swift" --prev-terms "$g/terms-old.swift" --log "$g/release-gates.log" 2>&1) && rc=0 || rc=$?
[ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q "REFUSED: new app is signed by team 'not set'" \
  && pass "gate refuses a real ad-hoc signature (team not set)" || fail "ad-hoc signature not refused (rc $rc): $out"
# The real tool's output format is what the gate parses, on the installed app.
if [ -d /Applications/EastSea.app ]; then
  codesign -dr - /Applications/EastSea.app 2>/dev/null | grep -q '^designated => identifier "com.pipln.eastsea"' \
    && pass "codesign -dr - prints the 'designated =>' line the gate compares" \
    || fail "codesign -dr - output format changed"
fi

exit "$bad"
