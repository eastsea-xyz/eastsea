#!/bin/bash
# Release-pipeline checks that need no cargo/Xcode build (0.7.0 release review,
# docs/research/release-070-migration-2026-10-07.md B3).
#
#   scripts/test-release-scripts.sh
#   AETHER_TEST_PACKAGE_MAC=<file> scripts/test-release-scripts.sh
#       Run the rollback and installer regressions against an older packager.
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
# 5. A clean package contains a verified previous node and its adjacent prover;
#    invalid signatures, dependencies or platform compatibility refuse packaging.
#    Its installer background, Finder layout and Applications shortcut survive
#    writable mounting and conversion to the final compressed DMG.
set -eu
cd "$(dirname "$0")/.."
mkdir -p "$PWD/tmp"
export TMPDIR="$PWD/tmp"
bad=0
good=0
fail() { echo "FAIL: $*" >&2; bad=$((bad + 1)); }
pass() { echo "ok: $*"; good=$((good + 1)); }

echo "=== [1/5] empty arrays under bash 3.2 set -u ==="
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

echo "=== [2/5] daemon BundleProgram ==="
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

echo "=== [3/5] Sparkle feeds ==="
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

echo "=== [4/5] release identity gate ==="
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
# R12: exercise the real packaging script with a clean-build fixture and tool
# doubles. No node builds, network calls, real DMGs or installed apps are used.
echo "=== [5/5] R12 clean package rollback ==="
mkdir -p "$PWD/tmp"
rollback_test=$(mktemp -d "$PWD/tmp/package-rollback-test.XXXXXX")
trap 'rm -rf "${rollback_test:?}"' EXIT
mkdir -p "$rollback_test/scripts" "$rollback_test/bin" "$rollback_test/tmp" "$rollback_test/out"
# Exercise the actual contamination matcher with enough trailing strings to
# expose an early-exit grep/SIGPIPE false negative under build-wallet's pipefail.
python3 - scripts/build-wallet.sh "$rollback_test" <<'PY'
from pathlib import Path
import re, sys
source = Path(sys.argv[1]).read_text()
root = Path(sys.argv[2])
matcher = re.search(r'^\s*\|\| (strings target/release/aether .*); then$', source, re.M)
assert matcher is not None, 'release contamination matcher is missing'
expression = matcher.group(1).replace('target/release/aether', '"$1"')
(root / 'contamination-check.sh').write_text('#!/bin/bash\nset -euo pipefail\n' + expression + '\n')
(root / 'contaminated-code').write_bytes(b'AETHER_TEST_INTERNAL_KEY_DIR\n' + b'fixture-padding\n' * 100000)
(root / 'ordinary-code').write_text('ordinary release code\n')
PY
if /bin/bash "$rollback_test/contamination-check.sh" "$rollback_test/contaminated-code"; then
  pass "release contamination matcher detects fixture code without a pipefail/SIGPIPE false negative"
else
  fail "release contamination matcher missed fixture code under pipefail"
fi
if /bin/bash "$rollback_test/contamination-check.sh" "$rollback_test/ordinary-code"; then
  fail "release contamination matcher rejected ordinary code"
else
  pass "release contamination matcher accepts ordinary code"
