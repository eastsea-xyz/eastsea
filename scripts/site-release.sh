#!/usr/bin/env bash
# Put the newest published app version on eastsea.xyz and deploy the site.
#   scripts/site-release.sh            # version from the latest GitHub release
#   scripts/site-release.sh 0.7.2      # explicit version
# The download buttons already point at releases/latest; this keeps the
# version shown in the copy in step. site/release.json records what the site
# says, so the next run replaces exactly that string.
set -euo pipefail
cd "$(dirname "$0")/.."
repo=eastsea-xyz/eastsea
new=${1:-$(gh release view --repo "$repo" --json tagName -q .tagName | sed 's/^app-v//')}
[[ "$new" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "not a version: $new" >&2; exit 1; }
gh release view "app-v$new" --repo "$repo" --json isDraft -q .isDraft | grep -qx false \
  || { echo "app-v$new is not a published release" >&2; exit 1; }
old=$(python3 -c 'import json;print(json.load(open("site/release.json"))["version"])')
if [ "$old" != "$new" ]; then
  for f in site/index.html site/privacy.html; do
    # Only the version token: "0.7.0" with dots escaped, bounded by non-digits.
    perl -pi -e "s/(?<![0-9.])\Q$old\E(?![0-9])/$new/g" "$f"
  done
  printf '{"version": "%s"}\n' "$new" > site/release.json
fi
grep -c "$new" site/index.html >/dev/null
CLOUDFLARE_ACCOUNT_ID=41629a6d8d7dd09287249a57f1f604c4 \
  wrangler pages deploy site --project-name eastsea-site --branch main --commit-dirty=true >/dev/null
echo "eastsea.xyz now says $new (was $old)"
