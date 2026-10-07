#!/usr/bin/env bash
# The Mac wallet shows one language per screen: Korean on a Korean-preferring
# Mac, English otherwise (founder review of 0.7.0). This fails when a
# user-visible string has no Korean translation, or when an English sentence
# in the sources never reaches the String Catalog at all.
#   scripts/check-wallet-l10n.sh --stringsdata DIR   check the compiler's extraction (the Xcode build phase)
#   scripts/check-wallet-l10n.sh --sync DIR          add new keys to Localizable.xcstrings, drop unused ones
#   scripts/check-wallet-l10n.sh --lint              only the source scan
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
py=/usr/bin/python3
case "${1:-}" in
  --stringsdata) "$py" "$here/wallet-l10n.py" lint && "$py" "$here/wallet-l10n.py" check --stringsdata "$2" ;;
  --sync) "$py" "$here/wallet-l10n.py" sync --stringsdata "$2" ;;
  --lint) "$py" "$here/wallet-l10n.py" lint ;;
  --missing) "$py" "$here/wallet-l10n.py" missing ;;
  *) echo "usage: $0 --stringsdata DIR | --sync DIR | --lint | --missing" >&2; exit 2 ;;
esac
