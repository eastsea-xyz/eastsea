#!/usr/bin/env bash
# Publish the macOS app: notarized DMG + Sparkle appcast on GitHub Releases.
#   scripts/release-mac.sh --prepare            # build and print fingerprints
#   scripts/release-mac.sh --publish-prepared   # after builder signing and chain publication
#   scripts/release-mac.sh [--draft]            # legacy 7780 release
#   scripts/release-mac.sh --publish-draft      # retry gates on the staged GitHub DMG
#   scripts/release-mac.sh --dry-run --canary-host stub-mac
# Every publication is staged as a draft, then VM and canary smoke-tested.
# --draft leaves the tested release as a draft; --skip-vm-smoke is an explicit,
# logged exception only when Tart/the clean base is not set up. Canary is mandatory.
# Version and build number come from apps/wallet/project.yml (MARKETING_VERSION,
# CURRENT_PROJECT_VERSION); bump the build number for every release.
# Feeds (release-070 review B2): EastSea 0.7.0+ reads eastsea-appcast.xml, which
# this script writes. appcast.xml is the LEGACY feed every Aether <= 0.6.6
# reads; its Sparkle can only install Aether.app, so that file must list the
# Aether bridge (scripts/release-bridge.sh) and never an EastSea. Every release
# here re-attaches the bridge release's appcast.xml, because both feeds are read
# from whichever release is `latest`.
# Needs: Developer ID (Pipln), ASC API key (~/.config/app-store-release/env.sh),
# the Sparkle EdDSA key in the login keychain (account "aether-pipln"), gh.
# A Terms.version change needs TERMS_BUMP_REASON (dist/release-gates.log).
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
yml=apps/wallet/project.yml
version=$(awk '/MARKETING_VERSION:/{print $2; exit}' "$yml")
build=$(awk '/CURRENT_PROJECT_VERSION:/{print $2; exit}' "$yml")
tag="app-v$version"
repo=eastsea-xyz/eastsea
dmg="dist/EastSea-$version.dmg"
mode=""
keep_draft=0
skip_vm_smoke=0
dry_run=0
canary_host=poc-m3
while [ "$#" -gt 0 ]; do
  case "$1" in
    --prepare|--publish-prepared|--publish-draft)
      [ -z "$mode" ] || { echo "REFUSED: choose one release mode" >&2; exit 2; }
      mode=$1 ;;
    --draft) keep_draft=1 ;;
    --skip-vm-smoke) skip_vm_smoke=1 ;;
    --dry-run) dry_run=1 ;;
    --canary-host)
      [ "$#" -ge 2 ] && [ -n "$2" ] || { echo "REFUSED: --canary-host needs a host" >&2; exit 2; }
      canary_host=$2; shift ;;
    --help|-h)
      echo "Usage: $0 [--prepare|--publish-prepared|--publish-draft] [--draft] [--skip-vm-smoke] [--canary-host HOST] [--dry-run]"
      exit 0 ;;
    *) echo "REFUSED: unknown release option: $1" >&2; exit 2 ;;
  esac
  shift
done
[[ "$canary_host" =~ ^([A-Za-z_][A-Za-z0-9_.-]*@)?[A-Za-z0-9][A-Za-z0-9_.-]*$ ]] || { echo "REFUSED: --canary-host needs an SSH alias, hostname or IPv4 address" >&2; exit 2; }
[ "$mode" != --prepare ] || [ "$keep_draft" -eq 0 ] || { echo "REFUSED: --prepare does not create a draft" >&2; exit 2; }
manifest="dist/EastSea-$version-manifest.json"
builder_sigs="dist/EastSea-$version-builder-sigs.json"
release_index="dist/EastSea-$version-release-index.json"
gates_log="dist/release-gates.log"

