#!/usr/bin/env bash
# Every wallet screen uses the English-base catalog with complete en/ko/ja/
# zh-Hans/zh-Hant coverage. Compiler extraction and direct String(localized:)
# keys must both exist in the catalog.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
py=/usr/bin/python3
case "${1:-}" in
  --stringsdata) "$py" "$here/wallet-l10n.py" lint && "$py" "$here/wallet-l10n.py" check --stringsdata "$2" ;;
  --sync) "$py" "$here/wallet-l10n.py" sync --stringsdata "$2" ;;
  --lint) "$py" "$here/wallet-l10n.py" lint ;;
  --missing) "$py" "$here/wallet-l10n.py" missing ;;
  --catalog) "$py" "$here/wallet-l10n.py" check ;;
  --self-test) "$py" "$here/wallet-l10n.py" self-test ;;
  *) echo "usage: $0 --stringsdata DIR | --sync DIR | --lint | --missing | --catalog | --self-test" >&2; exit 2 ;;
esac
