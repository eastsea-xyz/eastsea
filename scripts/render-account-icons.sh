#!/usr/bin/env bash
# Render only the account icon spec/view, never the wallet app or its keys.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$root"
mkdir -p "$root/tmp/account-icon-render"
export TMPDIR="$root/tmp/account-icon-render"
output="${1:-$root/docs/design/46-account-icon/swiftui}"
if [ "$#" -gt 1 ]; then
  echo "usage: scripts/render-account-icons.sh [OUTPUT_DIRECTORY]" >&2
  exit 2
fi
if [ "$(uname -s)" != Darwin ]; then
  echo "Static SwiftUI account icon rendering requires macOS." >&2
  exit 2
fi
compiler="$(xcrun --find swiftc)"
sdk="$(xcrun --show-sdk-path)"
binary="$TMPDIR/account-icon-snapshots"
scripts/compile-gate.sh "$compiler" -Onone -parse-as-library -sdk "$sdk" \
  -module-cache-path "$TMPDIR/module-cache" \
  apps/wallet/Sources/AccountIconSpec.swift \
  apps/wallet/Sources/AccountIconView.swift \
  apps/wallet/Screens/AccountIconSnapshots.swift -o "$binary"
"$binary" "$root/crates/client/tests/account-icon-vectors.json" "$output"