fi
cp "${AETHER_TEST_PACKAGE_MAC:-scripts/package-mac.sh}" "$rollback_test/scripts/package-mac.sh"
# Intercept Finder without opening an installed app or changing the desktop.
# Only the command path changes; the JavaScript renderer still uses AppKit.
python3 - "$rollback_test/scripts/package-mac.sh" <<'PY'
from pathlib import Path
import sys
path = Path(sys.argv[1])
path.write_text(path.read_text().replace('/usr/bin/osascript', 'osascript'))
PY
if [ -f scripts/package-rollback.sh ]; then cp scripts/package-rollback.sh "$rollback_test/scripts/"; fi
python3 - "$rollback_test" <<'PY'
import pathlib, plistlib, sys
root = pathlib.Path(sys.argv[1])
for name, old in (("current.app", False), ("previous.app", True)):
    app = root / name
    helpers = app / "Contents/Helpers"
    helpers.mkdir(parents=True)
    (app / "fixture-team").write_text("45WU468FZE\n")
    (app / "fixture-anchor").write_text("apple\n")
    if old:
        (app / "fixture-previous").touch()
    with (app / "Contents/Info.plist").open("wb") as f:
        plistlib.dump({"CFBundleIdentifier": "com.pipln.eastsea", "CFBundleVersion": "13", "LSMinimumSystemVersion": "14.0"}, f)
    node = helpers / "aether"
    def trust_guard(companion):
        return '''set -eu
if ! grep -q '^verified-app:' "$R12_TRUST_LOG" ||
   ! grep -Fqx "verified:$0" "$R12_TRUST_LOG" ||
   ! grep -Fqx "verified:$(dirname "$0")/''' + companion + '''" "$R12_TRUST_LOG"; then
    echo "fixture refuses previous helper execution before anchored verification" >&2
    exit 1
fi
printf 'executed:%s\\n' "$0" >> "$R12_TRUST_LOG"
'''
    metadata = '# fixture-archs arm64\n# fixture-minos 14.0\n# fixture-loadcmd LC_BUILD_VERSION\n# fixture-platform 1\n'
    node.write_text('#!/bin/bash\n# fixture-team 45WU468FZE\n# fixture-anchor apple\n# ' + ("previous node" if old else "new node") + '\n' + metadata + (trust_guard('aether-prover') if old else '') + '[ "$1" = protocol ] && echo 3\n')
    node.chmod(0o755)
    prover = helpers / "aether-prover"
    prover.write_text('#!/bin/bash\n# fixture-team 45WU468FZE\n# fixture-anchor apple\n# ' + ("previous prover" if old else "new prover") + '\n' + metadata + (trust_guard('aether.prev') if old else '') + 'echo old-compatible-program\n')
    prover.chmod(0o755)
    resources = app / "Contents/Resources"
    resources.mkdir()
    (resources / "network.json").write_text('{"chain_id":7780}\n')
    stub = resources / "eastsea-node-daemon.sh"
    stub.write_text("#!/bin/bash\nexit 0\n")
    stub.chmod(0o755)
    daemon = app / "Contents/Library/LaunchDaemons"
    daemon.mkdir(parents=True)
    with (daemon / "com.pipln.eastsea.node.plist").open("wb") as f:
        plistlib.dump({"BundleProgram": "Contents/Resources/eastsea-node-daemon.sh"}, f)
PY
cat > "$rollback_test/scripts/build-wallet.sh" <<'SH'
#!/bin/bash
set -eu
[ "${WALLET_CLEAN_BUILD:-0}" = 1 ] || { echo "fixture expected a clean build" >&2; exit 1; }
build=apps/wallet/build/Build/Products/Release
mkdir -p "$build"
rm -rf "$build/EastSea.app"
cp -R current.app "$build/EastSea.app"
SH
cat > "$rollback_test/scripts/prover-gate.sh" <<'SH'
#!/bin/bash
set -eu
if [ "${1:-}" = --prover ]; then
  [ -x "$2" ] && [ "$3" = --node ] && [ -x "$4" ] && [ "$5" = --network ] && [ -f "$6" ]
  "$2" info | grep -q '^old-compatible-program$' || { echo "prover gate: previous prover is incompatible" >&2; exit 1; }
  printf 'rollback prover checked\n' >> "$R12_GATE_LOG"
else
  [ -x "$1/Contents/Helpers/aether-prover" ]
fi
SH
cat > "$rollback_test/bin/codesign" <<'SH'
#!/bin/bash
set -eu
target=${!#}
required='anchor apple generic and certificate leaf[subject.OU] = "45WU468FZE" and certificate leaf[field.1.2.840.113635.100.6.1.13] exists'
requirement=""
for argument in "$@"; do
  case "$argument" in -R=*) requirement=${argument#-R=} ;; esac
done
case "$1" in
  --verify)
    [ ! -e "$target.bad-signature" ] && [ ! -e "$target/bad-signature" ] || exit 1
    previous=0
    if [ -d "$target" ] && [ -f "$target/fixture-previous" ]; then
      previous=1
      team=$(cat "$target/fixture-team")
      anchor=$(cat "$target/fixture-anchor")
    elif [ -f "$target" ] && grep -q '^# previous ' "$target"; then
      previous=1
      team=$(sed -n 's/^# fixture-team //p' "$target")
      anchor=$(sed -n 's/^# fixture-anchor //p' "$target")
    fi
    if [ "$previous" = 1 ]; then
      [ "$requirement" = "$required" ] || { echo "fixture refuses previous code without expected Apple anchor/OU/Developer ID requirement" >&2; exit 1; }
      [ "$anchor" = apple ] || { echo "fixture refuses previous code with an untrusted anchor" >&2; exit 1; }
      [ "$team" = 45WU468FZE ] || { echo "fixture refuses previous code with the wrong signing team" >&2; exit 1; }
      if [ -d "$target" ]; then
        printf 'verified-app:%s\n' "$target" >> "$R12_TRUST_LOG"
      else
        printf 'verified:%s\n' "$target" >> "$R12_TRUST_LOG"
      fi
    fi ;;
  -dv)
    if [ -d "$target" ]; then
      team=$(cat "$target/fixture-team")
    else
      team=$(sed -n 's/^# fixture-team //p' "$target")
    fi
    printf 'TeamIdentifier=%s\nAuthority=Fixture signing identity\n' "$team" >&2 ;;
  --force) : ;;
  *) echo "unexpected fixture codesign arguments" >&2; exit 1 ;;
