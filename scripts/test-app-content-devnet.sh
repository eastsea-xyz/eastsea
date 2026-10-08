#!/usr/bin/env bash
# Owned Chain/EVM + loopback iroh + the wallet's resolver and real WK scheme.
# This launches a test executable, never the EastSea app or a live node.
set -euo pipefail
cd "$(dirname "$0")/.."
root=$(pwd -P)
mkdir -p "$root/tmp/app-content"
export TMPDIR="$root/tmp"
gate="$root/scripts/compile-gate.sh"
"$gate" && swiftc -parse-as-library -o "$root/tmp/app-content/page-check" \
  apps/wallet/Sources/SeaURL.swift apps/wallet/Sources/SeaNameResolver.swift \
  apps/wallet/Sources/SeaRegistryReader.swift apps/wallet/Sources/NodeAppContentSource.swift \
  apps/wallet/Sources/AppContent.swift apps/wallet/Sources/AppBundleScheme.swift \
  apps/wallet/Sources/AppBrowserIdentity.swift scripts/fixtures/app-content-page.swift || exit "$?"
if [ "${1:-}" = --build-helper ]; then exit 0; fi
"$gate" && AETHER_APP_PAGE_HARNESS="$root/tmp/app-content/page-check" cargo test -j4 -p aether-node --test app_content_e2e -- --nocapture || exit "$?"
