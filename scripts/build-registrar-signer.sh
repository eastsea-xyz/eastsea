#!/usr/bin/env bash
# Build aether-registrar-signer: the registrar's P-256 attestation key in the
# Secure Enclave of the signing Mac (docs/ops/registrar.md).
#   scripts/build-registrar-signer.sh            build to target/registrar-signer/
#   scripts/build-registrar-signer.sh --install  also copy to ~/.local/bin
set -euo pipefail
cd "$(dirname "$0")/.."
out=target/registrar-signer
mkdir -p "$out"
swiftc -O -target arm64-apple-macos15.0 -module-name RegistrarSigner \
  apps/registrar-signer/Sources/*.swift \
  -framework CryptoKit -framework Security -framework CoreFoundation \
  -o "$out/aether-registrar-signer"
codesign -s - -f "$out/aether-registrar-signer" 2>/dev/null
echo "built $out/aether-registrar-signer"
if [ "${1:-}" = "--install" ]; then
  mkdir -p "$HOME/.local/bin"
  cp -f "$out/aether-registrar-signer" "$HOME/.local/bin/aether-registrar-signer"
  echo "installed ~/.local/bin/aether-registrar-signer"
fi