release_record() {
  if [ "$dry_run" -eq 1 ]; then printf '%s\n' "$*"; else printf '%s\n' "$*" | tee -a "$gates_log"; fi
}
release_command() {
  if [ "$dry_run" -eq 1 ]; then
    printf 'DRY-RUN:'; printf ' %q' "$@"; printf '\n'
  else
    "$@" 2>&1 | tee -a "$gates_log"
  fi
}
release_candidate_unchanged() {
  local current_sha
  current_sha=$(shasum -a 256 "$dmg" | awk '{print $1}') || { release_record "FAIL: cannot fingerprint DMG; release remains draft"; return 1; }
  if [ "$1" != "$current_sha" ]; then
    release_record "FAIL: DMG changed during smoke gates; release remains draft"
    return 1
  fi
}
release_smoke_gates() {
  local candidate_sha=""
  local vm_setup_status vm_setup_evidence
  release_record "Smoke candidate: $tag ($dmg); canary=$canary_host"
  if [ "$dry_run" -eq 0 ]; then
    candidate_sha=$(shasum -a 256 "$dmg" | awk '{print $1}') || return 1
    release_record "Smoke started: $(date -u '+%Y-%m-%dT%H:%M:%SZ'); DMG SHA-256: $candidate_sha"
  fi
  # --publish-prepared and --publish-draft must also consume a notarized DMG.
  release_command xcrun stapler validate "$dmg" || { release_record "FAIL: notarization ticket; release remains draft"; return 1; }
  if [ "$skip_vm_smoke" -eq 1 ]; then
    vm_setup_evidence=$(scripts/release-vm-smoke.sh --check-setup 2>&1) && vm_setup_status=0 || vm_setup_status=$?
    [ -z "$vm_setup_evidence" ] || release_record "$vm_setup_evidence"
    case "$vm_setup_status" in
      0) release_record "FAIL: --skip-vm-smoke refused: Tart and the VM base are set up; run the VM gate"; return 1 ;;
      78) : ;;
      *) release_record "FAIL: cannot check VM setup (exit $vm_setup_status); release remains draft"; return 1 ;;
    esac
    release_record "ALARM: VM SMOKE SKIPPED (--skip-vm-smoke): Tart or eastsea-smoke-base is not set up. Canary remains mandatory."
  elif [ "$dry_run" -eq 1 ]; then
    release_command scripts/release-vm-smoke.sh --dry-run "$dmg"
    scripts/release-vm-smoke.sh --dry-run "$dmg" || return 1
  else
    release_command scripts/release-vm-smoke.sh "$dmg" || { release_record "FAIL: VM smoke; release remains draft"; return 1; }
  fi
  if [ "$dry_run" -eq 0 ]; then release_candidate_unchanged "$candidate_sha" || return 1; fi
  if [ "$dry_run" -eq 1 ]; then
    release_command scripts/release-canary.sh --dry-run "$dmg" "$canary_host"
    scripts/release-canary.sh --dry-run "$dmg" "$canary_host" || return 1
  else
    release_command scripts/release-canary.sh "$dmg" "$canary_host" || { release_record "FAIL: canary smoke; release remains draft"; return 1; }
  fi
  if [ "$dry_run" -eq 0 ]; then release_candidate_unchanged "$candidate_sha" || return 1; fi
}
release_finish_draft() {
  release_smoke_gates || return 1
  if [ "$keep_draft" -eq 1 ]; then
    if [ "$dry_run" -eq 1 ]; then
      release_record "DRY-RUN: would keep $tag draft after gates (--draft)"
    else
      release_record "Gates passed; $tag remains draft (--draft). Use --publish-draft to recheck and publish."
    fi
  else
    release_command gh release edit "$tag" --repo "$repo" --draft=false --latest || return 1
    if [ "$dry_run" -eq 1 ]; then
      release_record "DRY-RUN: would publish $tag as latest after gates"
    else
      release_record "released $tag as latest after smoke gates"
    fi
  fi
}
release_load_draft_candidate() {
  local expected_sha=${1:-} staged_sha
  [ "$(gh release view "$tag" --repo "$repo" --json isDraft --jq .isDraft)" = true ] || { release_record "REFUSED: $tag must be an existing draft"; return 1; }
  draft_work=$(mktemp -d "$TMPDIR/publish-draft.XXXXXX")
  trap 'rm -rf "${draft_work:?}"' EXIT
  gh release download "$tag" --repo "$repo" --pattern "EastSea-$version.dmg" --dir "$draft_work"
  dmg="$draft_work/EastSea-$version.dmg"
  [ -f "$dmg" ] || { release_record "REFUSED: staged draft DMG missing"; return 1; }
  staged_sha=$(shasum -a 256 "$dmg" | awk '{print $1}')
  if [ -n "$expected_sha" ] && [ "$expected_sha" != "$staged_sha" ]; then
    release_record "FAIL: uploaded DMG differs from the candidate fingerprint; release remains draft"
    return 1
  fi
  release_record "Staged DMG SHA-256: $staged_sha"
}