esac
SH
cat > "$rollback_test/bin/otool" <<'SH'
#!/bin/bash
set -eu
case "$1" in
  -L)
    printf '%s:\n' "$2"
    if [ -f "$2.deps" ]; then
      cat "$2.deps"
    else
      printf '\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0, current version 1.0.0)\n'
    fi ;;
  -arch)
    [ "$3" = -l ] && [ "$#" -eq 4 ]
    file=$4
    command=$(sed -n 's/^# fixture-loadcmd //p' "$file")
    minimum=$(sed -n 's/^# fixture-minos //p' "$file")
    platform=$(sed -n 's/^# fixture-platform //p' "$file")
    printf '%s (architecture %s):\nLoad command 0\n      cmd %s\n  cmdsize 32\n' "$file" "$2" "$command"
    case "$command" in
      LC_BUILD_VERSION)
        printf ' platform %s\n' "$platform"
        if [ "$minimum" != missing ]; then printf '    minos %s\n' "$minimum"; fi ;;
      LC_VERSION_MIN_MACOSX)
        if [ "$minimum" != missing ]; then printf '  version %s\n' "$minimum"; fi ;;
    esac
    printf '      sdk 15.0\n' ;;
  *) echo "unexpected fixture otool arguments" >&2; exit 1 ;;
esac
SH
cat > "$rollback_test/bin/lipo" <<'SH'
#!/bin/bash
set -eu
[ "$1" = -archs ] && [ "$#" -eq 2 ]
architectures=$(sed -n 's/^# fixture-archs //p' "$2")
if [ "$architectures" != missing ]; then printf '%s\n' "$architectures"; fi
SH
cat > "$rollback_test/bin/hdiutil" <<'SH'
#!/bin/bash
set -eu
command=$1; shift
dmg=${!#}
source="" mount="" format="" output="" readonly=0 readwrite=0
while [ $# -gt 0 ]; do
  case "$1" in
    -srcfolder) source=$2; shift 2 ;;
    -mountpoint) mount=$2; shift 2 ;;
    -format) format=$2; shift 2 ;;
    -o) output=$2; shift 2 ;;
    -readonly) readonly=1; shift ;;
    -readwrite) readwrite=1; shift ;;
    *) shift ;;
  esac
done
case "$command" in
  create)
    [ -d "$source/EastSea.app" ]
    case "$format" in UDRW|UDZO) : ;; *) exit 1 ;; esac
    cp -R "$source" "$dmg.contents"
    printf '%s\n' "$format" > "$dmg.format"
    printf 'fixture DMG\n' > "$dmg"
    printf 'create:%s\n' "$format" >> "$R12_DMG_LOG"
    # Keep the previous one-step packager usable as the regression control.
    if [ "$format" = UDZO ]; then cp -R "$dmg.contents" "$R12_OUTPUT"; fi ;;
  attach)
    [ -d "$mount" ] && [ -f "$dmg" ] && [ "$((readonly + readwrite))" = 1 ] || exit 1
    case "$dmg" in
      */EastSea-previous.dmg)
        [ "$readonly" = 1 ] || { echo "fixture refuses a writable previous-release mount" >&2; exit 1; }
        cp -R "$R12_PREVIOUS_APP" "$mount/EastSea.app"
        printf 'previous-readonly-mount\n' >> "$R12_EVENTS" ;;
      *)
        if [ "$readwrite" = 1 ]; then
          [ "$(cat "$dmg.format")" = UDRW ]
          [ ! -e "$dmg.mount" ]
          printf '%s\n' "$mount" > "$dmg.mount"
        fi
        cp -R "$dmg.contents/." "$mount/" ;;
    esac
    if [ "$readwrite" = 1 ]; then mode="readwrite"; else mode="readonly"; fi
    printf '%s\n%s\n' "$mode" "$dmg" > "$mount.fixture-image"
    printf 'attach:%s\n' "$mode" >> "$R12_DMG_LOG" ;;
  detach)
    mount=$dmg
    mode=$(sed -n '1p' "$mount.fixture-image")
    image=$(sed -n '2p' "$mount.fixture-image")
    if [ "$mode" = readwrite ]; then
      rm -rf "$image.contents"
      cp -R "$mount" "$image.contents"
      cp -R "$mount" "$R12_OUTPUT.mounted"
      rm "$image.mount"
    fi
    case "$image" in */EastSea-previous.dmg) printf 'previous-detach\n' >> "$R12_EVENTS" ;; esac
    printf 'detach:%s\n' "$mode" >> "$R12_DMG_LOG"
    rm -rf "${mount:?}" "$mount.fixture-image"
    mkdir "$mount" ;;
  convert)
    [ "$format" = UDZO ] && [ "$(cat "$dmg.format")" = UDRW ] && [ ! -e "$dmg.mount" ] || exit 1
    [ -n "$output" ] && [ -d "$dmg.contents" ] || exit 1
    cp -R "$dmg.contents" "$output.contents"
    cp -R "$output.contents" "$R12_OUTPUT"
    printf '%s\n' "$format" > "$output.format"
    printf 'fixture compressed DMG\n' > "$output"
    printf 'convert:%s\n' "$format" >> "$R12_DMG_LOG" ;;
  *) echo "unexpected fixture hdiutil command: $command" >&2; exit 1 ;;
