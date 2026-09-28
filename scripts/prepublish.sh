#!/usr/bin/env bash
# Build a clean copy of this repository's history for publishing the source.
# Nothing here changes this repository: the rewrite happens in a fresh clone.
#   scripts/prepublish.sh <out-dir> [branch]
# - our hosts' real IPs (from the local .gitleaks.toml rule) -> documentation IPs,
#   in files and commit messages
# - old company commit e-mail -> the author's address
# - drops docs/.pdca-snapshots, .gitleaks.toml and any *.p8 from every commit
# - scans the result with gitleaks (default rules + ours) and fails on a finding
# Then review <out-dir> and push it yourself.
set -euo pipefail
cd "$(dirname "$0")/.."
OUT=${1:?usage: $0 <out-dir> [branch]}
BRANCH=${2:-$(git branch --show-current)}
CONFIG="$PWD/.gitleaks.toml"
[ -f "$CONFIG" ] || { echo "need the local .gitleaks.toml (it holds the IP rule)"; exit 1; }
command -v git-filter-repo >/dev/null || { echo "need git-filter-repo"; exit 1; }
command -v gitleaks >/dev/null || { echo "need gitleaks"; exit 1; }
[ -e "$OUT" ] && { echo "$OUT exists"; exit 1; }

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
# IPs listed in the rule's regex alternation, e.g. \b(1\.2\.3\.4|...)\b
python3 - "$CONFIG" "$WORK/replace.txt" <<'PY'
import re, sys
text = open(sys.argv[1]).read()
rule = re.search(r'id = "real-public-ip-of-our-hosts".*?regex = \'\'\'(.*?)\'\'\'', text, re.S)
ips = re.findall(r'\d{1,3}(?:\\?\.\d{1,3}){3}', rule.group(1)) if rule else []
docs = ["198.51.100.10", "203.0.113.20", "192.0.2.30", "198.51.100.40", "203.0.113.50"]
with open(sys.argv[2], "w") as f:
    for i, ip in enumerate(ips):
        f.write(f"{ip.replace(chr(92), '')}==>{docs[i % len(docs)]}\n")
print(f"{len(ips)} host IPs to replace")
PY
printf 'Jay Lee <k.jaylee@gmail.com> <kjaylee@pipln.com>\n' > "$WORK/mailmap"

git clone -q --no-local --single-branch --branch "$BRANCH" . "$OUT"
cd "$OUT"
git filter-repo --force \
  --replace-text "$WORK/replace.txt" --replace-message "$WORK/replace.txt" \
  --mailmap "$WORK/mailmap" \
  --invert-paths --path docs/.pdca-snapshots --path .gitleaks.toml --path-glob '*.p8'
git remote remove origin 2>/dev/null || true
gitleaks git --no-banner --redact --config "$CONFIG" . && echo "clean: review $OUT, then push it yourself"