if [ "$dry_run" -eq 1 ]; then
  if [ "$mode" = --prepare ]; then
    echo "DRY-RUN: build, notarize and prepare artifacts; no GitHub publication or smoke gates"
    exit 0
  fi
  echo "DRY-RUN: publication after packaging/notarization; no build, remote connection or app launch"
  if [ "$mode" != --publish-draft ]; then
    release_command gh release create "$tag" "$dmg" dist/eastsea-appcast.xml dist/appcast.xml --repo "$repo" --verify-tag --title "EastSea $version (testnet)" --draft --latest=false
  fi
  release_command gh release view "$tag" --repo "$repo" --json isDraft --jq .isDraft
  release_command gh release download "$tag" --repo "$repo" --pattern "EastSea-$version.dmg" --dir 'tmp/publish-draft.RUN'
  dmg="tmp/publish-draft.RUN/EastSea-$version.dmg"
  release_finish_draft
  exit 0
fi

mkdir -p tmp dist
export TMPDIR="$PWD/tmp"
if [ "$mode" = --publish-draft ]; then
  release_load_draft_candidate
  release_finish_draft
  exit 0
fi

if [ "$mode" != --publish-prepared ]; then
# shellcheck disable=SC1090
source ~/.config/app-store-release/env.sh
if [ "$mode" = --prepare ]; then : "${AETHER_RELEASE_LOG:?set AETHER_RELEASE_LOG to prepare an on-chain release}"; fi
if [ -n "${AETHER_RELEASE_LOG:-}" ]; then
  [ -z "$(git -C "$PWD" status --porcelain)" ] || { echo "ALARM: source worktree is dirty" >&2; exit 1; }
  source_commit=$(git -C "$PWD" rev-parse "refs/tags/$tag^{commit}")
  [ "$source_commit" = "$(git -C "$PWD" rev-parse HEAD)" ] || { echo "ALARM: $tag is not the source commit being built" >&2; exit 1; }
  SOURCE_DATE_EPOCH=$(git -C "$PWD" show -s --format=%ct HEAD)
  export SOURCE_DATE_EPOCH
  export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }--remap-path-prefix=$PWD=/aether-src"
  export OTHER_SWIFT_FLAGS="${OTHER_SWIFT_FLAGS:+$OTHER_SWIFT_FLAGS }-debug-prefix-map $PWD=/aether-src"
