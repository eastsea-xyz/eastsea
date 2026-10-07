#!/bin/bash
# Release identity gate: a new EastSea must look, to macOS, like the same app
# as the one users already run, or an update re-asks for permissions they
# granted (TCC, keychain, login item, the unattended daemon). Founder decision
# 2026-10-07: permissions already granted are never asked again on update.
#
#   scripts/release-identity-gate.sh check --app NEW.app --prev-app PREV.app \
#       --terms NEW/Onboarding.swift --prev-terms PREV/Onboarding.swift [--log FILE]
#       The checks alone, on paths (scripts/test-release-scripts.sh feeds fakes).
#   scripts/release-identity-gate.sh release DMG TAG REPO
#       What release-mac.sh runs: downloads the previous EastSea release's DMG
#       from GitHub (PREV_RELEASE_TAG overrides which), mounts both DMGs, finds
#       the previous release's Terms source, and runs `check`.
#
# Refuses (exit 1) unless:
#   1. the new bundle id is exactly com.pipln.eastsea;
#   2. both the new app and the previous release are signed by Developer ID
#      team 45WU468FZE (codesign -dv TeamIdentifier);
#   3. the designated requirement (codesign -dr -) is unchanged from the
#      previous release;
#   4. if the `Terms.version` line changed, TERMS_BUMP_REASON is non-empty;
#      the reason goes to the log (default dist/release-gates.log).
set -eu
cd "$(dirname "$0")/.."
BUNDLE_ID=com.pipln.eastsea
TEAM_ID=45WU468FZE
TERMS_FILE=apps/wallet/Sources/Onboarding.swift

terms_line() { grep -E '^[[:space:]]*static let version[[:space:]]*=' "$1" | head -1 | sed -E 's/^[[:space:]]+//'; }
team_of() { codesign -dv "$1" 2>&1 | sed -n 's/^TeamIdentifier=//p'; }
requirement_of() { codesign -dr - "$1" 2>/dev/null | grep '^designated =>' || true; }

check() {
  local app="" prev="" terms="" prev_terms="" log=dist/release-gates.log
  while [ $# -gt 0 ]; do
    case "$1" in
      --app) app=$2; shift 2 ;;
      --prev-app) prev=$2; shift 2 ;;
      --terms) terms=$2; shift 2 ;;
      --prev-terms) prev_terms=$2; shift 2 ;;
      --log) log=$2; shift 2 ;;
      *) echo "unknown argument $1" >&2; return 2 ;;
    esac
  done
  for f in "$app" "$prev" "$terms" "$prev_terms"; do
    [ -e "$f" ] || { echo "REFUSED: missing input '$f'" >&2; return 1; }
  done
  mkdir -p "$(dirname "$log")"
  local bad=0 when
  when=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  refuse() { echo "REFUSED: $*" >&2; echo "$when REFUSED $*" >> "$log"; bad=$((bad + 1)); }
  ok() { echo "ok: $*"; echo "$when ok $*" >> "$log"; }

  local id
  id=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist" 2>/dev/null || true)
  [ "$id" = "$BUNDLE_ID" ] && ok "bundle id $id" || refuse "bundle id is '$id', not $BUNDLE_ID"

  local new_team prev_team
  new_team=$(team_of "$app"); prev_team=$(team_of "$prev")
  [ "$new_team" = "$TEAM_ID" ] && ok "new app signed by team $new_team" \
    || refuse "new app is signed by team '$new_team', not $TEAM_ID"
  [ "$prev_team" = "$TEAM_ID" ] && ok "previous release signed by team $prev_team" \
    || refuse "previous release is signed by team '$prev_team', not $TEAM_ID"

  local new_req prev_req
  new_req=$(requirement_of "$app"); prev_req=$(requirement_of "$prev")
  if [ -n "$new_req" ] && [ "$new_req" = "$prev_req" ]; then
    ok "designated requirement unchanged: $new_req"
  else
    refuse "designated requirement changed: previous '$prev_req', new '$new_req'"
  fi

  local new_terms old_terms
  new_terms=$(terms_line "$terms"); old_terms=$(terms_line "$prev_terms")
  if [ -z "$new_terms" ] || [ -z "$old_terms" ]; then
    refuse "Terms.version not found (new '$new_terms', previous '$old_terms')"
  elif [ "$new_terms" = "$old_terms" ]; then
    ok "Terms.version unchanged: $new_terms"
  elif [ -n "${TERMS_BUMP_REASON:-}" ] && [ -n "$(printf '%s' "$TERMS_BUMP_REASON" | tr -d '[:space:]')" ]; then
    ok "Terms.version bumped '$old_terms' -> '$new_terms'; reason: $TERMS_BUMP_REASON"
  else
    refuse "Terms.version changed '$old_terms' -> '$new_terms' without TERMS_BUMP_REASON (bump only when users' rights or risks change)"
  fi
  return "$bad"
}

