#!/usr/bin/env bash
# S1a reproduction: Jolt (a16z, feat/akita-metal) prover on this Mac, CPU vs Metal.
#   benches/spike/jolt.sh [dir]   (default /Volumes/workspace/spike/jolt-akita)
set -euo pipefail
export PATH="$HOME/.cargo/bin:$PATH"   # rustup toolchain (Homebrew rustc lacks the RISC-V targets)
D=${1:-/Volumes/workspace/spike/jolt-akita}
[ -d "$D" ] || git clone --depth 1 -b feat/akita-metal https://github.com/a16z/jolt.git "$D"
cd "$D"
git log --oneline -1
cargo install --path . --locked -q
cargo build --release -q -p jolt-prover --features profiling,metal
for s in 20 22 24; do
  for b in optimized metal; do
    echo "== sha2-chain 2^$s backend=$b"
    /usr/bin/time -l ./target/release/jolt-prover profile --name sha2-chain --scale $s --format none --backend $b 2>&1 \
      | grep -E "PROOF_VERIFIED|Prover completed|Proof size|Verifier|maximum resident|panicked|Invalid input" || true
  done
done
cargo build --release -q -p p256-ecdsa-verify
RUST_LOG=info ./target/release/p256-ecdsa-verify 2>&1 | grep -E "Prover runtime|valid"
