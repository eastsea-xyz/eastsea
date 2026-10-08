#!/usr/bin/env bash
# Build the EastSea browser extension wallet (Chrome, Edge, Brave, Arc: Manifest V3).
#   scripts/build-extension.sh          build apps/extension/wasm (load apps/extension unpacked)
#   scripts/build-extension.sh --zip    also pack dist/eastsea-extension-<version>.zip for the stores
# The extension builds transactions with crates/wasm (the same Rust rules the app uses)
# and signs with a P-256 key that never leaves the browser.
set -euo pipefail
cd "$(dirname "$0")/.."
# Both shipped applications use the same pure sea:// parser.
cp -f apps/shared/sea-url.mjs apps/explorer/js/sea-url.mjs
cp -f apps/shared/sea-url.mjs apps/extension/src/lib/sea-url.mjs
export PATH="$HOME/.cargo/bin:$PATH"
rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
# The BLS verifier (blst) is C: it needs a clang with a wasm32 backend. Apple's
# clang has none. Use CC_wasm32_unknown_unknown if set, else Homebrew's llvm.
if [ -z "${CC_wasm32_unknown_unknown:-}" ]; then
  for llvm in /opt/homebrew/opt/llvm /opt/homebrew/opt/llvm@21 /opt/homebrew/opt/llvm@20 /opt/homebrew/opt/llvm@19; do
    if [ -x "$llvm/bin/clang" ] && "$llvm/bin/clang" --print-targets | grep -q wasm32; then
      export CC_wasm32_unknown_unknown="$llvm/bin/clang" AR_wasm32_unknown_unknown="$llvm/bin/llvm-ar"
      break
    fi
  done
fi
if [ -z "${CC_wasm32_unknown_unknown:-}" ]; then
  echo "error: no clang with a wasm32 target (Apple's clang has none). brew install llvm, or set CC_wasm32_unknown_unknown and AR_wasm32_unknown_unknown" >&2
  exit 1
fi
# Release artifact: same bytes wherever the checkout lives (gap G5). The wasm
# target dir goes under the checkout, so name it before repro-env.sh so the
# remap list covers it.
CARGO_TARGET_DIR="$PWD/target/wasm"
export CARGO_TARGET_DIR
. scripts/repro-env.sh
# Paths in panic messages name crates, not this machine (no home or checkout path).
RUSTFLAGS="--cfg getrandom_backend=\"wasm_js\" $AETHER_REMAP_FLAGS" \
  wasm-pack build crates/wasm --release --target web --out-dir ../../target/wasm-pkg --no-typescript --no-pack >/dev/null
mkdir -p apps/extension/wasm
cp -f target/wasm-pkg/aether_wasm.js target/wasm-pkg/aether_wasm_bg.wasm apps/extension/wasm/
# The explorer verifies account balances with the same wasm and the same
# pinned network (apps/explorer/js/verify.js); both are build products here.
mkdir -p apps/explorer/wasm
cp -f target/wasm-pkg/aether_wasm.js target/wasm-pkg/aether_wasm_bg.wasm apps/explorer/wasm/
cp -f apps/extension/network.json apps/explorer/network.json
echo "built apps/extension (load it unpacked from chrome://extensions)"
if [ "${1:-}" = "--zip" ]; then
  version=$(python3 -c 'import json; print(json.load(open("apps/extension/manifest.json"))["version"])')
  mkdir -p dist
  out="$PWD/dist/eastsea-extension-$version.zip"
  rm -f "$out"
  # Deterministic pack (scripts/deterministic-zip.py): sorted entries, fixed
  # timestamp (SOURCE_DATE_EPOCH, UTC), 0644 mode. Plain `zip` records file
  # mtimes and traversal order, so two builds of the same files got two hashes.
  python3 scripts/deterministic-zip.py "$out" apps/extension \
    manifest.json src ui icons wasm
  echo "packed $out"
fi