# The last source state of the previous version: the tag if its tree has the
# sources (an on-chain release tags its source commit), otherwise the parent
# of the commit that moved project.yml off that version (legacy tags carry
# only the public main tree), or HEAD if nothing has moved it yet.
prev_terms_source() {
  local tag=$1 out=$2 version commit newer=HEAD
  if git show "$tag:$TERMS_FILE" > "$out" 2>/dev/null; then echo "$tag"; return; fi
  version=${tag#app-v}
  for commit in $(git log --format=%H -- apps/wallet/project.yml); do
    if git show "$commit:apps/wallet/project.yml" | grep -qE "MARKETING_VERSION: $version\$"; then
      [ "$newer" = HEAD ] || newer="$newer^"
      git show "$newer:$TERMS_FILE" > "$out" && { git rev-parse "$newer"; return; }
      return 1
    fi
    newer=$commit
  done
  return 1
}

release() {
  local dmg=$1 tag=$2 repo=$3 prev_tag work
  work=$PWD/tmp/identity-gate
  rm -rf "$work"; mkdir -p "$work/new" "$work/prev" "$work/dl"
  prev_tag=${PREV_RELEASE_TAG:-}
  if [ -z "$prev_tag" ]; then
    for t in $(gh release list --repo "$repo" --exclude-drafts --limit 30 --json tagName --jq '.[].tagName'); do
      [ "$t" = "$tag" ] && continue
      if gh release download "$t" --repo "$repo" --pattern 'EastSea-*.dmg' --dir "$work/dl" 2>/dev/null; then prev_tag=$t; break; fi
    done
  else
    gh release download "$prev_tag" --repo "$repo" --pattern 'EastSea-*.dmg' --dir "$work/dl"
  fi
  [ -n "$prev_tag" ] || { echo "REFUSED: no previous EastSea release DMG found on $repo" >&2; return 1; }
  local prev_dmg
  prev_dmg=$(ls "$work"/dl/EastSea-*.dmg | head -1)
  echo "identity gate: $dmg against $prev_tag ($(basename "$prev_dmg"))"
  trap 'hdiutil detach -quiet "'"$work"'/new" 2>/dev/null || true; hdiutil detach -quiet "'"$work"'/prev" 2>/dev/null || true' EXIT
  hdiutil attach -quiet -readonly -nobrowse -mountpoint "$work/new" "$dmg"
  hdiutil attach -quiet -readonly -nobrowse -mountpoint "$work/prev" "$prev_dmg"
  local src
  src=$(prev_terms_source "$prev_tag" "$work/prev-terms.swift") \
    || { echo "REFUSED: cannot find the Terms source of $prev_tag" >&2; return 1; }
  echo "identity gate: previous Terms source $src" | tee -a dist/release-gates.log
  echo "identity gate: release $tag vs $prev_tag" >> dist/release-gates.log
  check --app "$work/new/EastSea.app" --prev-app "$work/prev/EastSea.app" \
    --terms "$TERMS_FILE" --prev-terms "$work/prev-terms.swift" --log dist/release-gates.log
}

case "${1:-}" in
  check) shift; check "$@" ;;
  release) shift; release "$@" ;;
  *) echo "usage: $0 check --app A --prev-app P --terms T --prev-terms PT [--log F] | release DMG TAG REPO" >&2; exit 2 ;;
esac
