#!/usr/bin/env bash
# Publish the macOS app: notarized DMG + Sparkle appcast on GitHub Releases.
#   scripts/release-mac.sh --prepare            # build and print fingerprints
#   scripts/release-mac.sh --publish-prepared   # after builder signing and chain publication
#   scripts/release-mac.sh [--draft]            # legacy 7780 release
# Version and build number come from apps/wallet/project.yml (MARKETING_VERSION,
# CURRENT_PROJECT_VERSION); bump the build number for every release.
# Needs: Developer ID (Pipln), ASC API key (~/.config/app-store-release/env.sh),
# the Sparkle EdDSA key in the login keychain (account "aether-pipln"), gh.
set -euo pipefail
cd "$(dirname "$0")/.."
mkdir -p tmp
export TMPDIR="$PWD/tmp"
export PATH="$HOME/.cargo/bin:$PATH"
yml=apps/wallet/project.yml
version=$(awk '/MARKETING_VERSION:/{print $2; exit}' "$yml")
build=$(awk '/CURRENT_PROJECT_VERSION:/{print $2; exit}' "$yml")
tag="app-v$version"
repo=kjaylee/aether-node
dmg="dist/Aether-$version.dmg"
mode=${1:-}
manifest="dist/Aether-$version-manifest.json"
builder_sigs="dist/Aether-$version-builder-sigs.json"
release_index="dist/Aether-$version-release-index.json"

if [ "$mode" != --publish-prepared ]; then
source ~/.config/app-store-release/env.sh
if [ "$mode" = --prepare ]; then : "${AETHER_RELEASE_LOG:?set AETHER_RELEASE_LOG to prepare an on-chain release}"; fi
if [ -n "${AETHER_RELEASE_LOG:-}" ]; then
  [ -z "$(git -C "$PWD" status --porcelain)" ] || { echo "ALARM: source worktree is dirty" >&2; exit 1; }
  source_commit=$(git -C "$PWD" rev-parse "refs/tags/$tag^{commit}")
  [ "$source_commit" = "$(git -C "$PWD" rev-parse HEAD)" ] || { echo "ALARM: $tag is not the source commit being built" >&2; exit 1; }
  export SOURCE_DATE_EPOCH=$(git -C "$PWD" show -s --format=%ct HEAD)
  export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }--remap-path-prefix=$PWD=/aether-src"
  export OTHER_SWIFT_FLAGS="${OTHER_SWIFT_FLAGS:+$OTHER_SWIFT_FLAGS }-debug-prefix-map $PWD=/aether-src"