fi
# A clean build has no old node to retain. Obtain the previous release before
# building, mount it read only, and pass that explicit input to the packager.
# The temporary mount is detached before later release traps/publication run.
rollback_work=""
rollback_mount=""
rollback_mounted=0
read_rollback_mount() {
  python3 - "$rollback_work/attach.plist" <<'PY'
import pathlib, plistlib, sys
with open(sys.argv[1], 'rb') as f:
    mounts = [e['mount-point'] for e in plistlib.load(f)['system-entities']
              if 'mount-point' in e and (pathlib.Path(e['mount-point']) / 'EastSea.app').is_dir()]
if len(mounts) != 1:
    raise SystemExit('REFUSED: previous DMG must mount exactly one EastSea.app')
print(mounts[0])
PY
}
cleanup_rollback_release() {
  if [ "$rollback_mounted" = 1 ]; then
    if [ -z "$rollback_mount" ]; then
      rollback_mount=$(read_rollback_mount) || return
    fi
    hdiutil detach -quiet "$rollback_mount" || return
    rollback_mounted=0
  fi
  if [ -n "$rollback_work" ]; then rm -rf "${rollback_work:?}"; rollback_work=""; fi
}
if [ -z "${AETHER_PREVIOUS_APP:-}" ]; then
  rollback_work=$(mktemp -d "$TMPDIR/rollback-release.XXXXXX")
  trap cleanup_rollback_release EXIT
  prev_tag=${PREV_RELEASE_TAG:-}
  previous_download=""
  if [ -n "$prev_tag" ]; then
    [ "$prev_tag" != "$tag" ] || { echo "REFUSED: previous release cannot be the release being built" >&2; exit 1; }
    previous_download=$(mktemp -d "$rollback_work/download.XXXXXX")
    gh release download "$prev_tag" --repo "$repo" --pattern 'EastSea-*.dmg' --dir "$previous_download"
  else
    for previous_tag in $(gh release list --repo "$repo" --exclude-drafts --exclude-pre-releases --limit 30 --json tagName --jq '.[].tagName'); do
      [ "$previous_tag" = "$tag" ] && continue
      candidate=$(mktemp -d "$rollback_work/download.XXXXXX")
      if gh release download "$previous_tag" --repo "$repo" --pattern 'EastSea-*.dmg' --dir "$candidate"; then
        prev_tag=$previous_tag; previous_download=$candidate; break
      fi
    done
  fi
  [ -n "$prev_tag" ] && [ -n "$previous_download" ] || { echo "REFUSED: no previous EastSea release found for rollback" >&2; exit 1; }
  shopt -s nullglob
  previous_dmgs=("$previous_download"/EastSea-*.dmg)
  shopt -u nullglob
  [ "${#previous_dmgs[@]}" -eq 1 ] || { echo "REFUSED: previous release must have exactly one EastSea DMG" >&2; exit 1; }
  # Set the state before attach so an interrupted/partly successful attach
  # cannot make cleanup recurse into a still-mounted previous release.
  rollback_mounted=1
  # Let hdiutil choose its system volume mount; explicit mountpoints on the
  # external workspace fail with EPERM. All scratch files stay in repo tmp/.
  hdiutil attach -readonly -nobrowse -plist "${previous_dmgs[0]}" > "$rollback_work/attach.plist"
  rollback_mount=$(read_rollback_mount)
  export AETHER_PREVIOUS_APP="$rollback_mount/EastSea.app"
  export PREV_RELEASE_TAG="$prev_tag"
fi
AETHER_VERSION="$version" SIGN_IDENTITY="Developer ID Application: Pipln (45WU468FZE)" scripts/package-mac.sh
cleanup_rollback_release
trap - EXIT
# Same app to macOS as the previous release (bundle id, team, designated
# requirement), and no silent Terms bump: otherwise refuse before anything is
# signed for Sparkle or published (scripts/release-identity-gate.sh).
scripts/release-identity-gate.sh release "$dmg" "$tag" "$repo"
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
    --dmg "$dmg" --app "$PWD/tmp/release-mount/EastSea.app" --sparkle-signature "$sparkle_sig" --source-tag "$tag" \
    --out "$manifest" ${emergency[@]+"${emergency[@]}"}
  hdiutil detach -quiet "$PWD/tmp/release-mount"
  trap - EXIT
  if [ "${#emergency[@]}" -eq 0 ]; then
    echo "After 2 builder signatures: scripts/release-approve.py combine $manifest SIGNATURE1.json SIGNATURE2.json --out $builder_sigs"
  else
    echo "EMERGENCY: all 3 builders must sign. scripts/release-approve.py combine $manifest SIGNATURE1.json SIGNATURE2.json SIGNATURE3.json --out $builder_sigs"
  fi
fi

url="https://github.com/$repo/releases/download/$tag/EastSea-$version.dmg"
cat > dist/eastsea-appcast.xml <<XML
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

Drag the app to Applications. The app is a wallet and, with the switch on, a node that verifies every block on this Mac. It connects to the EastSea testnet (test tokens have no value).

