#!/usr/bin/env bash
# Build the proving sidecar (apps/prover) and print its program id (the guest
# ELF's SHA-256). Node builds export it as AETHER_PROVER_PROGRAM so a node only
# verifies proofs of the program the protocol names.
#   export AETHER_PROVER_PROGRAM=$(scripts/prover-program.sh)
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
[ -d "${AETHER_JOLT:-/Volumes/workspace/aether-jolt}/jolt" ] || { echo "the Jolt fork (aether-jolt/jolt) is needed to build the prover" >&2; exit 1; }
(cd apps/prover && MACOSX_DEPLOYMENT_TARGET=14.0 cargo build -q --release) >&2
# The binary lands in CARGO_TARGET_DIR when one is set (shared build dirs).
"${CARGO_TARGET_DIR:-apps/prover/target}/release/aether-prover" info | python3 -c 'import json,sys; print(json.load(sys.stdin)["guest_elf_sha256"])'
