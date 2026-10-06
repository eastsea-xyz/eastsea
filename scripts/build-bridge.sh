#!/usr/bin/env bash
# Build Aether 0.6.7, the bridge from Aether to EastSea (apps/bridge), Release,
# from a clean derived-data directory (only the resolved Swift packages kept).
#   scripts/build-bridge.sh                     signed as project.yml says (Developer ID)
#   BRIDGE_ADHOC=1 scripts/build-bridge.sh      local check: ad-hoc signature
# Output: apps/bridge/build/Build/Products/Release/Aether.app
set -euo pipefail
cd "$(dirname "$0")/../apps/bridge"
xcodegen generate >/dev/null
if [ -d build ]; then
  find build -mindepth 1 -maxdepth 1 ! -name SourcePackages -exec rm -rf {} +
fi
# Optional settings as positional parameters: bash 3.2 + set -u has no
# empty-array expansion (scripts/test-release-scripts.sh).
set --
if [ "${BRIDGE_ADHOC:-0}" = 1 ]; then set -- CODE_SIGN_IDENTITY=- CODE_SIGN_STYLE=Manual DEVELOPMENT_TEAM=; fi
xcodebuild -project AetherBridge.xcodeproj -scheme AetherBridge -configuration Release \
  -derivedDataPath build "$@" build | grep -E "BUILD|error:"
app=build/Build/Products/Release/Aether.app
[ -d "$app" ] || { echo "build failed: $app missing" >&2; exit 1; }
# What Sparkle in Aether 0.6.6 requires of an update it installs.
id=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist")
name=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleName' "$app/Contents/Info.plist")
[ "$id" = com.pipln.aether ] && [ "$name" = Aether ] || { echo "bridge must be Aether.app / com.pipln.aether (got $name / $id)" >&2; exit 1; }
[ ! -e "$app/Contents/Helpers" ] || { echo "the bridge must not carry a node (Contents/Helpers)" >&2; exit 1; }
codesign --verify --deep --strict "$app"
echo "$PWD/$app $(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist") ($(/usr/libexec/PlistBuddy -c 'Print :CFBundleVersion' "$app/Contents/Info.plist"))"
