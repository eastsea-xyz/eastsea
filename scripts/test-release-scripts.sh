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
set -eu
cd "$(dirname "$0")/.."
bad=0
fail() { echo "FAIL: $*" >&2; bad=$((bad + 1)); }
pass() { echo "ok: $*"; }

echo "=== [1/2] empty arrays under bash 3.2 set -u ==="
for f in scripts/build-wallet.sh scripts/package-mac.sh scripts/release-mac.sh; do
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

echo "=== [2/2] daemon BundleProgram ==="
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

exit "$bad"