esac
SH
cat > "$rollback_test/bin/getconf" <<'SH'
#!/bin/bash
set -eu
[ "$#" -eq 1 ] && [ "$1" = DARWIN_USER_TEMP_DIR ] || exit 1
# The doubles need no real mount, so every temporary artifact stays in tmp/.
printf '%s/\n' "$TMPDIR"
SH
cat > "$rollback_test/bin/osascript" <<'SH'
#!/bin/bash
set -eu
if [ "$1" = -l ]; then
  [ "$2" = JavaScript ]
  exec /usr/bin/osascript "$@"
fi
[ "$#" -eq 2 ] && [ "$1" = - ] && [ -d "$2/EastSea.app" ] || exit 1
[ "$(sed -n '1p' "$2.fixture-image")" = readwrite ]
image=$(sed -n '2p' "$2.fixture-image")
[ "$(cat "$image.mount")" = "$2" ]
# Synthetic Finder metadata records the requested layout, then travels through
# the same mount/detach/convert copies as the actual image and Applications link.
cat > "$2/.DS_Store"
printf 'finder-layout\n' >> "$R12_DMG_LOG"
SH
chmod +x "$rollback_test/scripts/"*.sh "$rollback_test/bin/"*
package_fixture() {
  # A distinct captured output for each invocation; the old app is read only.
  local name=$1 previous=$2
  (cd "$rollback_test" && PATH="$rollback_test/bin:$PATH" TMPDIR="$rollback_test/tmp" \
    AETHER_VERSION="$name" SIGN_IDENTITY="Fixture signing identity" NOTARY_PROFILE='' APP_STORE_CONNECT_API_KEY_ID='' \
    AETHER_PREVIOUS_APP="$previous" R12_OUTPUT="$rollback_test/out/$name" \
    R12_GATE_LOG="$rollback_test/gates.log" R12_TRUST_LOG="$rollback_test/$name.trust.log" \
    R12_DMG_LOG="$rollback_test/$name.dmg.log" \
    /bin/bash scripts/package-mac.sh) \
    > "$rollback_test/$name.log" 2>&1
}
previous="$rollback_test/previous.app"
if package_fixture clean "$previous"; then
  rollback="$rollback_test/out/clean/EastSea.app/Contents/Helpers/NodeRollback.bundle/Contents/MacOS"
  if [ -x "$rollback/aether.prev" ]; then
    pass "R12 clean package includes executable NodeRollback.bundle/Contents/MacOS/aether.prev"
    if cmp -s "$previous/Contents/Helpers/aether" "$rollback/aether.prev"; then
      pass "R12 rollback retains the verified previous node bytes"
    else
      fail "R12 rollback node is not the previous helper"
    fi
    if cmp -s "$previous/Contents/Helpers/aether-prover" "$rollback/aether-prover"; then
      pass "R12 rollback retains its adjacent previous prover"
    else
      fail "R12 rollback does not retain the previous prover"
    fi
    if [ -f "$rollback_test/gates.log" ] && grep -q 'rollback prover checked' "$rollback_test/gates.log"; then
      pass "R12 previous prover is checked against the new bundle network"
    else
      fail "R12 previous prover was not compatibility checked"
    fi
    if python3 - "$rollback_test/clean.trust.log" <<'PY'
from pathlib import Path
import sys
events = Path(sys.argv[1]).read_text().splitlines()
executions = [(i, event.removeprefix('executed:')) for i, event in enumerate(events) if event.startswith('executed:')]
assert len(executions) == 2, events
app_verification = next(i for i, event in enumerate(events) if event.startswith('verified-app:'))
for i, executable in executions:
    assert app_verification < i, events
    assert events.index('verified:' + executable) < i, events
    companion = 'aether-prover' if Path(executable).name == 'aether.prev' else 'aether.prev'
    assert events.index('verified:' + str(Path(executable).with_name(companion))) < i, events
