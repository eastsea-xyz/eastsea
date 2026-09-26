#!/usr/bin/env bash
# Package the macOS app as a drag-to-install DMG (Aether.app + an Applications shortcut).
#
#   scripts/package-mac.sh
#       Local build, signed with the development identity from project.yml.
#   SIGN_IDENTITY="Developer ID Application: <Team> (<TEAMID>)" scripts/package-mac.sh
#       Release: re-sign the helpers, app and DMG with Developer ID (hardened runtime,
#       secure timestamp). Notarization then needs either
#         NOTARY_PROFILE=<name>                 (xcrun notarytool store-credentials), or
#         APP_STORE_CONNECT_API_KEY_ID / _ISSUER_ID / _PATH  (an App Store Connect API key;
#                                                `source ~/.config/app-store-release/env.sh`)
#       and the ticket is stapled to the DMG.
# Output: dist/Aether-<version>.dmg
set -euo pipefail
cd "$(dirname "$0")/.."
version=${AETHER_VERSION:-$(git describe --tags --always --dirty)}
scripts/build-wallet.sh macos >/dev/null
src="apps/wallet/build/Build/Products/Release/Aether.app"
[ -d "$src" ] || { echo "build failed: $src missing"; exit 1; }

stage=$(mktemp -d)
trap 'rm -rf "${stage:?}"' EXIT
app="$stage/Aether.app"
cp -R "$src" "$app"

if [ -n "${SIGN_IDENTITY:-}" ]; then
  # Inside out: helpers and Sparkle's nested code first, then the app bundle.
  for h in "$app/Contents/Helpers/"*; do
    codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$h"
  done
  sp="$app/Contents/Frameworks/Sparkle.framework"
  if [ -d "$sp" ]; then
    for x in "$sp"/Versions/B/XPCServices/*.xpc "$sp/Versions/B/Autoupdate" "$sp/Versions/B/Updater.app" "$sp"; do
      [ -e "$x" ] && codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$x"
    done
  fi
  codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$app"
fi
codesign --verify --deep --strict "$app"

ln -s /Applications "$stage/Applications"
mkdir -p dist
dmg="dist/Aether-$version.dmg"
rm -f "$dmg"
hdiutil create -quiet -volname "Aether" -srcfolder "$stage" -fs HFS+ -format UDZO "$dmg"

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
