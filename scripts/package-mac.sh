#!/usr/bin/env bash
# Package the macOS app as a drag-to-install DMG (EastSea.app + an Applications shortcut).
#
#   scripts/package-mac.sh
#       AETHER_PREVIOUS_APP=/path/to/previous/EastSea.app is required.
#       Local build, re-sealed with the build signing identity (or ad hoc).
#   SIGN_IDENTITY="Developer ID Application: <Team> (<TEAMID>)" scripts/package-mac.sh
#       Release: re-sign the helpers, app and DMG with Developer ID (hardened runtime,
#       secure timestamp). Notarization then needs either
#         NOTARY_PROFILE=<name>                 (xcrun notarytool store-credentials), or
#         APP_STORE_CONNECT_API_KEY_ID / _ISSUER_ID / _PATH  (an App Store Connect API key;
#                                                `source ~/.config/app-store-release/env.sh`)
#       and the ticket is stapled to the DMG.
# Output: dist/EastSea-<version>.dmg
set -euo pipefail
cd "$(dirname "$0")/.."
version=${AETHER_VERSION:-$(git describe --tags --always --dirty)}
mkdir -p "$PWD/tmp"
export TMPDIR="$PWD/tmp"
: "${AETHER_PREVIOUS_APP:?set AETHER_PREVIOUS_APP to a signed previous EastSea.app for rollback}"
# A release always builds from a clean derived-data directory (no stale
# Helpers/aether.prev or other leftovers from an earlier build in apps/wallet/build).
WALLET_CLEAN_BUILD=1 scripts/build-wallet.sh macos >/dev/null
src="apps/wallet/build/Build/Products/Release/EastSea.app"
[ -d "$src" ] || { echo "build failed: $src missing"; exit 1; }
# Release gate: the app must prove with the program the bundled network's
# validators verify, or none of its proofs ever verify (scripts/prover-gate.sh;
# AETHER_PROVER_GATE_OVERRIDE for a coordinated release, recorded).
scripts/prover-gate.sh "$src"

stage=$(mktemp -d "$TMPDIR/package-mac.XXXXXX")
trap 'rm -rf "${stage:?}"' EXIT
app="$stage/EastSea.app"
cp -R "$src" "$app"
scripts/package-rollback.sh "$AETHER_PREVIOUS_APP" "$app"
# Adding rollback code changes the resource seal, so local packages also
# need a new app signature. Reuse the build identity (ad hoc if none).
identity=${SIGN_IDENTITY:-}
timestamp=--timestamp
if [ -z "$identity" ]; then
  identity=$(codesign -dv --verbose=4 "$src" 2>&1 | sed -n 's/^Authority=//p' | head -1)
  identity=${identity:--}
  timestamp=--timestamp=none
fi

# Inside out: current helpers, the isolated rollback code bundle, Sparkle,
# then the app. A code bundle gives the old node an adjacent matching prover
# without mixing its runtime dependencies with the replacement helpers.
for h in "$app/Contents/Helpers/"*; do
  [ -f "$h" ] && codesign --force --options runtime "$timestamp" --sign "$identity" "$h"
done
codesign --force --options runtime "$timestamp" --preserve-metadata=entitlements,flags \
  --sign "$identity" "$app/Contents/Helpers/NodeRollback.bundle"
sp="$app/Contents/Frameworks/Sparkle.framework"
if [ -d "$sp" ]; then
  for x in "$sp"/Versions/B/XPCServices/*.xpc "$sp/Versions/B/Autoupdate" "$sp/Versions/B/Updater.app" "$sp"; do
    [ -e "$x" ] && codesign --force --options runtime "$timestamp" --sign "$identity" "$x"
  done
fi
codesign --force --options runtime "$timestamp" --sign "$identity" "$app"
codesign --verify --deep --strict "$app"
# The unattended daemon's BundleProgram must exist in the bundle we ship.
prog=$(/usr/libexec/PlistBuddy -c 'Print :BundleProgram' "$app/Contents/Library/LaunchDaemons/com.pipln.eastsea.node.plist")
[ -x "$app/$prog" ] || { echo "daemon BundleProgram $prog missing from $app"; exit 1; }

ln -s /Applications "$stage/Applications"
mkdir -p dist
dmg="dist/EastSea-$version.dmg"
rm -f "$dmg"
hdiutil create -quiet -volname "EastSea" -srcfolder "$stage" -fs HFS+ -format UDZO "$dmg"

if [ -n "${SIGN_IDENTITY:-}" ]; then
  codesign --force --timestamp --sign "$SIGN_IDENTITY" "$dmg"
  if [ -n "${NOTARY_PROFILE:-}" ]; then
    xcrun notarytool submit "$dmg" --keychain-profile "$NOTARY_PROFILE" --wait
  elif [ -n "${APP_STORE_CONNECT_API_KEY_ID:-}" ]; then
    xcrun notarytool submit "$dmg" --key "$APP_STORE_CONNECT_API_KEY_PATH" --key-id "$APP_STORE_CONNECT_API_KEY_ID" \
      --issuer "$APP_STORE_CONNECT_API_KEY_ISSUER_ID" --wait
  else
    echo "not notarized (set NOTARY_PROFILE or APP_STORE_CONNECT_API_KEY_*)"
  fi
  if [ -n "${NOTARY_PROFILE:-}${APP_STORE_CONNECT_API_KEY_ID:-}" ]; then
    xcrun stapler staple "$dmg"
    spctl --assess --type open --context context:primary-signature --verbose "$dmg"
  fi
fi
echo "$dmg ($(du -h "$dmg" | cut -f1))"