PY
    then
      pass "R12 app and both copied helpers pass Apple Developer ID verification before either CLI executes"
    else
      fail "R12 previous code execution did not follow anchored app/helper verification"
    fi
  else
    fail "R12 clean package is missing executable NodeRollback.bundle/Contents/MacOS/aether.prev"
  fi
else
  fail "R12 fixture packaging failed before the rollback assertion: $(cat "$rollback_test/clean.log")"
fi

# Each installer check also runs against the older packager selected above.
# Inspect the converted payload, not merely the source or pre-mount staging tree.
if python3 - "$rollback_test/clean.dmg.log" <<'PY'
from pathlib import Path
import sys
assert Path(sys.argv[1]).read_text().splitlines() == [
    'create:UDRW', 'attach:readwrite', 'finder-layout', 'detach:readwrite', 'convert:UDZO',
]
PY
then
  pass "DMG saves the Finder layout on a writable image before detaching and converting to UDZO"
else
  fail "DMG must create UDRW, attach writable, save Finder layout, detach and convert to UDZO"
fi
if python3 - "$rollback_test/out/clean" <<'PY'
from pathlib import Path
import re, subprocess, sys
payload = Path(sys.argv[1])
background = payload / '.background/installer.tiff'
assert background.is_file(), 'converted DMG is missing the installer background'
result = subprocess.run(['/usr/bin/tiffutil', '-info', str(background)], capture_output=True, text=True, check=True)
info = result.stdout + result.stderr
assert re.findall(r'Image Width: (\d+) Image Length: (\d+)', info) == [('640', '440'), ('1280', '880')], info
assert re.findall(r'Resolution: (\d+), (\d+)', info) == [('72', '72'), ('144', '144')], info
layout = re.sub(r'\(\*.*?\*\)|--[^\n]*', '', (payload / '.DS_Store').read_text(), flags=re.S)
assert 'set background picture of icon view options of installerWindow to (POSIX file (volumePath & "/.background/installer.tiff") as alias)' in {line.strip() for line in layout.splitlines()}
PY
then
  pass "DMG preserves the selected 640x440/1280x880 installer background at 72/144 dpi"
else
  fail "DMG must preserve and select the Retina installer background"
fi
if python3 - "$rollback_test/out/clean/.DS_Store" <<'PY'
from pathlib import Path
import re, sys
layout = re.sub(r'\(\*.*?\*\)|--[^\n]*', '', Path(sys.argv[1]).read_text(), flags=re.S)
settings = {line.strip() for line in layout.splitlines()}
for setting in (
    'set current view of installerWindow to icon view',
    'set bounds of installerWindow to {160, 120, 800, 588}',
    'set arrangement of icon view options of installerWindow to not arranged',
    'set icon size of icon view options of installerWindow to 96',
    'set text size of icon view options of installerWindow to 13',
    'set label position of icon view options of installerWindow to bottom',
    'set shows item info of icon view options of installerWindow to false',
    'set position of item "EastSea.app" of installerFolder to {168, 212}',
    'set position of item "Applications" of installerFolder to {472, 212}',
    'update installerFolder without registering applications',
    'close installerWindow',
):
    assert setting in settings, setting
for bar in ('toolbar', 'statusbar', 'pathbar'):
    assert f'set {bar} visible of installerWindow to false' in settings, bar
PY
then
  pass "DMG preserves the approved Finder window and EastSea (168,212)/Applications (472,212) icon positions"
else
  fail "DMG must preserve the approved Finder window and icon positions"
fi
if python3 - "$rollback_test/out/clean" <<'PY'
from pathlib import Path
import os, sys
for directory in (Path(sys.argv[1] + '.mounted'), Path(sys.argv[1])):
    link = directory / 'Applications'
    assert link.is_symlink() and os.readlink(link) == '/Applications', f'missing Applications shortcut in {directory}'
PY
then
  pass "DMG retains the /Applications shortcut through writable mounting and conversion"
else
  fail "DMG must retain the /Applications shortcut through writable mounting and conversion"
fi

# The packaging doubles cannot validate Apple's nested-code sealing rules.
# Check the same bundle layout with real, task-owned ad-hoc code as well.
native_app="$rollback_test/native.app"
native_rollback="$native_app/Contents/Helpers/NodeRollback.bundle"
mkdir -p "$native_app/Contents/MacOS" "$native_rollback/Contents/MacOS"
cp /usr/bin/true "$native_app/Contents/MacOS/EastSea"
cp /usr/bin/true "$native_rollback/Contents/MacOS/aether.prev"
cp /usr/bin/true "$native_rollback/Contents/MacOS/aether-prover"
python3 - "$native_app" "$native_rollback" <<'PY'
from pathlib import Path
import plistlib, sys
for directory, identifier, executable, kind in (
    (sys.argv[1], 'com.pipln.eastsea', 'EastSea', 'APPL'),
    (sys.argv[2], 'com.pipln.eastsea.node-rollback', 'aether.prev', 'BNDL'),
):
    with (Path(directory) / 'Contents/Info.plist').open('wb') as stream:
        plistlib.dump({'CFBundleIdentifier': identifier, 'CFBundleExecutable': executable,
                      'CFBundlePackageType': kind, 'CFBundleVersion': '1'}, stream)
