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
# Requires a logged-in macOS desktop: Finder saves the installer window layout.
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
mountpoint=
cleanup() {
  local status=$?
  if [ -n "$mountpoint" ]; then
    hdiutil detach -quiet "$mountpoint" 2>/dev/null \
      || hdiutil detach -quiet -force "$mountpoint" 2>/dev/null || true
    rmdir "$mountpoint" 2>/dev/null || true
  fi
  rm -rf "${stage:?}"
  return "$status"
}
trap cleanup EXIT
payload="$stage/volume"
mkdir -p "$payload"
app="$payload/EastSea.app"
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

ln -s /Applications "$payload/Applications"
# The chart room at dawn: design/brand/tokens.json and wallet-redesign/spec.md
# (b46d281). Use native AppKit drawing, so packaging needs no extra dependencies.
# A multi-resolution TIFF gives Finder a 640 x 440 pt canvas at 1x and Retina 2x.
mkdir -p "$payload/.background"
/usr/bin/osascript -l JavaScript - "$stage" <<'JXA'
ObjC.import('AppKit');
function run(argv) {
    var width = 640, height = 440;
    function color(hex, alpha) {
        return $.NSColor.colorWithSRGBRedGreenBlueAlpha(
            parseInt(hex.slice(0, 2), 16) / 255,
            parseInt(hex.slice(2, 4), 16) / 255,
            parseInt(hex.slice(4, 6), 16) / 255, alpha === undefined ? 1 : alpha);
    }
    var paper = color('F4EFE6'), navy = color('0D2135');
    var muted = color('4B5B6B'), line = color('D8CEBB'), gold = color('E8BF59');
    function rect(x, y, w, h) { return $.NSMakeRect(x, height - y - h, w, h); }
    function point(x, y) { return $.NSMakePoint(x, height - y); }
    function text(value, x, y, w, h, font, ink) {
        var attributes = $.NSMutableDictionary.alloc.init;
        attributes.setObjectForKey(font, $.NSFontAttributeName);
        attributes.setObjectForKey(ink, $.NSForegroundColorAttributeName);
        $(value).drawInRectWithAttributes(rect(x, y, w, h), attributes);
    }
    [1, 2].forEach(function(scale) {
        var bitmap = $.NSBitmapImageRep.alloc
            .initWithBitmapDataPlanesPixelsWidePixelsHighBitsPerSampleSamplesPerPixelHasAlphaIsPlanarColorSpaceNameBytesPerRowBitsPerPixel(
                null, width * scale, height * scale, 8, 4, true, false, $.NSDeviceRGBColorSpace, 0, 0);
        bitmap.size = $.NSMakeSize(width, height);
        var context = $.NSGraphicsContext.graphicsContextWithBitmapImageRep(bitmap);
        $.NSGraphicsContext.saveGraphicsState;
        $.NSGraphicsContext.setCurrentContext(context);
        paper.setFill;
        $.NSBezierPath.fillRect(rect(0, 0, width, height));
        navy.setFill;
        $.NSBezierPath.fillRect(rect(0, 0, width, 124));
        gold.setFill;
        $.NSBezierPath.bezierPathWithOvalInRect(rect(528, 36, 48, 48)).fill;
        color('E8BF59', 0.20).setStroke;
        for (var y = 76; y <= 116; y += 8) {
            var wave = $.NSBezierPath.bezierPath;
            wave.moveToPoint(point(408, y));
            wave.curveToPointControlPoint1ControlPoint2(point(640, y), point(484, y - 16), point(568, y + 16));
            wave.lineWidth = 1;
            wave.stroke;
        }
        text('EastSea', 48, 28, 340, 42, $.NSFont.systemFontOfSizeWeight(30, $.NSFontWeightSemibold), paper);
        text('Drag EastSea to Applications to install.', 48, 80, 464, 24,
            $.NSFont.systemFontOfSize(15), paper);
        navy.setStroke;
        var arrow = $.NSBezierPath.bezierPath;
        arrow.moveToPoint(point(288, 212));
        arrow.lineToPoint(point(352, 212));
        arrow.moveToPoint(point(340, 200));
        arrow.lineToPoint(point(352, 212));
        arrow.lineToPoint(point(340, 224));
        arrow.lineWidth = 3;
        arrow.stroke;
        line.setStroke;
        var rule = $.NSBezierPath.bezierPath;
        rule.moveToPoint(point(48, 304));
        rule.lineToPoint(point(592, 304));
        rule.lineWidth = 1;
        rule.stroke;
        // The image is shared by all locales; every install instruction is visible.
        var translations = [
            'EastSea를 응용 프로그램 폴더로 드래그해 설치하세요.',
            'EastSea をアプリケーションフォルダにドラッグしてインストール。',
            '将 EastSea 拖入「应用程序」文件夹以安装。',
            '將 EastSea 拖入「應用程式」檔案夾以安裝。'
        ];
        translations.forEach(function(value, index) {
            text(value, 48, 324 + index * 22, 544, 22, $.NSFont.systemFontOfSize(13), muted);
        });
        $.NSGraphicsContext.restoreGraphicsState;
        var png = bitmap.representationUsingTypeProperties($.NSBitmapImageFileTypePNG, $.NSDictionary.dictionary);
        var name = scale === 2 ? '/installer@2x.png' : '/installer.png';
        if (!png.writeToFileAtomically($(argv[0] + name), true)) throw Error('Cannot write installer background');
    });
}
JXA
/usr/bin/tiffutil -cathidpicheck "$stage/installer.png" "$stage/installer@2x.png" \
  -out "$payload/.background/installer.tiff"

