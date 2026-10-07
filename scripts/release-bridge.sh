#!/usr/bin/env bash
# Publish Aether 0.6.7, the bridge that moves Aether 0.6.6 users to EastSea
# (apps/bridge; release-070 review B2). Run it BEFORE the first EastSea release
# that goes to `latest`:
#   scripts/release-bridge.sh --prepare   build, sign, notarize, DMG + dist/appcast.xml; publish nothing
#   scripts/release-bridge.sh             the same, then a GitHub release app-v<bridge version>
#                                         that is NOT marked latest (EastSea's release is)
# Feeds: every Aether <= 0.6.6 reads releases/latest/download/appcast.xml. That
# file must always list this bridge, so each EastSea release re-attaches the
# bridge's appcast.xml (scripts/release-mac.sh) next to its own
# eastsea-appcast.xml. The bridge itself reads eastsea-appcast.xml at latest.
# Needs: Developer ID (Pipln), ASC API key (~/.config/app-store-release/env.sh),
# the Sparkle EdDSA key in the login keychain (account "aether-pipln"), gh.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p tmp dist
export TMPDIR="$PWD/tmp"
yml=apps/bridge/project.yml
version=$(awk '/MARKETING_VERSION:/{print $2; exit}' "$yml")
build=$(awk '/CURRENT_PROJECT_VERSION:/{print $2; exit}' "$yml")
tag="app-v$version"
repo=eastsea-xyz/eastsea
dmg="dist/Aether-$version.dmg"
identity="Developer ID Application: Pipln (45WU468FZE)"
mode=${1:-}
source ~/.config/app-store-release/env.sh

scripts/build-bridge.sh >/dev/null
src=apps/bridge/build/Build/Products/Release/Aether.app
stage=$(mktemp -d)
trap 'rm -rf "${stage:?}"' EXIT
app="$stage/Aether.app"
cp -R "$src" "$app"
# Inside out: Sparkle's nested code, then the app.
sp="$app/Contents/Frameworks/Sparkle.framework"
for x in "$sp"/Versions/B/XPCServices/*.xpc "$sp/Versions/B/Autoupdate" "$sp/Versions/B/Updater.app" "$sp"; do
  [ -e "$x" ] && codesign --force --options runtime --timestamp --sign "$identity" "$x"
done
codesign --force --options runtime --timestamp --sign "$identity" "$app"
codesign --verify --deep --strict "$app"
ln -s /Applications "$stage/Applications"
rm -f "$dmg"
hdiutil create -quiet -volname "Aether" -srcfolder "$stage" -fs HFS+ -format UDZO "$dmg"
codesign --force --timestamp --sign "$identity" "$dmg"
xcrun notarytool submit "$dmg" --key "$APP_STORE_CONNECT_API_KEY_PATH" --key-id "$APP_STORE_CONNECT_API_KEY_ID" \
  --issuer "$APP_STORE_CONNECT_API_KEY_ISSUER_ID" --wait
xcrun stapler staple "$dmg"
spctl --assess --type open --context context:primary-signature --verbose "$dmg"

sign=apps/bridge/build/SourcePackages/artifacts/sparkle/Sparkle/bin/sign_update
attrs=$("$sign" --account aether-pipln "$dmg")   # sparkle:edSignature="…" length="…"
url="https://github.com/$repo/releases/download/$tag/Aether-$version.dmg"
# The legacy feed: what Aether 0.6.6 (and this bridge's own Sparkle) reads.
cat > dist/appcast.xml <<XML
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>Aether</title>
    <item>
      <title>Aether $version: Aether is now EastSea</title>
      <pubDate>$(LC_ALL=C date -u "+%a, %d %b %Y %H:%M:%S +0000")</pubDate>
      <sparkle:version>$build</sparkle:version>
      <sparkle:shortVersionString>$version</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>14.0</sparkle:minimumSystemVersion>
      <enclosure url="$url" type="application/octet-stream" $attrs />
    </item>
  </channel>
</rss>
XML
cp dist/appcast.xml "dist/Aether-$version-appcast.xml"
echo "DMG SHA-256: $(shasum -a 256 "$dmg" | awk '{print $1}')  $dmg"
if [ "$mode" = --prepare ]; then
  echo "Prepared $dmg and dist/appcast.xml. Nothing published."
  exit 0
fi

notes="Aether $version moves this Mac from Aether to EastSea: it installs EastSea (signature and Apple notarization checked) and opens it; EastSea then moves your wallet and node data over, verified. No data is deleted."
git -C "$PWD" fetch -q origin main
if git -C "$PWD" ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null; then
  echo "tag $tag already exists on origin"; exit 1
fi
notes_file=$(mktemp)
printf '# Aether %s (bridge to EastSea)\n\n%s\n' "$version" "$notes" > "$notes_file"
idx=$(mktemp -u)
GIT_INDEX_FILE="$idx" git -C "$PWD" read-tree origin/main
blob=$(git -C "$PWD" hash-object -w "$notes_file")
GIT_INDEX_FILE="$idx" git -C "$PWD" update-index --add --cacheinfo "100644,$blob,releases/$tag.md"
tree=$(GIT_INDEX_FILE="$idx" git -C "$PWD" write-tree)
rm -f "$idx" "$notes_file"
commit=$(git -C "$PWD" commit-tree "$tree" -p origin/main -m "release: Aether $version (bridge to EastSea)")
git -C "$PWD" tag -f "$tag" "$commit" >/dev/null
git -C "$PWD" push -q origin "refs/tags/$tag"
# Not latest: 0.6.6 must keep reading the current latest appcast.xml until the
# EastSea release (with this appcast.xml re-attached) becomes latest.
gh release create "$tag" "$dmg" dist/appcast.xml --repo "$repo" --verify-tag \
  --title "Aether $version (bridge to EastSea)" --notes "$notes" --latest=false
echo "released $tag (not latest). Next: scripts/release-mac.sh for EastSea, which re-attaches this appcast.xml."