PY
if /usr/bin/codesign --force --options runtime --timestamp=none --sign - "$native_rollback/Contents/MacOS/aether.prev" \
    && /usr/bin/codesign --force --options runtime --timestamp=none --sign - "$native_rollback/Contents/MacOS/aether-prover" \
    && /usr/bin/codesign --force --options runtime --timestamp=none --preserve-metadata=entitlements,flags --sign - "$native_rollback" \
    && /usr/bin/codesign --force --options runtime --timestamp=none --sign - "$native_app" \
    && /usr/bin/codesign --verify --deep --strict "$native_app"; then
  pass "R12 nested rollback bundle passes real macOS deep strict signature verification"
else
  fail "R12 nested rollback bundle fails real macOS deep strict signature verification"
fi
refuse_package() {
  local name=$1 previous_app=$2 reason=$3
  if package_fixture "$name" "$previous_app"; then
    fail "R12 packaging accepted $name"
  elif grep -q "$reason" "$rollback_test/$name.log"; then
    pass "R12 packaging refuses $name"
  else
    fail "R12 packaging failed for the wrong reason ($name): $(cat "$rollback_test/$name.log")"
  fi
}
refuse_package missing-previous "$rollback_test/absent.app" 'previous app'
cp -R "$previous" "$rollback_test/unsigned.app"
touch "$rollback_test/unsigned.app/Contents/Helpers/aether.bad-signature"
refuse_package invalid-node-signature "$rollback_test/unsigned.app" 'signature'
cp -R "$previous" "$rollback_test/wrong-team.app"
sed -i '' 's/fixture-team 45WU468FZE/fixture-team ABCDE12345/' "$rollback_test/wrong-team.app/Contents/Helpers/aether"
refuse_package wrong-helper-team "$rollback_test/wrong-team.app" 'team'
cp -R "$previous" "$rollback_test/self-signed-app.app"
printf 'untrusted\n' > "$rollback_test/self-signed-app.app/fixture-anchor"
refuse_package self-signed-app-matching-team "$rollback_test/self-signed-app.app" 'untrusted anchor'
cp -R "$previous" "$rollback_test/self-signed-helper.app"
sed -i '' 's/fixture-anchor apple/fixture-anchor untrusted/' "$rollback_test/self-signed-helper.app/Contents/Helpers/aether"
refuse_package self-signed-helper-matching-team "$rollback_test/self-signed-helper.app" 'untrusted anchor'
cp -R "$previous" "$rollback_test/missing-prover.app"
rm "$rollback_test/missing-prover.app/Contents/Helpers/aether-prover"
refuse_package missing-previous-prover "$rollback_test/missing-prover.app" 'previous prover'
cp -R "$previous" "$rollback_test/private-library.app"
printf '\t@rpath/libUnavailable.dylib (compatibility version 1.0.0, current version 1.0.0)\n' \
  > "$rollback_test/private-library.app/Contents/Helpers/aether-prover.deps"
refuse_package unsupported-private-library "$rollback_test/private-library.app" 'unsupported dependency'
cp -R "$previous" "$rollback_test/invalid-protocol.app"
sed -i '' 's/echo 3/echo unknown/' "$rollback_test/invalid-protocol.app/Contents/Helpers/aether"
refuse_package invalid-previous-protocol "$rollback_test/invalid-protocol.app" 'protocol'
cp -R "$previous" "$rollback_test/incompatible-prover.app"
sed -i '' 's/echo old-compatible-program/echo incompatible-program/' "$rollback_test/incompatible-prover.app/Contents/Helpers/aether-prover"
refuse_package incompatible-previous-prover "$rollback_test/incompatible-prover.app" 'prover gate'

for component in aether aether-prover; do
  wrong_arch="$rollback_test/wrong-arch-$component.app"
  cp -R "$previous" "$wrong_arch"
  sed -i '' 's/fixture-archs arm64/fixture-archs x86_64/' "$wrong_arch/Contents/Helpers/$component"
  refuse_package "wrong-architecture-$component" "$wrong_arch" 'missing required architecture arm64'
  high_os="$rollback_test/high-minos-$component.app"
  cp -R "$previous" "$high_os"
  sed -i '' 's/fixture-minos 14.0/fixture-minos 15.0/' "$high_os/Contents/Helpers/$component"
  refuse_package "high-minimum-os-$component" "$high_os" 'minimum OS exceeds'
