#!/usr/bin/env bash
# Build aether-agent (MCP server + JSON CLI for AI agents; Secure Enclave keys).
#   scripts/build-agent.sh            build to target/agent/aether-agent
#   scripts/build-agent.sh --install  also copy to ~/.local/bin and the network file to the agent home
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
MACOSX_DEPLOYMENT_TARGET=15.0 cargo build -p aether-ffi --release
cargo run -q -p aether-ffi --bin uniffi-bindgen -- generate --library target/release/libaether_ffi.dylib --language swift --out-dir apps/wallet/Generated
mv -f apps/wallet/Generated/aether_ffiFFI.modulemap apps/wallet/Generated/module.modulemap
out=target/agent
mkdir -p "$out"
# Embed the skill file so `setup --apply` can install it anywhere.
python3 - "$out/Skill.swift" <<'PY'
import sys
md = open("agents/skills/aether-wallet/SKILL.md", encoding="utf-8").read()
open(sys.argv[1], "w", encoding="utf-8").write('enum Skill {\n    static let markdown = #"""\n' + md + '"""#\n}\n')
PY
swiftc -O -target arm64-apple-macos15.0 -module-name AetherAgent \
  -I apps/wallet/Generated -Xcc -fmodule-map-file=apps/wallet/Generated/module.modulemap \
  apps/agent/Sources/*.swift apps/wallet/Generated/aether_ffi.swift "$out/Skill.swift" \
  target/release/libaether_ffi.a \
  -framework SystemConfiguration -framework Security -framework CoreFoundation -framework OpenDirectory -framework IOKit \
  -o "$out/aether-agent"
codesign -s - -f "$out/aether-agent" 2>/dev/null
echo "built $out/aether-agent"
if [ "${1:-}" = "--install" ]; then
  mkdir -p "$HOME/.local/bin"
  cp -f "$out/aether-agent" "$HOME/.local/bin/aether-agent"
  home="$HOME/Library/Application Support/Aether/agent"
  mkdir -p "$home" && chmod 700 "$home"
  cp -f apps/wallet/Resources/network.json "$home/network.json"
  echo "installed ~/.local/bin/aether-agent"
fi