Updates arrive automatically (EastSea ▸ Check for Updates…).${RELEASE_NOTES:+

$RELEASE_NOTES}"
# The browser extension ships through the Chrome/Edge stores only. Its zip is built and hashed
# here for the store upload but is NOT a public release asset: release approval signs the DMG
# and the app executables, not this zip, so a swapped zip must never be installable from the
# GitHub release (audit 3, A3-6).
if [ "$mode" != --publish-prepared ]; then scripts/build-extension.sh --zip >/dev/null; fi
ext_version=$(python3 -c 'import json; print(json.load(open("apps/extension/manifest.json"))["version"])')
ext="dist/eastsea-extension-$ext_version.zip"
echo "Extension zip SHA-256: $(shasum -a 256 "$ext" | awk '{print $1}')  $ext"

if [ "$mode" = --prepare ] || { [ -n "${AETHER_RELEASE_LOG:-}" ] && [ "$mode" != --publish-prepared ]; }; then
  echo "Prepared artifacts only. Publish the manifest on chain, then run --publish-prepared with AETHER_RELEASE_INDEX."
  exit 0
fi
# The legacy feed, taken from the bridge's own release and checked to list the
# bridge (an Aether.app DMG, >= 0.6.7): without it, publishing this release as
# latest would leave every Aether 0.6.6 reading a feed it cannot install from.
# AETHER_LEGACY_FEED=frozen (founder 2026-10-07: retire 0.6.6, no bridge):
# re-attach 0.6.6's own feed, which lists only Aether <= 0.6.6, so Aether
# offers no update at all instead of an EastSea it cannot install.
if [ "${AETHER_LEGACY_FEED:-bridge}" = frozen ]; then
  bridge_tag=app-v0.6.6; want=frozen
else
  bridge_tag=${AETHER_BRIDGE_TAG:-app-v0.6.7}; want=bridge
fi
rm -rf tmp/bridge-feed && mkdir -p tmp/bridge-feed
gh release download "$bridge_tag" --repo "$repo" --pattern appcast.xml --dir tmp/bridge-feed
python3 - tmp/bridge-feed/appcast.xml "$want" <<'PY'
import sys, xml.etree.ElementTree as ET
ns = {"sparkle": "http://www.andymatuschak.org/xml-namespaces/sparkle"}
items = ET.parse(sys.argv[1]).getroot().findall("./channel/item")
assert items, "the legacy feed has no item"
for it in items:
    url = it.find("enclosure").get("url")
    short = tuple(map(int, it.find("sparkle:shortVersionString", ns).text.split(".")))
    assert "/Aether-" in url and url.endswith(".dmg"), f"legacy feed item is not an Aether.app DMG: {url}"
    if sys.argv[2] == "frozen":
        assert short <= (0, 6, 6), f"frozen legacy feed offers {short}, newer than 0.6.6"
    else:
        assert short >= (0, 6, 7), f"legacy feed item {short} is not the bridge"
print(f"legacy feed ({sys.argv[2]}):", ", ".join(i.find("sparkle:shortVersionString", ns).text for i in items))
PY
cp tmp/bridge-feed/appcast.xml dist/appcast.xml
assets=("$dmg" dist/eastsea-appcast.xml dist/appcast.xml)
if [ "$mode" = --publish-prepared ]; then
  : "${AETHER_RELEASE_INDEX:?set the finalized ReleaseLog entry index}"
  scripts/release-approve.py finalize --manifest "$manifest" --signatures "$builder_sigs" \
    --dmg "$dmg" --appcast dist/eastsea-appcast.xml --version "$version" --build "$build" \
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

uploaded_sha=$(shasum -a 256 "$dmg" | awk '{print $1}')
release_command gh release create "$tag" "${assets[@]}" --repo "$repo" --verify-tag --title "EastSea $version (testnet)" --notes "$notes" --draft --latest=false
# Consume GitHub's uploaded bytes, and refuse a concurrent change during upload.
release_load_draft_candidate "$uploaded_sha"
release_finish_draft