mkdir -p dist
dmg="dist/EastSea-$version.dmg"
rm -f "$dmg"
hdiutil create -quiet -volname "EastSea" -srcfolder "$payload" -fs HFS+ -format UDRW "$stage/installer.dmg"
# hdiutil refuses mountpoints on the external /Volumes/workspace filesystem.
# Only the mountpoint lives in the OS temp directory; generated files use TMPDIR.
mountpoint=$(mktemp -d "$(getconf DARWIN_USER_TEMP_DIR)EastSea-dmg.XXXXXX")
hdiutil attach -quiet -readwrite -noautoopen -nobrowse -mountpoint "$mountpoint" "$stage/installer.dmg"
/usr/bin/osascript - "$mountpoint" <<'APPLESCRIPT'
on run argv
  set volumePath to item 1 of argv
  tell application "Finder"
    activate
    set installerFolder to folder (POSIX file volumePath as alias)
    open installerFolder
    delay 1
    set installerWindow to container window of installerFolder
    set current view of installerWindow to icon view
    set toolbar visible of installerWindow to false
    set statusbar visible of installerWindow to false
    set pathbar visible of installerWindow to false
    set bounds of installerWindow to {160, 120, 800, 588}
    set arrangement of icon view options of installerWindow to not arranged
    set icon size of icon view options of installerWindow to 96
    set text size of icon view options of installerWindow to 13
    set label position of icon view options of installerWindow to bottom
    set shows item info of icon view options of installerWindow to false
    set background picture of icon view options of installerWindow to (POSIX file (volumePath & "/.background/installer.tiff") as alias)
    set position of item "EastSea.app" of installerFolder to {168, 212}
    set position of item "Applications" of installerFolder to {472, 212}
    update installerFolder without registering applications
    delay 2
    close installerWindow
  end tell
end run
APPLESCRIPT
attempt=0
while [ ! -s "$mountpoint/.DS_Store" ] && [ "$attempt" -lt 10 ]; do
  sleep 1
  attempt=$((attempt + 1))
done
[ -s "$mountpoint/.DS_Store" ] || { echo "Finder did not save the installer layout" >&2; exit 1; }
hdiutil detach -quiet "$mountpoint"
rmdir "$mountpoint"
mountpoint=
hdiutil convert -quiet -format UDZO -o "$dmg" "$stage/installer.dmg"

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
