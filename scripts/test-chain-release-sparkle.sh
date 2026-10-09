#!/usr/bin/env bash
# Native, task-owned Sparkle handoff fixture; never launches or replaces an app.
set -euo pipefail
cd "$(dirname "$0")/.."
root="$(pwd -P)"
mkdir -p "$root/tmp"
export TMPDIR="$root/tmp"
framework="${SPARKLE_FRAMEWORK_DIR:-$root/apps/wallet/build/SourcePackages/artifacts/sparkle/Sparkle/Sparkle.xcframework/macos-arm64_x86_64}"
if [ ! -d "$framework/Sparkle.framework" ]; then
  echo "Sparkle is required for the native release fixture; run scripts/build-wallet.sh to resolve its pinned package." >&2
  exit 1
fi
python3 scripts/swift-test-cache.py --output "$root/tmp/sw-chain-release-sparkle" -- \
  -Onone -F "$framework" -framework Sparkle -Xlinker -rpath -Xlinker "$framework" \
  -module-cache-path "$root/tmp/swift-module-cache" \
  apps/wallet/Sources/ReleaseApproval.swift apps/wallet/Sources/UpdateChannel.swift \
  apps/wallet/Sources/ChainRelease.swift apps/wallet/Sources/ReleaseArtifact.swift \
  apps/wallet/Sources/ReleaseUpdateGate.swift apps/wallet/Sources/AppLanguage.swift \
  apps/wallet/Tests/LocalizationTestSupport.swift apps/wallet/Tests/chain-release-sparkle/main.swift
"$root/tmp/sw-chain-release-sparkle"
