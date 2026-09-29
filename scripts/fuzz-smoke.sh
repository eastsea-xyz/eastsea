#!/usr/bin/env bash
# Run every untrusted-input parser under libFuzzer for one minute in CI.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
export PATH="$HOME/.cargo/bin:$PATH"
export TMPDIR="$root/tmp"
mkdir -p "$TMPDIR"
export CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-"$root/tmp/cargo-target"}

if ! command -v cargo-fuzz >/dev/null; then
    echo "cargo-fuzz is required (cargo install cargo-fuzz)" >&2
    exit 1
fi

export AETHER_PROVER_PROGRAM=$("$root/scripts/prover-program.sh")
cd "$root/fuzz"
for target in tx_envelope block_payload beacon_answer era_file snapshot_chunk; do
    mkdir -p "$root/tmp/fuzz-corpus/$target" "$root/tmp/fuzz-artifacts/$target"
    cp -n "$root/fuzz/corpus/$target/"* "$root/tmp/fuzz-corpus/$target/"
    cargo +nightly-2026-01-15 fuzz run "$target" "$root/tmp/fuzz-corpus/$target" -- \
        -max_total_time=60 -artifact_prefix="$root/tmp/fuzz-artifacts/$target/"
done