done
cp -R "$previous" "$rollback_test/missing-arch.app"
sed -i '' 's/fixture-archs arm64/fixture-archs missing/' "$rollback_test/missing-arch.app/Contents/Helpers/aether"
refuse_package missing-architecture-metadata "$rollback_test/missing-arch.app" 'architecture metadata'
cp -R "$previous" "$rollback_test/malformed-arch.app"
sed -i '' 's/fixture-archs arm64/fixture-archs arm64 broken!/' "$rollback_test/malformed-arch.app/Contents/Helpers/aether-prover"
refuse_package malformed-architecture-metadata "$rollback_test/malformed-arch.app" 'architecture metadata'
cp -R "$previous" "$rollback_test/missing-minos.app"
sed -i '' 's/fixture-minos 14.0/fixture-minos missing/' "$rollback_test/missing-minos.app/Contents/Helpers/aether"
refuse_package missing-minimum-os-metadata "$rollback_test/missing-minos.app" 'minimum OS metadata'
cp -R "$previous" "$rollback_test/malformed-minos.app"
sed -i '' 's/fixture-minos 14.0/fixture-minos unknown/' "$rollback_test/malformed-minos.app/Contents/Helpers/aether-prover"
refuse_package malformed-minimum-os-metadata "$rollback_test/malformed-minos.app" 'minimum OS metadata'
cp -R "$previous" "$rollback_test/non-macos.app"
sed -i '' 's/fixture-platform 1/fixture-platform 2/' "$rollback_test/non-macos.app/Contents/Helpers/aether-prover"
refuse_package non-macos-helper "$rollback_test/non-macos.app" 'non-macOS platform'

# Universal replacements require every supported architecture, regardless of
# what architecture the packaging machine would choose to execute locally.
current_node="$rollback_test/current.app/Contents/Helpers/aether"
cp "$current_node" "$rollback_test/current-node.baseline"
sed -i '' 's/fixture-archs arm64/fixture-archs arm64 x86_64/' "$current_node"
refuse_package partial-universal-coverage "$previous" 'missing required architecture x86_64'
cp "$rollback_test/current-node.baseline" "$current_node"

# Older Mach-O LC_VERSION_MIN_MACOSX metadata remains supported.
cp -R "$previous" "$rollback_test/legacy-minos.app"
for component in aether aether-prover; do
  sed -i '' 's/fixture-loadcmd LC_BUILD_VERSION/fixture-loadcmd LC_VERSION_MIN_MACOSX/' \
    "$rollback_test/legacy-minos.app/Contents/Helpers/$component"
done
if package_fixture legacy-minimum-os "$rollback_test/legacy-minos.app"; then
  pass "R12 accepts compatible LC_VERSION_MIN_MACOSX metadata"
else
  fail "R12 rejected compatible legacy minimum OS metadata: $(cat "$rollback_test/legacy-minimum-os.log")"
fi

# Missing app-level LSMinimumSystemVersion falls back to the replacement node,
# while missing/malformed binary or explicitly supplied app metadata fails.
current_info="$rollback_test/current.app/Contents/Info.plist"
cp "$current_info" "$rollback_test/current-info.baseline"
python3 - "$current_info" <<'PY'
import plistlib, sys
with open(sys.argv[1], 'rb') as stream:
    info = plistlib.load(stream)
info.pop('LSMinimumSystemVersion')
with open(sys.argv[1], 'wb') as stream:
    plistlib.dump(info, stream)
PY
if package_fixture node-minimum-os-fallback "$previous"; then
  pass "R12 falls back to replacement node minimum OS when the app minimum is absent"
else
  fail "R12 failed replacement minimum OS fallback: $(cat "$rollback_test/node-minimum-os-fallback.log")"
fi
cp "$rollback_test/current-info.baseline" "$current_info"
sed -i '' 's/fixture-minos 14.0/fixture-minos missing/' "$current_node"
refuse_package missing-replacement-minimum-os "$previous" 'minimum OS metadata'
cp "$rollback_test/current-node.baseline" "$current_node"
/usr/libexec/PlistBuddy -c 'Set :LSMinimumSystemVersion unknown' "$current_info"
refuse_package malformed-app-minimum-os "$previous" 'minimum OS metadata'
cp "$rollback_test/current-info.baseline" "$current_info"

