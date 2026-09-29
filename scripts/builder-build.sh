#!/usr/bin/env bash
# Independent builder Mac: rebuild the tagged source without a distribution
# certificate, compare normalized code hashes to the release Mac's manifest.
#   scripts/builder-build.sh <manifest.json> <shared-Aether.dmg> <rebuilt.json>
set -euo pipefail
cd "$(dirname "$0")/.."
manifest=${1:?manifest.json required}
dmg=${2:?distributed DMG required}
rebuilt=${3:?rebuilt output required}
mkdir -p tmp
export TMPDIR="$PWD/tmp"
export PATH="$HOME/.cargo/bin:$PATH"
export SOURCE_DATE_EPOCH=$(git -C "$PWD" show -s --format=%ct HEAD)
export RUSTFLAGS="${RUSTFLAGS:+$RUSTFLAGS }--remap-path-prefix=$PWD=/aether-src"
export OTHER_SWIFT_FLAGS="${OTHER_SWIFT_FLAGS:+$OTHER_SWIFT_FLAGS }-debug-prefix-map $PWD=/aether-src"
export CODE_SIGNING_ALLOWED=NO
[ -z "$(git -C "$PWD" status --porcelain)" ] || { echo "ALARM: source worktree is dirty" >&2; exit 1; }
scripts/build-wallet.sh macos
scripts/release-approve.py rebuild "$manifest" --dmg "$dmg" \
  --app apps/wallet/build/Build/Products/Release/Aether.app --out "$rebuilt"
echo "Rebuild matched. Sign with: scripts/builder-sign sign $manifest --local-manifest $rebuilt"
