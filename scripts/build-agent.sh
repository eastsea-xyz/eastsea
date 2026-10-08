#!/usr/bin/env bash
# Build aether-agent (MCP server + JSON CLI for AI agents; Secure Enclave keys).
#   scripts/build-agent.sh            build to target/agent/aether-agent
#   scripts/build-agent.sh --install  also copy to ~/.local/bin and the network file to the agent home
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
mkdir -p "$PWD/tmp"
export TMPDIR="$PWD/tmp"
# The agent ships inside the app bundle: same bytes wherever the checkout lives.
. scripts/repro-env.sh
aether_repro_rustflags
aether_repro_link_flags
MACOSX_DEPLOYMENT_TARGET=15.0 cargo build -p aether-ffi --release --locked
cargo run -q --locked -p aether-ffi --features bindgen --bin uniffi-bindgen -- generate --library target/release/libaether_ffi.dylib --language swift --out-dir apps/wallet/Generated
mv -f apps/wallet/Generated/aether_ffiFFI.modulemap apps/wallet/Generated/module.modulemap
out=target/agent
mkdir -p "$out"
# Embed the skill file so `setup --apply` can install it anywhere.
python3 - "$out/Skill.swift" <<'PY'
import sys
md = open("agents/skills/aether-wallet/SKILL.md", encoding="utf-8").read()
open(sys.argv[1], "w", encoding="utf-8").write('enum Skill {\n    static let markdown = #"""\n' + md + '"""#\n}\n')
PY
# Embed the DEX deployments (apps/agent/Resources/dex/*.json), keyed by chain id.
python3 - "$out/DexDeployments.swift" <<'PY'
import glob, json, sys
rows = []
for path in sorted(glob.glob("apps/agent/Resources/dex/*.json")):
    text = open(path, encoding="utf-8").read()
    rows.append('        %d: #"""\n%s\n"""#,' % (int(json.loads(text)["chainId"]), text.rstrip()))
body = "\n".join(rows) if rows else "        :"
open(sys.argv[1], "w", encoding="utf-8").write("enum DexDeployments {\n    static let byChainId: [UInt64: String] = [\n" + body + "\n    ]\n}\n")
PY
swiftc -O -target arm64-apple-macos15.0 -module-name AetherAgent \
  -file-prefix-map "$PWD"=/aether-node -debug-prefix-map "$PWD"=/aether-node \
  -I apps/wallet/Generated -Xcc -fmodule-map-file=apps/wallet/Generated/module.modulemap \
  apps/agent/Sources/*.swift apps/wallet/Generated/aether_ffi.swift "$out/Skill.swift" "$out/DexDeployments.swift" \
  target/release/libaether_ffi.a \
  -framework SystemConfiguration -framework Security -framework CoreFoundation -framework OpenDirectory -framework IOKit \
  -o "$out/aether-agent"
# swiftc links a Mach-O like any other: same UUID rewrite as the node (this
# replaces the ad-hoc signature, so it is also the signing step).
aether_repro_fix_uuid "$out/aether-agent"
echo "built $out/aether-agent"
if [ "${1:-}" = "--install" ]; then
  mkdir -p "$HOME/.local/bin"
  cp -f "$out/aether-agent" "$HOME/.local/bin/aether-agent"
  home="$HOME/Library/Application Support/Aether/agent"
  mkdir -p "$home" && chmod 700 "$home"
  cp -f apps/wallet/Resources/network.json "$home/network.json"
  echo "installed ~/.local/bin/aether-agent"
fi
