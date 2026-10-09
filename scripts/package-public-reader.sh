#!/usr/bin/env bash
# Package the same verified reader for the extension and the static site.
# Run after scripts/build-extension.sh; this step performs no compilation.
set -euo pipefail
cd "$(dirname "$0")/.."
for f in apps/explorer/wasm/aether_wasm.js apps/explorer/wasm/aether_wasm_bg.wasm; do
  [ -f "$f" ] || { echo "missing $f: first run the gated extension build" >&2; exit 1; }
done
cp apps/explorer/js/peers.js apps/extension/src/lib/peers.js
cp apps/explorer/public-read-peers.json apps/extension/public-read-peers.json
mkdir -p site/explorer/js site/explorer/wasm
cp apps/explorer/index.html apps/explorer/explorer.css apps/explorer/network.json \
  apps/explorer/token-sources.json apps/explorer/public-read-peers.json site/explorer/
cp apps/explorer/js/*.js site/explorer/js/
cp apps/explorer/wasm/aether_wasm.js apps/explorer/wasm/aether_wasm_bg.wasm site/explorer/wasm/
echo "packaged the public peer reader for site/explorer and the extension"
