#!/usr/bin/env bash
# Build the Aether browser extension wallet (Chrome, Edge, Brave, Arc: Manifest V3).
#   scripts/build-extension.sh          build apps/extension/wasm (load apps/extension unpacked)
#   scripts/build-extension.sh --zip    also pack dist/aether-extension-<version>.zip for the stores
# The extension builds transactions with crates/wasm (the same Rust rules the app uses)
# and signs with a P-256 key that never leaves the browser.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true
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
echo "built apps/extension (load it unpacked from chrome://extensions)"
if [ "${1:-}" = "--zip" ]; then
  version=$(python3 -c 'import json; print(json.load(open("apps/extension/manifest.json"))["version"])')
  mkdir -p dist
  out="$PWD/dist/aether-extension-$version.zip"
  rm -f "$out"
  # Deterministic pack (scripts/deterministic-zip.py): sorted entries, fixed
  # timestamp (SOURCE_DATE_EPOCH, UTC), 0644 mode. Plain `zip` records file
  # mtimes and traversal order, so two builds of the same files got two hashes.
  python3 scripts/deterministic-zip.py "$out" apps/extension \
    manifest.json src ui icons wasm
  echo "packed $out"
fi