# Exercise release preparation as well: external release/credential/signing
# surfaces are replaced, and the real release script still drives packaging.
cp scripts/release-mac.sh "$rollback_test/scripts/release-mac.sh"
python3 - "$rollback_test/scripts/release-mac.sh" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
source = p.read_text()
needle = 'source ~/.config/app-store-release/env.sh'
assert source.count(needle) == 1
p.write_text(source.replace(needle, ': # fixture credential setup'))
PY
printf 'MARKETING_VERSION: 0.7.2\nCURRENT_PROJECT_VERSION: 15\n' > "$rollback_test/apps/wallet/project.yml"
mkdir -p "$rollback_test/apps/extension" "$rollback_test/apps/wallet/build/SourcePackages/artifacts/sparkle/Sparkle/bin"
printf '{"version":"1"}\n' > "$rollback_test/apps/extension/manifest.json"
cat > "$rollback_test/scripts/release-identity-gate.sh" <<'SH'
#!/bin/bash
exit 0
SH
cat > "$rollback_test/scripts/release-approve.py" <<'SH'
#!/bin/bash
printf 'release-prepare\n' >> "$R12_EVENTS"
SH
cat > "$rollback_test/scripts/build-extension.sh" <<'SH'
#!/bin/bash
mkdir -p dist
printf 'fixture extension\n' > dist/eastsea-extension-1.zip
SH
cat > "$rollback_test/apps/wallet/build/SourcePackages/artifacts/sparkle/Sparkle/bin/sign_update" <<'SH'
#!/bin/bash
printf 'sparkle:edSignature="fixture" length="11"\n'
SH
cat > "$rollback_test/bin/git" <<'SH'
#!/bin/bash
set -eu
if [ "${1:-}" = -C ]; then shift 2; fi
case "$1" in
  status) : ;;
  rev-parse) echo fixture-commit ;;
  show) echo 123456 ;;
  *) echo "unexpected fixture git command" >&2; exit 1 ;;
esac
SH
cat > "$rollback_test/bin/gh" <<'SH'
#!/bin/bash
set -eu
case "$1/$2" in
  release/list) echo app-v0.7.1 ;;
  release/download)
    directory=""
    while [ $# -gt 0 ]; do
      case "$1" in --dir) directory=$2; shift 2 ;; *) shift ;; esac
    done
    [ -n "$directory" ]
    printf 'fixture previous DMG\n' > "$directory/EastSea-previous.dmg"
    printf 'previous-download\n' >> "$R12_EVENTS" ;;
  *) echo "unexpected fixture gh command" >&2; exit 1 ;;
esac
SH
# The fixture's build command records when the clean replacement starts.
python3 - "$rollback_test/scripts/build-wallet.sh" <<'PY'
from pathlib import Path
import sys
p = Path(sys.argv[1])
p.write_text(p.read_text().replace('set -eu\n', 'set -eu\nprintf \'clean-build\\n\' >> "$R12_EVENTS"\n'))
PY
chmod +x "$rollback_test/scripts/"* "$rollback_test/bin/"* \
  "$rollback_test/apps/wallet/build/SourcePackages/artifacts/sparkle/Sparkle/bin/sign_update"
if (cd "$rollback_test" && PATH="$rollback_test/bin:$PATH" TMPDIR="$rollback_test/tmp" AETHER_PREVIOUS_APP='' PREV_RELEASE_TAG='' \
  NOTARY_PROFILE='' APP_STORE_CONNECT_API_KEY_ID='' \
  AETHER_RELEASE_LOG="$rollback_test/chain-release.json" AETHER_RELEASE_CHAIN_ID=7780 \
  R12_OUTPUT="$rollback_test/out/release" R12_GATE_LOG="$rollback_test/release-gates.log" \
  R12_TRUST_LOG="$rollback_test/release.trust.log" \
  R12_DMG_LOG="$rollback_test/release.dmg.log" \
  R12_EVENTS="$rollback_test/events.log" R12_PREVIOUS_APP="$previous" \
  /bin/bash scripts/release-mac.sh --prepare) > "$rollback_test/release.log" 2>&1; then
  if python3 - "$rollback_test/events.log" <<'PY'
import pathlib, sys
events = pathlib.Path(sys.argv[1]).read_text().splitlines()
try:
    assert events.index('previous-download') < events.index('previous-readonly-mount') < events.index('clean-build')
    assert events.index('clean-build') < events.index('previous-detach') < events.index('release-prepare')
except (ValueError, AssertionError):
    print('release events:', events, file=sys.stderr)
    sys.exit(1)
PY
  then
    pass "R12 release prepares the previous app read only before its clean build and detaches before release preparation"
  else
    fail "R12 release did not prepare a previous app before its clean build"
  fi
else
  fail "R12 release fixture failed: $(cat "$rollback_test/release.log")"
fi

printf 'release script checks: %s passed, %s failed\n' "$good" "$bad"
exit "$bad"
