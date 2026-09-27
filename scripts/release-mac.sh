#!/usr/bin/env bash
# Publish the macOS app: notarized DMG + Sparkle appcast on GitHub Releases.
#   scripts/release-mac.sh [--draft]
# Version and build number come from apps/wallet/project.yml (MARKETING_VERSION,
# CURRENT_PROJECT_VERSION); bump the build number for every release.
# Needs: Developer ID (Pipln), ASC API key (~/.config/app-store-release/env.sh),
# the Sparkle EdDSA key in the login keychain (account "aether-pipln"), gh.
set -euo pipefail
cd "$(dirname "$0")/.."
yml=apps/wallet/project.yml
version=$(awk '/MARKETING_VERSION:/{print $2; exit}' "$yml")
build=$(awk '/CURRENT_PROJECT_VERSION:/{print $2; exit}' "$yml")
tag="app-v$version"
repo=kjaylee/aether-node
dmg="dist/Aether-$version.dmg"

source ~/.config/app-store-release/env.sh
AETHER_VERSION="$version" SIGN_IDENTITY="Developer ID Application: Pipln (45WU468FZE)" scripts/package-mac.sh

sign=apps/wallet/build/SourcePackages/artifacts/sparkle/Sparkle/bin/sign_update
attrs=$("$sign" --account aether-pipln "$dmg")   # sparkle:edSignature="…" length="…"
url="https://github.com/$repo/releases/download/$tag/Aether-$version.dmg"
cat > dist/appcast.xml <<XML
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>Aether</title>
    <item>
      <title>Aether $version</title>
      <pubDate>$(LC_ALL=C date -u "+%a, %d %b %Y %H:%M:%S +0000")</pubDate>
      <sparkle:version>$build</sparkle:version>
      <sparkle:shortVersionString>$version</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>14.0</sparkle:minimumSystemVersion>
      <enclosure url="$url" type="application/octet-stream" $attrs />
    </item>
  </channel>
</rss>
XML

notes="Aether $version for macOS (Apple silicon), signed by Pipln and notarized by Apple.

Drag Aether to Applications. The app is a wallet and, with the switch on, a node that verifies every block on this Mac. It connects to the Aether testnet (test tokens have no value).

Updates arrive automatically (Aether ▸ Check for Updates…).${RELEASE_NOTES:+

$RELEASE_NOTES}"
# The browser extension ships with each app release.
scripts/build-extension.sh --zip >/dev/null
ext_version=$(python3 -c 'import json; print(json.load(open("apps/extension/manifest.json"))["version"])')
ext="dist/aether-extension-$ext_version.zip"

# Each release gets its own commit on top of the public main: the main tree
# plus releases/<tag>.md, dated now, so GitHub lists releases newest first
# (it orders them by the tagged commit's date). Only the tag is pushed; no
# branch moves and no source beyond the public main goes out.
git fetch -q origin main
if git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null; then
  echo "tag $tag already exists on origin"; exit 1
fi
notes_file=$(mktemp)
printf '# Aether %s (testnet)\n\n%s\n' "$version" "$notes" > "$notes_file"
idx=$(mktemp -u)
GIT_INDEX_FILE="$idx" git read-tree origin/main
blob=$(git hash-object -w "$notes_file")
GIT_INDEX_FILE="$idx" git update-index --add --cacheinfo "100644,$blob,releases/$tag.md"
tree=$(GIT_INDEX_FILE="$idx" git write-tree)
rm -f "$idx" "$notes_file"
commit=$(git commit-tree "$tree" -p origin/main -m "release: Aether $version (testnet)")
git tag -f "$tag" "$commit" >/dev/null
git push -q origin "refs/tags/$tag"

gh release create "$tag" "$dmg" dist/appcast.xml "$ext" --repo "$repo" --verify-tag --title "Aether $version (testnet)" --notes "$notes" --latest ${1:-}
echo "released $tag"