fi
AETHER_VERSION="$version" SIGN_IDENTITY="Developer ID Application: Pipln (45WU468FZE)" scripts/package-mac.sh
sign=apps/wallet/build/SourcePackages/artifacts/sparkle/Sparkle/bin/sign_update
attrs=$("$sign" --account aether-pipln "$dmg")   # sparkle:edSignature="…" length="…"
echo "DMG SHA-256: $(shasum -a 256 "$dmg" | awk '{print $1}')  $dmg"
if [ -n "${AETHER_RELEASE_LOG:-}" ]; then
  : "${AETHER_RELEASE_CHAIN_ID:?set AETHER_RELEASE_CHAIN_ID with AETHER_RELEASE_LOG}"
  [[ "$attrs" =~ sparkle:edSignature=\"([^\"]+)\" ]] || { echo "Sparkle EdDSA signature missing" >&2; exit 1; }
  sparkle_sig=${BASH_REMATCH[1]}
  emergency=()
  if [ "${AETHER_RELEASE_EMERGENCY:-0}" = 1 ]; then emergency=(--emergency); fi
  mkdir -p tmp/release-mount
  hdiutil attach -quiet -readonly -nobrowse -mountpoint "$PWD/tmp/release-mount" "$dmg"
  trap 'hdiutil detach -quiet "$PWD/tmp/release-mount" 2>/dev/null || true' EXIT
  scripts/release-approve.py prepare --chain-id "$AETHER_RELEASE_CHAIN_ID" \
    --log "$AETHER_RELEASE_LOG" --version "$version" --build "$build" \
    --dmg "$dmg" --app "$PWD/tmp/release-mount/Aether.app" --sparkle-signature "$sparkle_sig" --source-tag "$tag" \
    --out "$manifest" "${emergency[@]}"
  hdiutil detach -quiet "$PWD/tmp/release-mount"
  trap - EXIT
  if [ "${#emergency[@]}" -eq 0 ]; then
    echo "After 2 builder signatures: scripts/release-approve.py combine $manifest SIGNATURE1.json SIGNATURE2.json --out $builder_sigs"
  else
    echo "EMERGENCY: all 3 builders must sign. scripts/release-approve.py combine $manifest SIGNATURE1.json SIGNATURE2.json SIGNATURE3.json --out $builder_sigs"
  fi
fi

url="https://github.com/$repo/releases/download/$tag/Aether-$version.dmg"
cat > dist/appcast.xml <<XML
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>EastSea</title>
    <item>
      <title>EastSea $version</title>
      <pubDate>$(LC_ALL=C date -u "+%a, %d %b %Y %H:%M:%S +0000")</pubDate>
      <sparkle:version>$build</sparkle:version>
      <sparkle:shortVersionString>$version</sparkle:shortVersionString>
      <sparkle:minimumSystemVersion>14.0</sparkle:minimumSystemVersion>
      <enclosure url="$url" type="application/octet-stream" $attrs />
    </item>
  </channel>
</rss>
XML
fi

notes="EastSea $version for macOS (Apple silicon), signed by Pipln and notarized by Apple.

Drag the app to Applications. The app is a wallet and, with the switch on, a node that verifies every block on this Mac. It connects to the Aether testnet (test tokens have no value).

Updates arrive automatically (EastSea ▸ Check for Updates…).${RELEASE_NOTES:+

$RELEASE_NOTES}"
# The browser extension ships with each app release.
if [ "$mode" != --publish-prepared ]; then scripts/build-extension.sh --zip >/dev/null; fi
ext_version=$(python3 -c 'import json; print(json.load(open("apps/extension/manifest.json"))["version"])')
ext="dist/aether-extension-$ext_version.zip"
echo "Extension zip SHA-256: $(shasum -a 256 "$ext" | awk '{print $1}')  $ext"

if [ "$mode" = --prepare ] || { [ -n "${AETHER_RELEASE_LOG:-}" ] && [ "$mode" != --publish-prepared ]; }; then
  echo "Prepared artifacts only. Publish the manifest on chain, then run --publish-prepared with AETHER_RELEASE_INDEX."
  exit 0
fi
assets=("$dmg" dist/appcast.xml "$ext")
if [ "$mode" = --publish-prepared ]; then
  : "${AETHER_RELEASE_INDEX:?set the finalized ReleaseLog entry index}"
  scripts/release-approve.py finalize --manifest "$manifest" --signatures "$builder_sigs" \
    --dmg "$dmg" --appcast dist/appcast.xml --version "$version" --build "$build" \
    --index "$AETHER_RELEASE_INDEX" --out "$release_index"
  assets+=("$manifest" "$builder_sigs" "$release_index")
  assets+=("${manifest%.json}.inventory.json")
fi

# Each release gets its own commit on top of the public main: the main tree
# plus releases/<tag>.md, dated now, so GitHub lists releases newest first
# (it orders them by the tagged commit's date). Only the tag is pushed; no
# branch moves and no source beyond the public main goes out.
if [ "$mode" = --publish-prepared ]; then
  git -C "$PWD" rev-parse -q --verify "refs/tags/$tag^{commit}" >/dev/null
  git -C "$PWD" push -q origin "refs/tags/$tag"
else
  git -C "$PWD" fetch -q origin main
  if git -C "$PWD" ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null; then
    echo "tag $tag already exists on origin"; exit 1
  fi
  notes_file=$(mktemp)
  printf '# EastSea %s (testnet)\n\n%s\n' "$version" "$notes" > "$notes_file"
  idx=$(mktemp -u)
  GIT_INDEX_FILE="$idx" git -C "$PWD" read-tree origin/main
  blob=$(git -C "$PWD" hash-object -w "$notes_file")
  GIT_INDEX_FILE="$idx" git -C "$PWD" update-index --add --cacheinfo "100644,$blob,releases/$tag.md"
  tree=$(GIT_INDEX_FILE="$idx" git -C "$PWD" write-tree)
  rm -f "$idx" "$notes_file"
  commit=$(git -C "$PWD" commit-tree "$tree" -p origin/main -m "release: EastSea $version (testnet)")
  git -C "$PWD" tag -f "$tag" "$commit" >/dev/null
  git -C "$PWD" push -q origin "refs/tags/$tag"
fi

draft=()
if [ "$mode" = --draft ]; then draft=(--draft); fi
gh release create "$tag" "${assets[@]}" --repo "$repo" --verify-tag --title "EastSea $version (testnet)" --notes "$notes" --latest "${draft[@]}"
echo "released $tag"
