#!/usr/bin/env bash
# Build the prove_block guest ELF with the jolt CLI (riscv64imac-zero-linux-musl,
# std mode), exactly as jolt-sdk's generated `compile_prove_block` would, and copy
# it to OUT. The aether-prover host embeds that file (build.rs), so the binary
# never compiles the guest at runtime.
#
#   ./build-guest.sh [OUT]      default OUT: target-guest/prove_block.elf
#
# Needs: the `jolt` CLI (JOLT_PATH overrides) and the rustup toolchain on PATH.
# Memory parameters must match #[jolt::provable] on guest/src/lib.rs::prove_block.
set -euo pipefail
cd "$(dirname "$0")"
here="$(pwd)"
out="${1:-$here/target-guest/prove_block.elf}"
target_dir="${GUEST_TARGET_DIR:-$here/target-guest/aether-prover-guest-prove_block}"
jolt_cmd="${JOLT_PATH:-jolt}"

cargo_home="${CARGO_HOME:-$HOME/.cargo}"

# When run from build.rs, drop the outer cargo's settings (they target the host).
for v in $(env | cut -d= -f1 | grep -E '^(CARGO|CARGO_.*|RUSTC.*|RUSTFLAGS|RUSTDOC.*|RUSTUP_TOOLCHAIN|TARGET|HOST|OUT_DIR|PROFILE|OPT_LEVEL|DEBUG|NUM_JOBS)$'); do
  unset "$v"
done

# The ELF is the program the proofs are about: prover and verifier must embed
# the same bytes. Strip machine-specific paths (panic locations) so a build
# does not depend on where the checkouts, cargo home or toolchain live.
repo="$(cd ../.. && pwd)"
jolt_src="$(cd "${JOLT_SRC:-/Volumes/workspace/aether-jolt/jolt}" && pwd)"
sysroot="$(rustc --print sysroot)"
# One timestamp and zeroed archive dates, as in the host build
# (scripts/repro-env.sh): the embedded ELF's hash is the program id, so it must
# not follow the build time either.
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-$(git -C "$repo" log -1 --pretty=%ct 2>/dev/null || echo 1767225600)}"
export ZERO_AR_DATE=1
# (rustc applies the last matching prefix, so the nested target dir goes last.)
export ZEROOS_GUEST_RUSTFLAGS="--remap-path-prefix=$repo=/aether --remap-path-prefix=$jolt_src=/jolt --remap-path-prefix=$cargo_home=/cargo --remap-path-prefix=$sysroot=/rustc --remap-path-prefix=$target_dir=/target"

JOLT_FUNC_NAME=prove_block "$jolt_cmd" build -p aether-prover-guest \
  --mode std --backtrace off \
  --stack-size 4194304 --heap-size 268435456 \
  -- --release --target-dir "$target_dir" --features guest >&2

elf="$target_dir/riscv64imac-zero-linux-musl/release/aether-prover-guest"
test -f "$elf" || { echo "build-guest: ELF not found at $elf" >&2; exit 1; }
mkdir -p "$(dirname "$out")"
cp "$elf" "$out"
echo "build-guest: $(shasum -a 256 "$out" | cut -d' ' -f1)  $out ($(wc -c < "$out" | tr -d ' ') bytes)" >&2
